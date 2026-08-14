//! Frontier token, edge assembly, and label guard for the policy-safe graph.
//!
//! **Task 2.3.5** — implements three concrete contracts:
//!
//! * [`FrontierToken`] / [`FrontierTokenBuilder`] / [`DecodedFrontierToken`] —
//!   opaque resumption cursors that encode the traversal frontier, hop depth,
//!   graph revision, and policy version without exposing hidden IDs (MGR-002
//!   AC 6; design §A4/A6).
//! * [`EdgeAssembler`] — builds endpoint-complete [`ProjectedEdge`]s from
//!   [`TraversalEdge`]s + entity lookup; omits edges where either endpoint is
//!   missing from the authorized lookup (MGR-002 AC 4/6).
//! * [`LabelGuard`] — enforces at the API surface that no `display_name` on
//!   any [`ProjectedNode`] or [`EndpointSummary`] is a raw UUID string (design
//!   §A4; MGR-001 AC 4).
//!
//! # Design Invariants
//! * A4: No hidden ID, name, or topology is exposed. Missing data is
//!   `Unavailable` (i.e. `None`) or omitted.
//! * A5: Authorization precedes projection — edges where either endpoint is
//!   absent from the authorized lookup are silently dropped.
//! * A6: Frontier tokens are bounded and opaque; callers MUST NOT parse them.

use std::collections::HashMap;

use base64::{engine::general_purpose::URL_SAFE, Engine as _};
use serde::{Deserialize, Serialize};

use crate::graph::projection::{
    EffectivePolicySummary, EndpointSummary, ProjectedEdge, ProjectedNode, ProvenanceSummary,
    TruthStateSummary,
};
use crate::graph::traversal::{TraversalEdge, TraversalNode};
use crate::model::{EntityId, GraphRevision};

// ── 1. FrontierToken ─────────────────────────────────────────────────────

/// An opaque cursor token for resuming traversal from the current frontier.
///
/// Encodes: the set of entity IDs at the frontier (the nodes that would be
/// expanded next), the current hop depth, the graph revision, the policy
/// version, and an expiry epoch.
///
/// The serialized form is opaque to callers (base64-encoded JSON); callers
/// MUST NOT parse or construct it manually.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontierToken {
    /// Opaque base64-encoded content — callers treat this as an opaque string.
    pub token: String,
}

// ── 2. Internal token payload (not public) ───────────────────────────────

/// The in-memory representation of a frontier token's content.
/// Serialized to/from JSON, then base64-encoded.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FrontierPayload {
    /// Authorized entity IDs at the frontier.
    ids: Vec<String>,
    /// Hop depth reached.
    hop: u8,
    /// Graph revision this token was issued for.
    rev: u64,
    /// Policy version hash this token was issued for.
    pv: String,
}

// ── 3. DecodedFrontierToken ──────────────────────────────────────────────

/// The decoded, validated content of a [`FrontierToken`].
///
/// Returned by [`FrontierTokenBuilder::decode`] only when the token is valid
/// and matches the current revision and policy version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrontierToken {
    /// The authorized entity IDs at the frontier.
    pub frontier_ids: Vec<EntityId>,
    /// The hop depth the previous traversal reached.
    pub hop_depth: u8,
    /// The graph revision this token was issued for.
    pub graph_revision: GraphRevision,
}

// ── 4. FrontierTokenBuilder ──────────────────────────────────────────────

/// Builds and decodes opaque frontier resumption tokens (design §A6).
///
/// Tokens are non-cryptographic base64-encoded JSON blobs; they are NOT
/// signed or encrypted. Callers MUST treat the `token` string as opaque.
/// Revision and policy-version checks prevent stale tokens from being accepted.
pub struct FrontierTokenBuilder;

impl FrontierTokenBuilder {
    /// Build a frontier token.
    ///
    /// - `frontier_ids`: authorized entity IDs at the current traversal
    ///   frontier (NOT hidden IDs — only IDs cut off by a cap).
    /// - `hop_depth`: the hop depth reached.
    /// - `graph_revision`: the current authority revision.
    /// - `policy_version`: the caller's policy version hash.
    pub fn build(
        frontier_ids: &[EntityId],
        hop_depth: u8,
        graph_revision: GraphRevision,
        policy_version: &str,
    ) -> FrontierToken {
        let payload = FrontierPayload {
            ids: frontier_ids
                .iter()
                .map(|id| id.as_str().to_owned())
                .collect(),
            hop: hop_depth,
            rev: graph_revision.get(),
            pv: policy_version.to_owned(),
        };
        let json = serde_json::to_string(&payload).expect("FrontierPayload is always serializable");
        let token = URL_SAFE.encode(json.as_bytes());
        FrontierToken { token }
    }

    /// Decode and validate a frontier token.
    ///
    /// Returns `None` when:
    /// - The token is not valid base64 or the payload is not valid JSON.
    /// - `rev` in the token does not equal `graph_revision`.
    /// - `pv` in the token does not equal `policy_version`.
    /// - Any stored ID fails [`EntityId`] validation.
    pub fn decode(
        token: &FrontierToken,
        graph_revision: GraphRevision,
        policy_version: &str,
    ) -> Option<DecodedFrontierToken> {
        let bytes = URL_SAFE.decode(&token.token).ok()?;
        let json = std::str::from_utf8(&bytes).ok()?;
        let payload: FrontierPayload = serde_json::from_str(json).ok()?;

        // Reject stale revision or policy mismatch.
        if payload.rev != graph_revision.get() {
            return None;
        }
        if payload.pv != policy_version {
            return None;
        }

        // Validate all stored IDs.
        let mut frontier_ids = Vec::with_capacity(payload.ids.len());
        for raw_id in &payload.ids {
            let id = EntityId::new(raw_id).ok()?;
            frontier_ids.push(id);
        }

        Some(DecodedFrontierToken {
            frontier_ids,
            hop_depth: payload.hop,
            graph_revision,
        })
    }
}

// ── 5. EdgeAssembler ─────────────────────────────────────────────────────

/// Builds endpoint-complete [`ProjectedEdge`]s from [`TraversalEdge`]s and an
/// authorized entity lookup map (MGR-002 AC 4/6).
///
/// Edges where either endpoint is absent from the lookup are silently omitted
/// (design §A4: no hidden ID is exposed; design §A5: authorization precedes
/// projection).
pub struct EdgeAssembler;

impl EdgeAssembler {
    /// Build a single endpoint-complete [`ProjectedEdge`] from a
    /// [`TraversalEdge`] and an entity lookup map.
    ///
    /// Rules:
    /// - If either endpoint is missing from `entity_lookup`, the edge is
    ///   omitted (`None` is returned).
    /// - `display_name` on each [`EndpointSummary`] is passed through
    ///   [`LabelGuard::sanitize_display_name`] before use.
    /// - [`EffectivePolicySummary`] is built from the edge's own
    ///   namespace/scope/sensitivity/policy_version.
    /// - `graph_revision` is preserved on the returned edge.
    pub fn assemble_edge(
        edge: &TraversalEdge,
        entity_lookup: &HashMap<String, TraversalNode>,
        graph_revision: GraphRevision,
    ) -> Option<ProjectedEdge> {
        let source_node = entity_lookup.get(edge.source_id.as_str())?;
        let target_node = entity_lookup.get(edge.target_id.as_str())?;

        let source_endpoint = node_to_endpoint_summary(source_node);
        let target_endpoint = node_to_endpoint_summary(target_node);

        let truth_state = TruthStateSummary::bare(edge.truth_state.clone());

        let valid_time = if edge.valid_from.is_some() || edge.valid_until.is_some() {
            Some(crate::graph::projection::ProjectedValidTime {
                valid_from: edge.valid_from.clone(),
                valid_until: edge.valid_until.clone(),
                timezone_offset_min: None,
            })
        } else {
            None
        };

        let effective_policy = EffectivePolicySummary {
            namespace: edge.namespace.clone(),
            scope: edge.scope.clone(),
            sensitivity: edge.sensitivity,
            policy_version: edge.policy_version.clone(),
        };

        let provenance = ProvenanceSummary {
            source_kind: edge.source_kind.clone(),
            actor_id: edge.actor_id.clone(),
            method: None,
            method_version: None,
            created_at: edge.created_at.clone(),
        };

        Some(ProjectedEdge {
            id: edge.identity_hash.clone(),
            link_type: edge.link_type.clone(),
            link_type_version: edge.link_type_version,
            authority_class: edge.authority_class,
            direction: edge.direction,
            source_endpoint,
            target_endpoint,
            truth_state,
            valid_time,
            provenance,
            graph_revision,
            effective_policy,
            evidence_count: edge.evidence_count,
            authorized_actions: vec![],
        })
    }

    /// Build all endpoint-complete projected edges from a slice of traversal
    /// edges. Edges where either endpoint is absent from `entity_lookup` are
    /// silently omitted.
    pub fn assemble_all(
        edges: &[TraversalEdge],
        entity_lookup: &HashMap<String, TraversalNode>,
        graph_revision: GraphRevision,
    ) -> Vec<ProjectedEdge> {
        edges
            .iter()
            .filter_map(|e| Self::assemble_edge(e, entity_lookup, graph_revision))
            .collect()
    }
}

/// Convert a [`TraversalNode`] to an [`EndpointSummary`], applying the
/// [`LabelGuard`] to the display name.
fn node_to_endpoint_summary(node: &TraversalNode) -> EndpointSummary {
    EndpointSummary {
        id: node.id.clone(),
        entity_type: node.entity_type.clone(),
        display_name: LabelGuard::sanitize_display_name(node.display_name.clone()),
        truth_state: node.truth_state.clone(),
    }
}

// ── 6. LabelGuard ────────────────────────────────────────────────────────

/// Validates that display names are never raw UUID strings (design §A4;
/// MGR-001 AC 4).
///
/// A raw UUID matches exactly:
/// `^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`
///
/// UUIDs are stable identifiers for internal use; they MUST NOT appear as
/// human-facing labels in any projected item.
pub struct LabelGuard;

impl LabelGuard {
    /// Returns `true` when `label` looks like a raw UUID (lowercase canonical
    /// form only — mixed-case is NOT a UUID in our canonical form).
    pub fn is_uuid_label(label: &str) -> bool {
        // Exact 36-character match: 8-4-4-4-12 lowercase hex digits with dashes.
        let b = label.as_bytes();
        if b.len() != 36 {
            return false;
        }
        // Dash positions: 8, 13, 18, 23
        if b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
            return false;
        }
        // Validate each hex segment (lowercase only).
        is_lower_hex_segment(b, 0, 8)
            && is_lower_hex_segment(b, 9, 13)
            && is_lower_hex_segment(b, 14, 18)
            && is_lower_hex_segment(b, 19, 23)
            && is_lower_hex_segment(b, 24, 36)
    }

    /// Returns `None` when `name` is a raw UUID (callers render a
    /// kind-appropriate placeholder). Returns `Some(name)` unchanged when the
    /// name is a valid human label.
    pub fn sanitize_display_name(name: Option<String>) -> Option<String> {
        match name {
            None => None,
            Some(n) if Self::is_uuid_label(&n) => None,
            Some(n) => Some(n),
        }
    }

    /// Check that a [`ProjectedNode`] has no UUID label.
    ///
    /// Returns `Ok(())` when valid, `Err("display_name")` when a UUID label
    /// was found on the `display_name` field.
    pub fn check_node(node: &ProjectedNode) -> Result<(), &'static str> {
        if let Some(name) = &node.display_name {
            if Self::is_uuid_label(name) {
                return Err("display_name");
            }
        }
        Ok(())
    }

    /// Check that an [`EndpointSummary`] has no UUID label.
    ///
    /// Returns `Ok(())` when valid, `Err("display_name")` when a UUID label
    /// was found.
    pub fn check_endpoint(endpoint: &EndpointSummary) -> Result<(), &'static str> {
        if let Some(name) = &endpoint.display_name {
            if Self::is_uuid_label(name) {
                return Err("display_name");
            }
        }
        Ok(())
    }
}

/// Returns `true` when all bytes in `buf[start..end]` are ASCII lowercase hex
/// digits (`0-9`, `a-f`).
#[inline]
fn is_lower_hex_segment(buf: &[u8], start: usize, end: usize) -> bool {
    buf[start..end]
        .iter()
        .all(|&c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::projection::{
        DirectionClass, EdgeAuthorityClass, NodeMetadata, ProjectedItemId, ProjectedNodeKind,
    };
    use crate::model::{GraphRevision, RecordId, TruthState};

    // ── Helpers ──────────────────────────────────────────────────────────

    fn make_id(suffix: u8) -> EntityId {
        let s = format!("00000000-0000-7000-8000-0000000000{:02x}", suffix);
        EntityId::new(&s).unwrap()
    }

    fn make_node(id: EntityId, display_name: Option<&str>) -> TraversalNode {
        crate::graph::traversal::TraversalNode {
            id,
            is_authorized: true,
            entity_type: Some("person".into()),
            display_name: display_name.map(|s| s.to_owned()),
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

    fn make_edge(src: &EntityId, tgt: &EntityId) -> TraversalEdge {
        crate::graph::traversal::TraversalEdge {
            identity_hash: "e-ab".into(),
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
            evidence_count: 3,
            created_at: None,
            source_kind: None,
            actor_id: None,
            staleness_class: None,
            revision: GraphRevision::base(),
        }
    }

    fn make_projected_node(display_name: Option<&str>) -> ProjectedNode {
        ProjectedNode {
            id: ProjectedItemId::Record(RecordId::new_v7()),
            node_kind: ProjectedNodeKind::Entity,
            authority_class: EdgeAuthorityClass::Stored,
            graph_revision: GraphRevision::base(),
            effective_policy: crate::graph::projection::EffectivePolicySummary {
                namespace: "user".into(),
                scope: "chat".into(),
                sensitivity: 0,
                policy_version: "v1".into(),
            },
            truth_state: TruthStateSummary::bare(TruthState::Current),
            valid_time: None,
            provenance: ProvenanceSummary {
                source_kind: None,
                actor_id: None,
                method: None,
                method_version: None,
                created_at: None,
            },
            display_name: display_name.map(|s| s.to_owned()),
            metadata: NodeMetadata::Entity {
                entity_type: None,
                is_canonical: true,
            },
            authorized_actions: vec![],
        }
    }

    // ── FrontierToken build/decode round-trip ─────────────────────────────

    #[test]
    fn frontier_token_build_decode_roundtrip() {
        let ids = vec![make_id(1), make_id(2), make_id(3)];
        let rev = GraphRevision::new(42);
        let pv = "abc123policy";

        let token = FrontierTokenBuilder::build(&ids, 2, rev, pv);
        let decoded = FrontierTokenBuilder::decode(&token, rev, pv).unwrap();

        assert_eq!(decoded.hop_depth, 2);
        assert_eq!(decoded.graph_revision, rev);
        assert_eq!(decoded.frontier_ids.len(), 3);
        let decoded_strs: Vec<&str> = decoded.frontier_ids.iter().map(|id| id.as_str()).collect();
        assert!(decoded_strs.contains(&ids[0].as_str()));
        assert!(decoded_strs.contains(&ids[1].as_str()));
        assert!(decoded_strs.contains(&ids[2].as_str()));
    }

    #[test]
    fn frontier_token_build_decode_empty_frontier() {
        let rev = GraphRevision::base();
        let token = FrontierTokenBuilder::build(&[], 0, rev, "v1");
        let decoded = FrontierTokenBuilder::decode(&token, rev, "v1").unwrap();
        assert!(decoded.frontier_ids.is_empty());
        assert_eq!(decoded.hop_depth, 0);
    }

    // ── Decode rejects mismatched revision ────────────────────────────────

    #[test]
    fn frontier_token_decode_rejects_mismatched_revision() {
        let ids = vec![make_id(1)];
        let rev_issued = GraphRevision::new(10);
        let rev_current = GraphRevision::new(11); // different
        let pv = "v1";

        let token = FrontierTokenBuilder::build(&ids, 1, rev_issued, pv);
        let result = FrontierTokenBuilder::decode(&token, rev_current, pv);
        assert!(
            result.is_none(),
            "Decode must reject token issued for a different revision"
        );
    }

    // ── Decode rejects mismatched policy_version ──────────────────────────

    #[test]
    fn frontier_token_decode_rejects_mismatched_policy_version() {
        let ids = vec![make_id(1)];
        let rev = GraphRevision::new(5);

        let token = FrontierTokenBuilder::build(&ids, 1, rev, "policy-v1");
        let result = FrontierTokenBuilder::decode(&token, rev, "policy-v2");
        assert!(
            result.is_none(),
            "Decode must reject token issued for a different policy version"
        );
    }

    #[test]
    fn frontier_token_decode_rejects_corrupt_payload() {
        let bad_token = FrontierToken {
            token: "not-valid-base64!!!".into(),
        };
        let result = FrontierTokenBuilder::decode(&bad_token, GraphRevision::base(), "v1");
        assert!(result.is_none());
    }

    // ── EdgeAssembler tests ───────────────────────────────────────────────

    #[test]
    fn edge_assembler_both_endpoints_present_produces_edge() {
        let a = make_id(1);
        let b = make_id(2);
        let edge = make_edge(&a, &b);
        let rev = GraphRevision::new(7);

        let mut lookup = HashMap::new();
        lookup.insert(a.as_str().to_owned(), make_node(a.clone(), Some("Alice")));
        lookup.insert(b.as_str().to_owned(), make_node(b.clone(), Some("Bob")));

        let result = EdgeAssembler::assemble_edge(&edge, &lookup, rev);
        assert!(result.is_some());
        let projected = result.unwrap();
        assert_eq!(projected.id, "e-ab");
        assert_eq!(projected.graph_revision, rev);
        assert_eq!(projected.evidence_count, 3);
        assert_eq!(
            projected.source_endpoint.display_name.as_deref(),
            Some("Alice")
        );
        assert_eq!(
            projected.target_endpoint.display_name.as_deref(),
            Some("Bob")
        );
    }

    #[test]
    fn edge_assembler_source_not_in_lookup_omits_edge() {
        let a = make_id(1);
        let b = make_id(2);
        let edge = make_edge(&a, &b);
        let rev = GraphRevision::new(1);

        let mut lookup = HashMap::new();
        // Only target is in lookup; source is absent.
        lookup.insert(b.as_str().to_owned(), make_node(b.clone(), Some("Bob")));

        let result = EdgeAssembler::assemble_edge(&edge, &lookup, rev);
        assert!(
            result.is_none(),
            "Edge must be omitted when source is missing"
        );
    }

    #[test]
    fn edge_assembler_target_not_in_lookup_omits_edge() {
        let a = make_id(1);
        let b = make_id(2);
        let edge = make_edge(&a, &b);
        let rev = GraphRevision::new(1);

        let mut lookup = HashMap::new();
        // Only source is in lookup; target is absent.
        lookup.insert(a.as_str().to_owned(), make_node(a.clone(), Some("Alice")));

        let result = EdgeAssembler::assemble_edge(&edge, &lookup, rev);
        assert!(
            result.is_none(),
            "Edge must be omitted when target is missing"
        );
    }

    #[test]
    fn edge_assembler_assemble_all_omits_edges_with_missing_endpoints() {
        let a = make_id(1);
        let b = make_id(2);
        let c = make_id(3);
        let edge_ab = make_edge(&a, &b); // both present → included
        let mut edge_ac = make_edge(&a, &c); // c absent → omitted
        edge_ac.identity_hash = "e-ac".into();

        let rev = GraphRevision::new(2);
        let mut lookup = HashMap::new();
        lookup.insert(a.as_str().to_owned(), make_node(a.clone(), Some("Alice")));
        lookup.insert(b.as_str().to_owned(), make_node(b.clone(), Some("Bob")));
        // c is NOT in lookup

        let edges = vec![edge_ab, edge_ac];
        let result = EdgeAssembler::assemble_all(&edges, &lookup, rev);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "e-ab");
    }

    #[test]
    fn edge_assembler_sanitizes_uuid_display_name() {
        let a = make_id(1);
        let b = make_id(2);
        let edge = make_edge(&a, &b);
        let rev = GraphRevision::base();

        let mut lookup = HashMap::new();
        // Source has a UUID as display_name — should be sanitized to None.
        lookup.insert(
            a.as_str().to_owned(),
            make_node(a.clone(), Some("00000000-0000-7000-8000-000000000001")),
        );
        lookup.insert(b.as_str().to_owned(), make_node(b.clone(), Some("Bob")));

        let result = EdgeAssembler::assemble_edge(&edge, &lookup, rev).unwrap();
        assert!(
            result.source_endpoint.display_name.is_none(),
            "UUID display_name must be sanitized to None"
        );
        assert_eq!(result.target_endpoint.display_name.as_deref(), Some("Bob"));
    }

    // ── LabelGuard::is_uuid_label ─────────────────────────────────────────

    #[test]
    fn label_guard_detects_canonical_uuid() {
        // Standard canonical lowercase UUID v4
        assert!(LabelGuard::is_uuid_label(
            "550e8400-e29b-41d4-a716-446655440000"
        ));
        // UUID v7 style (our test IDs)
        assert!(LabelGuard::is_uuid_label(
            "00000000-0000-7000-8000-000000000001"
        ));
    }

    #[test]
    fn label_guard_rejects_non_uuid_names() {
        assert!(!LabelGuard::is_uuid_label("Alice"));
        assert!(!LabelGuard::is_uuid_label("Project KRIA"));
        assert!(!LabelGuard::is_uuid_label(""));
        assert!(!LabelGuard::is_uuid_label("person"));
    }

    #[test]
    fn label_guard_rejects_uppercase_uuid() {
        // Uppercase is NOT our canonical form — must not match.
        assert!(!LabelGuard::is_uuid_label(
            "550E8400-E29B-41D4-A716-446655440000"
        ));
        assert!(!LabelGuard::is_uuid_label(
            "550e8400-E29B-41d4-a716-446655440000"
        ));
    }

    #[test]
    fn label_guard_rejects_wrong_length() {
        // One char too short
        assert!(!LabelGuard::is_uuid_label(
            "550e8400-e29b-41d4-a716-44665544000"
        ));
        // One char too long
        assert!(!LabelGuard::is_uuid_label(
            "550e8400-e29b-41d4-a716-4466554400000"
        ));
    }

    #[test]
    fn label_guard_rejects_wrong_dash_positions() {
        // Dashes moved one position
        assert!(!LabelGuard::is_uuid_label(
            "550e840-0e29b-41d4-a716-446655440000"
        ));
    }

    // ── LabelGuard::sanitize_display_name ────────────────────────────────

    #[test]
    fn sanitize_returns_none_for_uuid() {
        let uuid_name = Some("550e8400-e29b-41d4-a716-446655440000".to_string());
        assert_eq!(LabelGuard::sanitize_display_name(uuid_name), None);
    }

    #[test]
    fn sanitize_passes_through_valid_name() {
        let name = Some("Alice".to_string());
        assert_eq!(
            LabelGuard::sanitize_display_name(name),
            Some("Alice".to_string())
        );
    }

    #[test]
    fn sanitize_passes_through_none() {
        assert_eq!(LabelGuard::sanitize_display_name(None), None);
    }

    // ── LabelGuard::check_node ────────────────────────────────────────────

    #[test]
    fn check_node_ok_when_no_uuid_display_name() {
        let node = make_projected_node(Some("Alice"));
        assert!(LabelGuard::check_node(&node).is_ok());
    }

    #[test]
    fn check_node_ok_when_display_name_is_none() {
        let node = make_projected_node(None);
        assert!(LabelGuard::check_node(&node).is_ok());
    }

    #[test]
    fn check_node_err_when_display_name_is_uuid() {
        let node = make_projected_node(Some("550e8400-e29b-41d4-a716-446655440000"));
        let result = LabelGuard::check_node(&node);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "display_name");
    }

    // ── LabelGuard::check_endpoint ────────────────────────────────────────

    #[test]
    fn check_endpoint_ok_when_valid_name() {
        let endpoint = EndpointSummary {
            id: make_id(1),
            entity_type: None,
            display_name: Some("Bob".into()),
            truth_state: TruthState::Current,
        };
        assert!(LabelGuard::check_endpoint(&endpoint).is_ok());
    }

    #[test]
    fn check_endpoint_err_when_display_name_is_uuid() {
        let endpoint = EndpointSummary {
            id: make_id(1),
            entity_type: None,
            display_name: Some("00000000-0000-7000-8000-000000000001".into()),
            truth_state: TruthState::Current,
        };
        let result = LabelGuard::check_endpoint(&endpoint);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "display_name");
    }

    #[test]
    fn check_endpoint_ok_when_display_name_is_none() {
        let endpoint = EndpointSummary {
            id: make_id(1),
            entity_type: None,
            display_name: None,
            truth_state: TruthState::Current,
        };
        assert!(LabelGuard::check_endpoint(&endpoint).is_ok());
    }
}
