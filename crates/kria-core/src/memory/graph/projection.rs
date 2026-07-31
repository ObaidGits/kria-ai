//! Canonical projection DTOs for the policy-safe mixed graph contract.
//!
//! **Task 2.3.1** — defines the stable, serde-serializable graph projection
//! types required by MGR-002 (Canonical Mixed Graph Projection):
//!
//! * [`ProjectedNodeKind`] / [`EdgeAuthorityClass`] — closed-world kind and
//!   authority enums (AC 2.1 / 2.2).
//! * [`EffectivePolicySummary`] — policy-safe namespace/scope/sensitivity
//!   snapshot for AC 2.3.
//! * [`ProvenanceSummary`] — policy-safe actor/method/time provenance for
//!   AC 2.3.
//! * [`ProjectedValidTime`] — Valid Time window with timezone offset for
//!   AC 2.3.
//! * [`TruthStateSummary`] — wraps [`TruthState`] with a re-verification
//!   class for AC 2.3.
//! * [`EndpointSummary`] — policy-safe canonical entity info on relationship
//!   endpoints (AC 2.4).
//! * [`AuthorizedAction`] — placeholder action enum resolved at query time
//!   (AC 2.3).
//! * [`ProjectedNode`] — entity-primary typed node (AC 2.1, 2.3, 2.5).
//! * [`ProjectedEdge`] — typed policy-safe edge with endpoint summaries
//!   (AC 2.2, 2.4, 2.6).
//! * [`FrontierAggregate`] — typed aggregate frontier for unavailable
//!   endpoints without exposing hidden identifiers (AC 2.6).
//! * [`GraphProjectionResponse`] — top-level versioned bounded response
//!   (MGR-007 AC 7.2).
//!
//! # Design Invariants
//! * A4: No claim is invented; missing data is `Unavailable` or omitted.
//! * A5: Effective_Policy precedes projection, ranking, and serialization.
//! * A6: Traversal is cycle-safe and ≤3 hops; frontiers are bounded.
//! * A7: Every response carries one [`GraphRevision`].
//! * Every type is `Debug + Clone + Serialize + Deserialize`.
//! * Raw UUIDs are NEVER used as human-facing labels.

use serde::{Deserialize, Serialize};

use crate::memory::model::{EntityId, GraphRevision, RecordId, TruthState, UtcTimestamp};
use crate::memory::types::StalenessClass;

// ── 1. ProjectedNodeKind ──────────────────────────────────────────────────

/// The closed set of node kinds in the typed graph projection (MGR-002 AC 1).
///
/// `Navigation` is not a node kind; navigation containers are expressed
/// through [`EdgeAuthorityClass::Navigation`] and tagged on generated
/// grouping edges rather than nodes.  All values are lowercase on the wire so
/// they match the `record_kind` / `entity_type` columns verbatim and remain
/// stable across schema versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectedNodeKind {
    /// A canonical named entity (`entities_v2`).
    Entity,
    /// A cognitive memory record (`records` — kind memory/summary/skill/rule).
    Memory,
    /// An evidence record attached to a relationship.
    Evidence,
    /// An external or internal source record.
    Source,
    /// A generated aggregate placeholder for a set of authorized items that
    /// cannot be individually disclosed (e.g. a frontier bucket).
    Aggregate,
}

impl ProjectedNodeKind {
    /// The canonical wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectedNodeKind::Entity => "entity",
            ProjectedNodeKind::Memory => "memory",
            ProjectedNodeKind::Evidence => "evidence",
            ProjectedNodeKind::Source => "source",
            ProjectedNodeKind::Aggregate => "aggregate",
        }
    }

    /// All known variants (useful for enumeration / UI).
    pub fn all() -> &'static [ProjectedNodeKind] {
        &[
            ProjectedNodeKind::Entity,
            ProjectedNodeKind::Memory,
            ProjectedNodeKind::Evidence,
            ProjectedNodeKind::Source,
            ProjectedNodeKind::Aggregate,
        ]
    }
}

// ── 2. EdgeAuthorityClass ─────────────────────────────────────────────────

/// The authority class of a projected edge (MGR-002 AC 2; mirrors the
/// `authority_class CHECK(stored/derived/inferred)` column on `relationships`
/// and adds the `navigation` class for generated grouping containers).
///
/// Design §4.2: navigation containers (generated facets) are labeled as such
/// here and in [`ProjectedNodeKind::Aggregate`]; they are never authority
/// topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeAuthorityClass {
    /// Directly recorded by a governed write (the default for explicit
    /// relationships stored in the authority store).
    Stored,
    /// Computed deterministically from authority records (e.g. entity
    /// co-mention, consolidation lineage).
    Derived,
    /// Produced by an inference step that is not purely deterministic (e.g.
    /// model-suggested relationship).
    Inferred,
    /// Generated navigation grouping — NOT authority topology; the group label
    /// must be surfaced to the caller (MGR-001 AC 4, design §4 A4).
    Navigation,
}

impl EdgeAuthorityClass {
    /// The canonical wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeAuthorityClass::Stored => "stored",
            EdgeAuthorityClass::Derived => "derived",
            EdgeAuthorityClass::Inferred => "inferred",
            EdgeAuthorityClass::Navigation => "navigation",
        }
    }

    /// All known variants.
    pub fn all() -> &'static [EdgeAuthorityClass] {
        &[
            EdgeAuthorityClass::Stored,
            EdgeAuthorityClass::Derived,
            EdgeAuthorityClass::Inferred,
            EdgeAuthorityClass::Navigation,
        ]
    }
}

// ── 3. EffectivePolicySummary ─────────────────────────────────────────────

/// Policy-safe projection of the Effective_Policy for a graph item (MGR-002
/// AC 3).
///
/// This is a **read-side summary** only — it carries no secret capability bits
/// or owner identifiers beyond what the caller's policy already permits.  The
/// `policy_version` field is the provenance hash of the contributing policies
/// (see [`crate::memory::policy::effective_policy::EffectivePolicy`]).
///
/// Design §4.1 policy columns: `namespace`, `scope`, `sensitivity 0..3`,
/// `policy_version` (provenance hash).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectivePolicySummary {
    /// Policy namespace (e.g. `"user"`, `"workspace"`).
    pub namespace: String,
    /// Policy scope (e.g. `"chat"`, `"global"`).
    pub scope: String,
    /// Effective sensitivity level `0..=3` (max of all contributors).
    pub sensitivity: u8,
    /// Provenance hash of the contributing policy set (BLAKE3 hex, from
    /// [`crate::memory::policy::effective_policy::EffectivePolicy::provenance_hash`]).
    pub policy_version: String,
}

// ── 4. ProvenanceSummary ──────────────────────────────────────────────────

/// Policy-safe provenance summary for a projected node or edge (MGR-002 AC 3).
///
/// Fields are the minimal attribution needed for epistemic transparency (A4)
/// without exposing hidden actor identities or session-level detail that
/// exceeds the caller's granted scope.
///
/// The full [`crate::memory::model::Provenance`] is available to authorized
/// callers through the inspector workflow; this struct is the bounded
/// projection-layer subset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceSummary {
    /// The kind of the originating source (e.g. `"native"`, `"mcp"`,
    /// `"import"`).  Omitted when unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<String>,
    /// Opaque actor identifier — NOT a display name or raw UUID label.
    /// Omitted when unavailable or when the caller's policy does not permit
    /// actor disclosure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    /// Method name (e.g. `"extraction"`, `"consolidation"`, `"manual"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Method version string (e.g. `"v1"`, `"2024-01"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method_version: Option<String>,
    /// Transaction-time creation instant in UTC (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<UtcTimestamp>,
}

// ── 5. ProjectedValidTime ─────────────────────────────────────────────────

/// The Valid Time window for a projected item (MGR-002 AC 3; MGR-010 AC 3).
///
/// Both bounds are optional (open interval = all time).  `timezone_offset_min`
/// preserves the originating source timezone for display while the authority
/// stores everything as UTC (design §4.1 `tz_offset_min`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedValidTime {
    /// Inclusive lower bound in UTC (RFC 3339).  `None` = open / unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<UtcTimestamp>,
    /// Exclusive upper bound in UTC (RFC 3339).  `None` = open / ongoing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<UtcTimestamp>,
    /// Offset in minutes from UTC for the originating local time (e.g. `330`
    /// for IST +05:30, `0` for UTC, `-300` for EST).  `None` when unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone_offset_min: Option<i16>,
}

// ── 6. TruthStateSummary ──────────────────────────────────────────────────

/// Truth disposition plus re-verification class for a projected item
/// (MGR-002 AC 3; design §4.2 `truth_state` + `staleness_class`).
///
/// Wraps the canonical [`TruthState`] (forward-compatible) and adds the
/// re-verification class that governs whether and how soon the item should be
/// re-checked — these are independent: a `Current` item can be `Slow`
/// (rarely needs re-verification) while an `Unverified` item might be
/// `VolatileVerifiable` (needs re-verification soon).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruthStateSummary {
    /// The canonical truth/lifecycle disposition.
    pub truth_state: TruthState,
    /// The re-verification class.  `None` when unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staleness_class: Option<StalenessClass>,
}

impl TruthStateSummary {
    /// Construct from a known truth state with no staleness information.
    pub fn bare(truth_state: TruthState) -> Self {
        TruthStateSummary {
            truth_state,
            staleness_class: None,
        }
    }

    /// Construct with both fields.
    pub fn with_staleness(truth_state: TruthState, staleness_class: StalenessClass) -> Self {
        TruthStateSummary {
            truth_state,
            staleness_class: Some(staleness_class),
        }
    }
}

// ── 7. EndpointSummary ────────────────────────────────────────────────────

/// Policy-safe canonical entity summary for a relationship endpoint
/// (MGR-002 AC 4).
///
/// This is the minimal entity information that MUST accompany every projected
/// edge endpoint so callers never need to issue a separate lookup just to
/// render a label.  Hidden entity identifiers are NEVER exposed here —
/// unavailable endpoints use [`FrontierAggregate`] instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSummary {
    /// Stable entity identifier.  Always present for visible endpoints.
    pub id: EntityId,
    /// Free-text entity type (e.g. `"person"`, `"project"`, `"tool"`).
    /// `None` when the entity type has not been assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    /// Human-facing display name.  MUST NOT be a raw UUID.
    /// `None` when unavailable (callers display `"Unnamed"` or similar).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Truth disposition of the entity at projection time.
    pub truth_state: TruthState,
}

// ── 8. AuthorizedAction ───────────────────────────────────────────────────

/// Placeholder for actions that the caller is authorized to perform on a
/// projected item (MGR-002 AC 3 / design §A5).
///
/// **This is a placeholder** — the actual set of actions is resolved at query
/// time by the capability engine (task 3.x) and intersected with the
/// caller's [`CallerContext`](crate::memory::model::CallerContext).  The
/// variants are exhaustive for the planned v2 API surface; unknown future
/// actions round-trip through `Other`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizedAction {
    /// Expand the node's neighbourhood or memory/evidence children.
    Expand,
    /// Open the full inspector view for this item.
    Inspect,
    /// Submit a correction claim against this item.
    Correct,
    /// Create a new relationship from this item.
    Relate,
    /// Initiate a governed forget operation.
    Forget,
    /// Restore a previously forgotten item.
    Restore,
    /// Initiate a governed hard-delete operation.
    Delete,
    /// Request a shortest-path traversal to another item.
    PathTo,
    /// View the retrieval-use trace for this item.
    TraceUse,
    /// Forward-compat: an action not yet known to this binary.
    #[serde(other)]
    Other,
}

// ── 9. NodeMetadata ───────────────────────────────────────────────────────

/// Typed per-kind metadata carried on a [`ProjectedNode`].
///
/// Each variant contains only the fields that make sense for the node kind so
/// callers never have to deal with empty optional bags.  `Entity` nodes carry
/// their type; `Memory` nodes carry their record kind and token estimate;
/// `Evidence` nodes carry polarity; `Source` nodes carry their kind; and
/// `Aggregate` nodes refer to a frontier summary.
///
/// Note: derives `PartialEq` but NOT `Eq` because the `Evidence` variant
/// contains a calibration-free `f32` score which does not implement `Eq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeMetadata {
    /// Metadata for an `entity` node.
    Entity {
        /// Free-text entity type (e.g. `"person"`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entity_type: Option<String>,
        /// Whether this entity is the canonical form of a resolved cluster.
        is_canonical: bool,
    },
    /// Metadata for a `memory` node.
    Memory {
        /// The record kind (e.g. `"memory"`, `"summary"`, `"skill"`, `"rule"`).
        record_kind: String,
        /// Estimated token cost.  `None` when unavailable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        estimated_tokens: Option<u32>,
    },
    /// Metadata for an `evidence` node.
    Evidence {
        /// Evidence polarity (`"supports"` or `"contradicts"`).
        polarity: String,
        /// Calibration-less relative score.  `None` when unavailable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        score: Option<f32>,
    },
    /// Metadata for a `source` node.
    Source {
        /// Source kind (e.g. `"native"`, `"mcp"`, `"import"`).
        source_kind: String,
        /// External identity label (e.g. tool name).  MUST NOT expose hidden IDs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        external_label: Option<String>,
    },
    /// Metadata for an `aggregate` frontier placeholder.
    Aggregate {
        /// The node kind of the aggregated items.
        aggregate_kind: ProjectedNodeKind,
        /// Caller-authorized count.  `None` when the count itself is unavailable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        authorized_count: Option<u32>,
        /// Why the full set is not shown.
        truncation_reason: TruncationReason,
    },
}

// ── 10. DirectionClass ───────────────────────────────────────────────────

/// Direction semantics of a projected edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectionClass {
    /// The edge has a meaningful source→target direction.
    Directed,
    /// The edge is undirected (symmetric relation).
    Symmetric,
}

// ── 11. ProjectedNode ────────────────────────────────────────────────────

/// A fully typed, policy-safe graph node (MGR-002 AC 1, 3, 5).
///
/// Every projected node carries the complete set of required fields defined
/// in MGR-002 AC 3 plus typed per-kind metadata.  `authorized_actions` is
/// populated at query time by the capability engine; it may be empty when
/// the caller has no actions available for this item.
///
/// Design A4: no field is invented — missing data is `Unavailable` or `None`.
/// Design A5: `effective_policy` is always populated before serialization.
/// Design A7: `graph_revision` ties this item to a single authority snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectedNode {
    /// Stable item identity.  For entity nodes: [`EntityId`]; for record nodes:
    /// the [`RecordId`] serialized as a string.  NEVER used as a display label.
    pub id: ProjectedItemId,
    /// The node kind.
    pub node_kind: ProjectedNodeKind,
    /// Authority class of the node's primary relationship to the graph.
    pub authority_class: EdgeAuthorityClass,
    /// The graph revision at which this item was projected (one per response).
    pub graph_revision: GraphRevision,
    /// Effective policy summary for this node (always present).
    pub effective_policy: EffectivePolicySummary,
    /// Truth disposition and re-verification class.
    pub truth_state: TruthStateSummary,
    /// Valid Time window.  `None` for nodes without explicit temporal bounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_time: Option<ProjectedValidTime>,
    /// Policy-safe provenance summary.
    pub provenance: ProvenanceSummary,
    /// Human-facing display name.  MUST NOT be a raw UUID.  `None` when
    /// unavailable — callers render a kind-appropriate placeholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Typed per-kind metadata.
    pub metadata: NodeMetadata,
    /// Actions the caller is authorized to invoke on this node.  Populated
    /// by the capability resolver at query time; empty = read-only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authorized_actions: Vec<AuthorizedAction>,
}

// ── 12. ProjectedEdge ───────────────────────────────────────────────────

/// A fully typed, policy-safe graph edge (MGR-002 AC 2, 4, 6).
///
/// `source_endpoint` and `target_endpoint` are always present when both
/// entity endpoints are visible to the caller.  When an endpoint is
/// unavailable (policy-hidden), the corresponding edge is OMITTED entirely,
/// or the caller receives a [`FrontierAggregate`] node representing the
/// hidden side — never a raw hidden identifier.
///
/// The `id` field is the stable relationship identity from
/// `relationships.identity_hash` (canonical BLAKE3 hex, not a UUID label).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectedEdge {
    /// Stable relationship identity hash (canonical BLAKE3 hex from
    /// `relationships.identity_hash`).  NEVER a raw UUID human label.
    pub id: String,
    /// Relation name from the registry (e.g. `"derived_from"`, `"mentions_entity"`).
    pub link_type: String,
    /// Relation version from the registry.
    pub link_type_version: u32,
    /// Authority class of this edge.
    pub authority_class: EdgeAuthorityClass,
    /// Direction semantics.
    pub direction: DirectionClass,
    /// Policy-safe canonical entity summary for the source endpoint.
    pub source_endpoint: EndpointSummary,
    /// Policy-safe canonical entity summary for the target endpoint.
    pub target_endpoint: EndpointSummary,
    /// Truth disposition and re-verification class of this edge.
    pub truth_state: TruthStateSummary,
    /// Valid Time window.  `None` for edges without explicit temporal bounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_time: Option<ProjectedValidTime>,
    /// Policy-safe provenance summary for this edge.
    pub provenance: ProvenanceSummary,
    /// The graph revision at which this edge was projected.
    pub graph_revision: GraphRevision,
    /// Effective policy summary for this edge.
    pub effective_policy: EffectivePolicySummary,
    /// Count of evidence records attached to this edge.  `0` when none.
    pub evidence_count: u32,
    /// Actions the caller is authorized to invoke on this edge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authorized_actions: Vec<AuthorizedAction>,
}

// ── 13. FrontierAggregate ───────────────────────────────────────────────

/// A typed aggregate frontier for an unavailable endpoint (MGR-002 AC 6).
///
/// Used when a projected edge references an endpoint that the caller cannot
/// see (policy-hidden, forgotten, or deleted).  The edge is either OMITTED or
/// the hidden side is represented as a `FrontierAggregate` WITHOUT exposing
/// any hidden identifier, count, or topology from unauthorized nodes.
///
/// Design §6.5: "frontier metadata reveals only authorized aggregate tokens."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontierAggregate {
    /// The node kind of the aggregated (hidden) items.
    pub aggregate_kind: ProjectedNodeKind,
    /// Count of items in the frontier that the caller IS authorized to see.
    /// `None` when even the count is not disclosable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorized_count: Option<u32>,
    /// Why the full set is hidden.
    pub truncation_reason: TruncationReason,
}

// ── 14. TruncationReason ────────────────────────────────────────────────

/// Why a node set or traversal was truncated (MGR-007 AC 6).
///
/// Returned on frontier aggregates and in the top-level
/// [`GraphProjectionResponse`] when the result is truncated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruncationReason {
    /// The traversal depth limit (≤3 hops) was reached.
    DepthLimit,
    /// The configured item-count cap was reached.
    ItemLimit,
    /// One or more nodes were hidden by policy.
    PolicyFiltered,
    /// The endpoint is deleted and no longer accessible.
    Deleted,
    /// The endpoint is forgotten and excluded from default reads.
    Forgotten,
    /// The caller's token/payload budget was exhausted.
    BudgetExhausted,
    /// The cursor or revision is expired/incompatible.
    CursorExpired,
    /// Forward-compat: an unrecognized truncation reason.
    #[serde(other)]
    Other,
}

// ── 15. TotalSemantics ──────────────────────────────────────────────────

/// Semantics of the total-count field in a [`GraphProjectionResponse`]
/// (MGR-006 AC 3; MGR-007 AC 2).
///
/// Callers render `"showing N of M"`, `"at least M"`, or `"estimate M"` based
/// on this discriminant.  The embedded count is the total authorized items
/// (not the page size).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "semantics", rename_all = "snake_case")]
pub enum TotalSemantics {
    /// The count is exact and fully enumerated.
    Exact { count: u64 },
    /// At least this many items exist (traversal was capped).
    AtLeast { count: u64 },
    /// An estimate; exact enumeration was not performed.
    Estimate { count: u64 },
}

// ── 16. ProjectedItemId ─────────────────────────────────────────────────

/// A stable item identity that can represent either an [`EntityId`] or a
/// [`RecordId`] in the projection layer.
///
/// The wire format is always a canonical lowercase UUID string.
/// NEVER used as a human-facing display label.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum ProjectedItemId {
    /// An entity identity.
    Entity(EntityId),
    /// A cognitive record identity.
    Record(RecordId),
}

impl ProjectedItemId {
    /// The wire string of the underlying UUID.
    pub fn as_uuid_str(&self) -> &str {
        match self {
            ProjectedItemId::Entity(id) => id.as_str(),
            ProjectedItemId::Record(id) => id.as_str(),
        }
    }
}

// ── 17. GraphProjectionResponse ─────────────────────────────────────────

/// Top-level versioned bounded graph projection response (MGR-002; MGR-007
/// AC 2).
///
/// One response = one [`GraphRevision`] snapshot.  The caller must apply or
/// discard the response atomically; mixing two revisions is an error.
///
/// Design A6: responses are bounded — `truncated` signals that the full result
/// was not returned and `frontier_token` can be used to continue traversal.
/// Design A7: every item in `nodes` and `edges` must carry the same
/// `graph_revision` as the response header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphProjectionResponse {
    /// Monotonically-increasing schema version for this response contract.
    pub schema_version: u32,
    /// The single authority revision that produced this response.
    pub graph_revision: GraphRevision,
    /// Canonical hash of the query that produced this response (for caching /
    /// deduplication).  BLAKE3 hex.
    pub query_hash: String,
    /// Policy scope summary for the caller at projection time.
    pub policy_scope_summary: EffectivePolicySummary,
    /// The projected nodes.
    pub nodes: Vec<ProjectedNode>,
    /// The projected edges.
    pub edges: Vec<ProjectedEdge>,
    /// Whether the result was truncated (depth, item, or budget limit reached).
    pub truncated: bool,
    /// The reason for truncation, when `truncated` is `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncation_reason: Option<TruncationReason>,
    /// An opaque token the caller may use to continue traversal from the
    /// frontier.  `None` when no continuation is available or the traversal
    /// is complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontier_token: Option<String>,
    /// The semantics and value of the total authorized item count.
    pub total_semantics: TotalSemantics,
    /// Cursor for paginating within this revision snapshot.  `None` when no
    /// further pages are available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::model::{EntityId, GraphRevision, RecordId, UtcTimestamp};
    use crate::memory::types::StalenessClass;

    fn sample_policy() -> EffectivePolicySummary {
        EffectivePolicySummary {
            namespace: "user".into(),
            scope: "chat".into(),
            sensitivity: 1,
            policy_version: "a".repeat(64),
        }
    }

    fn sample_provenance() -> ProvenanceSummary {
        ProvenanceSummary {
            source_kind: Some("native".into()),
            actor_id: Some("actor-001".into()),
            method: Some("extraction".into()),
            method_version: Some("v1".into()),
            created_at: Some(UtcTimestamp::now()),
        }
    }

    fn sample_truth() -> TruthStateSummary {
        TruthStateSummary::with_staleness(TruthState::Current, StalenessClass::Slow)
    }

    fn sample_valid_time() -> ProjectedValidTime {
        ProjectedValidTime {
            valid_from: Some(UtcTimestamp::now()),
            valid_until: None,
            timezone_offset_min: Some(330),
        }
    }

    // ── Enum round-trips ────────────────────────────────────────────────

    #[test]
    fn projected_node_kind_roundtrips() {
        for kind in ProjectedNodeKind::all() {
            let json = serde_json::to_string(kind).unwrap();
            let back: ProjectedNodeKind = serde_json::from_str(&json).unwrap();
            assert_eq!(*kind, back);
            // Wire value must be lowercase snake_case, not the UUID.
            assert!(
                !json.contains('-'),
                "kind json must not look like a UUID: {json}"
            );
        }
    }

    #[test]
    fn edge_authority_class_roundtrips() {
        for cls in EdgeAuthorityClass::all() {
            let json = serde_json::to_string(cls).unwrap();
            let back: EdgeAuthorityClass = serde_json::from_str(&json).unwrap();
            assert_eq!(*cls, back);
        }
    }

    #[test]
    fn authorized_action_roundtrips_known_and_unknown() {
        let known = [
            AuthorizedAction::Expand,
            AuthorizedAction::Inspect,
            AuthorizedAction::Correct,
            AuthorizedAction::Relate,
            AuthorizedAction::Forget,
            AuthorizedAction::Restore,
            AuthorizedAction::Delete,
            AuthorizedAction::PathTo,
            AuthorizedAction::TraceUse,
        ];
        for action in &known {
            let json = serde_json::to_string(action).unwrap();
            let back: AuthorizedAction = serde_json::from_str(&json).unwrap();
            assert_eq!(*action, back);
        }
        // Forward-compat: unknown value maps to Other.
        let other: AuthorizedAction = serde_json::from_str("\"future_action\"").unwrap();
        assert_eq!(other, AuthorizedAction::Other);
    }

    // ── Struct round-trips ───────────────────────────────────────────────

    #[test]
    fn effective_policy_summary_roundtrips() {
        let p = sample_policy();
        let json = serde_json::to_string(&p).unwrap();
        let back: EffectivePolicySummary = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn provenance_summary_roundtrips() {
        let p = sample_provenance();
        let json = serde_json::to_string(&p).unwrap();
        let back: ProvenanceSummary = serde_json::from_str(&json).unwrap();
        // created_at uses RFC 3339 so equality holds through serde.
        assert_eq!(p.source_kind, back.source_kind);
        assert_eq!(p.actor_id, back.actor_id);
        assert_eq!(p.method, back.method);
        assert_eq!(p.method_version, back.method_version);
    }

    #[test]
    fn provenance_summary_minimal_roundtrips() {
        let p = ProvenanceSummary {
            source_kind: None,
            actor_id: None,
            method: None,
            method_version: None,
            created_at: None,
        };
        let json = serde_json::to_string(&p).unwrap();
        // Omit-empty: no nulls in JSON.
        assert!(
            !json.contains("null"),
            "empty fields must be omitted: {json}"
        );
        let back: ProvenanceSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn projected_valid_time_roundtrips() {
        let vt = sample_valid_time();
        let json = serde_json::to_string(&vt).unwrap();
        let back: ProjectedValidTime = serde_json::from_str(&json).unwrap();
        assert_eq!(vt.timezone_offset_min, back.timezone_offset_min);
        assert_eq!(vt.valid_until, back.valid_until);
    }

    #[test]
    fn truth_state_summary_roundtrips() {
        let ts = sample_truth();
        let json = serde_json::to_string(&ts).unwrap();
        let back: TruthStateSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(ts.truth_state, back.truth_state);
        assert_eq!(ts.staleness_class, back.staleness_class);
    }

    #[test]
    fn truth_state_summary_bare_omits_staleness() {
        let ts = TruthStateSummary::bare(TruthState::Unverified);
        let json = serde_json::to_string(&ts).unwrap();
        assert!(
            !json.contains("staleness"),
            "bare summary must omit staleness: {json}"
        );
        let back: TruthStateSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(ts, back);
    }

    #[test]
    fn endpoint_summary_roundtrips() {
        let ep = EndpointSummary {
            id: EntityId::new_v7(),
            entity_type: Some("person".into()),
            display_name: Some("Ada Lovelace".into()),
            truth_state: TruthState::Current,
        };
        let json = serde_json::to_string(&ep).unwrap();
        // NEVER a raw UUID as the display label — the display_name field must differ.
        assert!(
            !ep.display_name.as_deref().unwrap_or("").contains('-'),
            "display_name must not look like a UUID"
        );
        let back: EndpointSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(ep, back);
    }

    #[test]
    fn frontier_aggregate_roundtrips() {
        let fa = FrontierAggregate {
            aggregate_kind: ProjectedNodeKind::Entity,
            authorized_count: Some(3),
            truncation_reason: TruncationReason::PolicyFiltered,
        };
        let json = serde_json::to_string(&fa).unwrap();
        let back: FrontierAggregate = serde_json::from_str(&json).unwrap();
        assert_eq!(fa, back);
    }

    // ── ProjectedNode round-trip ─────────────────────────────────────────

    #[test]
    fn projected_node_entity_roundtrips() {
        let node = ProjectedNode {
            id: ProjectedItemId::Entity(EntityId::new_v7()),
            node_kind: ProjectedNodeKind::Entity,
            authority_class: EdgeAuthorityClass::Stored,
            graph_revision: GraphRevision::new(42),
            effective_policy: sample_policy(),
            truth_state: sample_truth(),
            valid_time: Some(sample_valid_time()),
            provenance: sample_provenance(),
            display_name: Some("Ada Lovelace".into()),
            metadata: NodeMetadata::Entity {
                entity_type: Some("person".into()),
                is_canonical: true,
            },
            authorized_actions: vec![AuthorizedAction::Inspect, AuthorizedAction::Expand],
        };
        let json = serde_json::to_string(&node).unwrap();
        let back: ProjectedNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node.node_kind, back.node_kind);
        assert_eq!(node.authority_class, back.authority_class);
        assert_eq!(node.graph_revision, back.graph_revision);
        assert_eq!(node.display_name, back.display_name);
        assert_eq!(node.authorized_actions, back.authorized_actions);
    }

    #[test]
    fn projected_node_memory_roundtrips() {
        let node = ProjectedNode {
            id: ProjectedItemId::Record(RecordId::new_v7()),
            node_kind: ProjectedNodeKind::Memory,
            authority_class: EdgeAuthorityClass::Derived,
            graph_revision: GraphRevision::new(7),
            effective_policy: sample_policy(),
            truth_state: TruthStateSummary::bare(TruthState::Confirmed),
            valid_time: None,
            provenance: ProvenanceSummary {
                source_kind: None,
                actor_id: None,
                method: Some("consolidation".into()),
                method_version: Some("v2".into()),
                created_at: None,
            },
            display_name: None,
            metadata: NodeMetadata::Memory {
                record_kind: "summary".into(),
                estimated_tokens: Some(128),
            },
            authorized_actions: vec![],
        };
        let json = serde_json::to_string(&node).unwrap();
        // authorized_actions is empty → omitted from JSON.
        assert!(
            !json.contains("authorized_actions"),
            "empty authorized_actions must be omitted: {json}"
        );
        let back: ProjectedNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node.node_kind, back.node_kind);
        assert_eq!(node.authorized_actions, back.authorized_actions);
    }

    // ── ProjectedEdge round-trip ─────────────────────────────────────────

    #[test]
    fn projected_edge_roundtrips() {
        let edge = ProjectedEdge {
            id: "a".repeat(64), // synthetic BLAKE3 hex
            link_type: "derived_from".into(),
            link_type_version: 1,
            authority_class: EdgeAuthorityClass::Stored,
            direction: DirectionClass::Directed,
            source_endpoint: EndpointSummary {
                id: EntityId::new_v7(),
                entity_type: Some("project".into()),
                display_name: Some("KRIA".into()),
                truth_state: TruthState::Current,
            },
            target_endpoint: EndpointSummary {
                id: EntityId::new_v7(),
                entity_type: Some("memory".into()),
                display_name: None,
                truth_state: TruthState::Unverified,
            },
            truth_state: TruthStateSummary::bare(TruthState::Current),
            valid_time: None,
            provenance: sample_provenance(),
            graph_revision: GraphRevision::new(1),
            effective_policy: sample_policy(),
            evidence_count: 2,
            authorized_actions: vec![AuthorizedAction::Inspect],
        };
        let json = serde_json::to_string(&edge).unwrap();
        let back: ProjectedEdge = serde_json::from_str(&json).unwrap();
        assert_eq!(edge.id, back.id);
        assert_eq!(edge.link_type, back.link_type);
        assert_eq!(edge.authority_class, back.authority_class);
        assert_eq!(edge.direction, back.direction);
        assert_eq!(edge.evidence_count, back.evidence_count);
    }

    // ── GraphProjectionResponse round-trip ───────────────────────────────

    #[test]
    fn graph_projection_response_roundtrips() {
        let resp = GraphProjectionResponse {
            schema_version: 2,
            graph_revision: GraphRevision::new(99),
            query_hash: "b".repeat(64),
            policy_scope_summary: sample_policy(),
            nodes: vec![],
            edges: vec![],
            truncated: false,
            truncation_reason: None,
            frontier_token: None,
            total_semantics: TotalSemantics::Exact { count: 0 },
            cursor: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: GraphProjectionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp.schema_version, back.schema_version);
        assert_eq!(resp.graph_revision, back.graph_revision);
        assert_eq!(resp.truncated, back.truncated);
    }

    #[test]
    fn graph_projection_response_truncated_roundtrips() {
        let resp = GraphProjectionResponse {
            schema_version: 2,
            graph_revision: GraphRevision::new(5),
            query_hash: "c".repeat(64),
            policy_scope_summary: sample_policy(),
            nodes: vec![],
            edges: vec![],
            truncated: true,
            truncation_reason: Some(TruncationReason::DepthLimit),
            frontier_token: Some("opaque-cursor-token".into()),
            total_semantics: TotalSemantics::AtLeast { count: 120 },
            cursor: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: GraphProjectionResponse = serde_json::from_str(&json).unwrap();
        assert!(back.truncated);
        assert_eq!(back.truncation_reason, Some(TruncationReason::DepthLimit));
        assert_eq!(back.frontier_token.as_deref(), Some("opaque-cursor-token"));
    }

    // ── TotalSemantics round-trips ───────────────────────────────────────

    #[test]
    fn total_semantics_variants_roundtrip() {
        let variants = [
            TotalSemantics::Exact { count: 42 },
            TotalSemantics::AtLeast { count: 100 },
            TotalSemantics::Estimate { count: 999 },
        ];
        for v in &variants {
            let json = serde_json::to_string(v).unwrap();
            let back: TotalSemantics = serde_json::from_str(&json).unwrap();
            assert_eq!(*v, back);
        }
    }

    // ── ProjectedItemId ──────────────────────────────────────────────────

    #[test]
    fn projected_item_id_roundtrips() {
        let eid = ProjectedItemId::Entity(EntityId::new_v7());
        let rid = ProjectedItemId::Record(RecordId::new_v7());
        for id in [&eid, &rid] {
            let json = serde_json::to_string(id).unwrap();
            let back: ProjectedItemId = serde_json::from_str(&json).unwrap();
            assert_eq!(*id, back);
        }
    }

    #[test]
    fn truncation_reason_forward_compat() {
        // Unknown value round-trips as Other.
        let other: TruncationReason = serde_json::from_str("\"some_future_reason\"").unwrap();
        assert_eq!(other, TruncationReason::Other);
    }
}
