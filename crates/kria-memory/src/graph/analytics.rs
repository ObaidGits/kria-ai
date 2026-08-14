//! Honest analytics vocabulary for graph analysis outputs.
//!
//! **Task 2.4.6** — Implements MGR-011 (honest analytics vocabulary):
//!
//! * [`ComponentMembership`] — connected-component output named `component`
//!   per MGR-011 AC 1. The word "cluster" or "group" is never used.
//! * [`AnalyticsAlgorithmId`] — versioned named algorithm identity, required
//!   on all community and centrality outputs.
//! * [`GraphPredicate`] — the edge/node filter used for analytics; different
//!   predicates produce incomparable results.
//! * [`AnalyticsQuality`] — stability, comparability, and named metric.
//! * [`AnalyticsMetric`] — a named technical metric; never presented as a
//!   probability or percentage without calibration evidence.
//! * [`CommunityOutput`] — community detection result carrying full algorithm
//!   provenance per MGR-011 AC 2.
//! * [`CentralityOutput`] — centrality measurement carrying algorithm and
//!   scope per MGR-011 AC 3.
//! * [`AnalyticsComparabilityGuard`] — checks whether two results from the
//!   same analytics type are comparable.
//!
//! # Design Invariants
//! * A4: No visible claim, score, or metric is invented; technical names are
//!   used verbatim or the value is omitted.
//! * MGR-011 AC 1: Output type for connected components is `component` in
//!   code, contracts, tests, and documentation.
//! * MGR-011 AC 2: Community output includes named algorithm, version,
//!   parameters, graph predicate, Graph_Revision, and quality metadata.
//! * MGR-011 AC 3: Centrality output includes named algorithm and evaluated
//!   scope.
//! * MGR-011 AC 4: Analytical values lacking grounded interpretation use the
//!   technical metric name or are omitted.

use serde::{Deserialize, Serialize};

use crate::model::GraphRevision;

// ── 1. AnalyticsAlgorithmId ───────────────────────────────────────────────

/// A versioned, named analytics algorithm identity.
///
/// Required on all community and centrality outputs to prevent interpreting
/// results across algorithm changes as comparable. Changing any field breaks
/// comparability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyticsAlgorithmId {
    /// The algorithm name (e.g. `"louvain"`, `"label_propagation"`,
    /// `"betweenness"`).
    pub name: String,
    /// The algorithm version string (e.g. `"v1"`, `"2024-01"`).
    pub version: String,
    /// Algorithm-specific parameters as a JSON-compatible string.
    /// Empty string means "default parameters".
    pub parameters: String,
}

// ── 2. GraphPredicate ─────────────────────────────────────────────────────

/// The graph predicate (edge/node filter) used for analytics.
///
/// Different predicates produce incomparable results. Comparability is
/// invalidated when the predicate description or evaluated revision changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphPredicate {
    /// Description of the predicate (e.g. `"all_stored_edges"`,
    /// `"positive_evidence_only"`).
    pub description: String,
    /// The graph revision at which the predicate was evaluated.
    pub evaluated_at_revision: GraphRevision,
}

// ── 3. AnalyticsMetric ────────────────────────────────────────────────────

/// A named analytical metric — never presented as a probability or percentage
/// without calibration evidence.
///
/// MGR-011 AC 4 / design §A4: a value MAY be called confidence only when
/// bounded to `[0.0, 1.0]` with calibration evidence. Without that, use the
/// technical metric name (e.g. `"modularity"`, `"coverage"`,
/// `"performance"`).
///
/// Note: derives `PartialEq` but NOT `Eq` because `value: f64` does not
/// implement `Eq` (NaN != NaN).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsMetric {
    /// The technical metric name (e.g. `"modularity"`, `"coverage"`,
    /// `"performance"`). Never `"probability"` or `"confidence"` unless
    /// calibration evidence exists.
    pub metric_name: String,
    /// The metric value.
    pub value: f64,
    /// The algorithm/version that computed this metric.
    pub computed_by: String,
}

// ── 4. AnalyticsQuality ───────────────────────────────────────────────────

/// Quality metadata for an analytics result.
///
/// Required on community outputs. Indicates whether the result is trustworthy
/// and whether it can be compared to previous results.
///
/// Comparability becomes `false` when algorithm, version, parameters,
/// predicate, or graph revision changes. When `comparable_to_previous` is
/// `false`, `comparability_invalidated_reason` MUST be set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsQuality {
    /// Whether the result is stable (convergence reached).
    pub is_stable: bool,
    /// Number of iterations performed (`None` if not applicable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
    /// Quality score or metric for the result (algorithm-specific). Named by
    /// the algorithm — never presented as a probability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_metric: Option<AnalyticsMetric>,
    /// Whether this result can be compared to a previous result. Becomes
    /// `false` when algorithm, version, parameters, predicate, or revision
    /// changes.
    pub comparable_to_previous: bool,
    /// Reason why comparability was invalidated (when
    /// `comparable_to_previous = false`). MUST be `Some` when
    /// `comparable_to_previous = false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparability_invalidated_reason: Option<String>,
}

impl AnalyticsQuality {
    /// Returns `true` when the quality is internally consistent:
    /// `comparable_to_previous = false` implies `comparability_invalidated_reason` is set.
    pub fn is_consistent(&self) -> bool {
        if !self.comparable_to_previous {
            self.comparability_invalidated_reason.is_some()
        } else {
            true
        }
    }
}

// ── 5. ComponentMembership ────────────────────────────────────────────────

/// A connected component in the graph, named `component` per MGR-011 AC 1.
///
/// The output type for connected-component analysis results. MUST be called
/// `component` in code, contracts, tests, and documentation. No alias to
/// "cluster", "group", "community", or any other term is permitted for this
/// specific result type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentMembership {
    /// A stable opaque component ID (e.g. a hash of the member set).
    pub component_id: String,
    /// The entity/record IDs that are members of this component.
    pub member_ids: Vec<String>,
    /// The member count. MUST equal `member_ids.len()` at construction.
    pub member_count: u32,
    /// The graph revision at which this component was computed.
    pub graph_revision: GraphRevision,
}

impl ComponentMembership {
    /// Construct a `ComponentMembership`, setting `member_count` from
    /// `member_ids.len()`.
    ///
    /// # Panics
    /// Panics if `member_ids.len()` overflows `u32` (more than ~4 billion
    /// members — impossible in practice for a single-laptop authority).
    pub fn new(
        component_id: impl Into<String>,
        member_ids: Vec<String>,
        graph_revision: GraphRevision,
    ) -> Self {
        let member_count = member_ids
            .len()
            .try_into()
            .expect("component member count overflows u32");
        ComponentMembership {
            component_id: component_id.into(),
            member_ids,
            member_count,
            graph_revision,
        }
    }
}

// ── 6. CommunityOutput ────────────────────────────────────────────────────

/// A community detection result, required to carry full algorithm provenance.
///
/// MGR-011 AC 2: includes named algorithm, version, parameters, graph
/// predicate, Graph_Revision, and quality metadata. A `CommunityOutput`
/// cannot be constructed without an [`AnalyticsAlgorithmId`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommunityOutput {
    /// A stable opaque community ID.
    pub community_id: String,
    /// The entity/record IDs in this community.
    pub member_ids: Vec<String>,
    /// The algorithm that produced this community.
    pub algorithm: AnalyticsAlgorithmId,
    /// The graph predicate evaluated to produce this community.
    pub predicate: GraphPredicate,
    /// Quality metadata (stability, comparability, metric).
    pub quality: AnalyticsQuality,
}

// ── 7. CentralityOutput ───────────────────────────────────────────────────

/// A centrality measurement, required to carry algorithm and scope.
///
/// MGR-011 AC 3: includes named algorithm and evaluated scope. The `score`
/// field is named `score` (not `importance`, `rank`, or any interpretive
/// synonym) unless a more specific technical name is warranted by the
/// algorithm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CentralityOutput {
    /// The entity/record ID being measured.
    pub record_id: String,
    /// The centrality score. Named `score` — not `importance`, `rank`, etc.
    pub score: f64,
    /// The algorithm that computed this score.
    pub algorithm: AnalyticsAlgorithmId,
    /// The scope of the graph over which centrality was evaluated
    /// (e.g. `"full_graph"`, `"subgraph_N_hops"`, `"namespace_X"`).
    pub evaluated_scope: String,
    /// The graph revision at which this was computed.
    pub graph_revision: GraphRevision,
}

// ── 8. AnalyticsComparabilityGuard ────────────────────────────────────────

/// Guards comparability between two analytics results.
///
/// Returns whether two results can be compared, and why not if they cannot.
/// Comparability is broken when algorithm name, version, parameters, predicate
/// description, or graph revision differs.
pub struct AnalyticsComparabilityGuard;

impl AnalyticsComparabilityGuard {
    /// Check whether two community outputs are comparable.
    ///
    /// Returns `Ok(())` when comparable, `Err(reason)` when not. Incomparable
    /// when: algorithm name, version, or parameters differ; predicate
    /// description differs; or graph revision (captured in
    /// `predicate.evaluated_at_revision`) differs.
    pub fn check_community_comparability(
        a: &CommunityOutput,
        b: &CommunityOutput,
    ) -> Result<(), String> {
        if a.algorithm.name != b.algorithm.name {
            return Err(format!(
                "algorithm name changed: {:?} → {:?}",
                a.algorithm.name, b.algorithm.name
            ));
        }
        if a.algorithm.version != b.algorithm.version {
            return Err(format!(
                "algorithm version changed: {:?} → {:?}",
                a.algorithm.version, b.algorithm.version
            ));
        }
        if a.algorithm.parameters != b.algorithm.parameters {
            return Err(format!(
                "algorithm parameters changed: {:?} → {:?}",
                a.algorithm.parameters, b.algorithm.parameters
            ));
        }
        if a.predicate.description != b.predicate.description {
            return Err(format!(
                "graph predicate changed: {:?} → {:?}",
                a.predicate.description, b.predicate.description
            ));
        }
        if a.predicate.evaluated_at_revision != b.predicate.evaluated_at_revision {
            return Err(format!(
                "graph revision changed: {} → {}",
                a.predicate.evaluated_at_revision, b.predicate.evaluated_at_revision
            ));
        }
        Ok(())
    }

    /// Check whether two centrality outputs for the same record are
    /// comparable.
    ///
    /// Returns `Ok(())` when comparable, `Err(reason)` when not. Incomparable
    /// when: algorithm name, version, or parameters differ; evaluated scope
    /// differs; or graph revision differs.
    pub fn check_centrality_comparability(
        a: &CentralityOutput,
        b: &CentralityOutput,
    ) -> Result<(), String> {
        if a.algorithm.name != b.algorithm.name {
            return Err(format!(
                "algorithm name changed: {:?} → {:?}",
                a.algorithm.name, b.algorithm.name
            ));
        }
        if a.algorithm.version != b.algorithm.version {
            return Err(format!(
                "algorithm version changed: {:?} → {:?}",
                a.algorithm.version, b.algorithm.version
            ));
        }
        if a.algorithm.parameters != b.algorithm.parameters {
            return Err(format!(
                "algorithm parameters changed: {:?} → {:?}",
                a.algorithm.parameters, b.algorithm.parameters
            ));
        }
        if a.evaluated_scope != b.evaluated_scope {
            return Err(format!(
                "evaluated scope changed: {:?} → {:?}",
                a.evaluated_scope, b.evaluated_scope
            ));
        }
        if a.graph_revision != b.graph_revision {
            return Err(format!(
                "graph revision changed: {} → {}",
                a.graph_revision, b.graph_revision
            ));
        }
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ──────────────────────────────────────────────────────────

    fn algo(name: &str) -> AnalyticsAlgorithmId {
        AnalyticsAlgorithmId {
            name: name.into(),
            version: "v1".into(),
            parameters: "".into(),
        }
    }

    fn predicate(description: &str, revision: u64) -> GraphPredicate {
        GraphPredicate {
            description: description.into(),
            evaluated_at_revision: GraphRevision::new(revision),
        }
    }

    fn community(algorithm: AnalyticsAlgorithmId, predicate: GraphPredicate) -> CommunityOutput {
        CommunityOutput {
            community_id: "c1".into(),
            member_ids: vec!["e1".into(), "e2".into()],
            algorithm,
            predicate,
            quality: AnalyticsQuality {
                is_stable: true,
                iterations: Some(10),
                quality_metric: None,
                comparable_to_previous: true,
                comparability_invalidated_reason: None,
            },
        }
    }

    fn centrality(
        record_id: &str,
        algorithm: AnalyticsAlgorithmId,
        scope: &str,
        revision: u64,
    ) -> CentralityOutput {
        CentralityOutput {
            record_id: record_id.into(),
            score: 0.5,
            algorithm,
            evaluated_scope: scope.into(),
            graph_revision: GraphRevision::new(revision),
        }
    }

    // ── ComponentMembership ───────────────────────────────────────────────

    /// member_count matches member_ids.len() after construction via new().
    #[test]
    fn component_member_count_matches_ids_len() {
        let rev = GraphRevision::new(1);
        let ids = vec!["e1".into(), "e2".into(), "e3".into()];
        let comp = ComponentMembership::new("comp-abc", ids.clone(), rev);
        assert_eq!(comp.member_count, ids.len() as u32);
        assert_eq!(comp.member_ids.len(), comp.member_count as usize);
    }

    #[test]
    fn component_empty_membership() {
        let rev = GraphRevision::new(0);
        let comp = ComponentMembership::new("comp-empty", vec![], rev);
        assert_eq!(comp.member_count, 0);
        assert!(comp.member_ids.is_empty());
    }

    #[test]
    fn component_membership_roundtrips_serde() {
        let comp = ComponentMembership::new(
            "comp-xyz",
            vec!["e1".into(), "e2".into()],
            GraphRevision::new(7),
        );
        let json = serde_json::to_string(&comp).unwrap();
        let back: ComponentMembership = serde_json::from_str(&json).unwrap();
        assert_eq!(comp, back);
    }

    // ── CommunityOutput ───────────────────────────────────────────────────

    /// Community output requires algorithm and predicate fields to be present.
    #[test]
    fn community_output_algorithm_and_predicate_required() {
        let a = algo("louvain");
        let p = predicate("all_stored_edges", 5);
        let c = community(a.clone(), p.clone());
        // Both fields are always present — algorithm and predicate are not
        // Option types; the struct cannot be constructed without them.
        assert_eq!(c.algorithm.name, "louvain");
        assert_eq!(c.predicate.description, "all_stored_edges");
    }

    #[test]
    fn community_output_roundtrips_serde() {
        let c = community(algo("louvain"), predicate("all_stored_edges", 3));
        let json = serde_json::to_string(&c).unwrap();
        let back: CommunityOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(c.algorithm, back.algorithm);
        assert_eq!(c.predicate, back.predicate);
    }

    // ── CentralityOutput ──────────────────────────────────────────────────

    /// Centrality output requires algorithm and evaluated_scope.
    #[test]
    fn centrality_output_algorithm_and_scope_required() {
        let c = centrality("e1", algo("betweenness"), "full_graph", 2);
        // Both fields are non-optional; verifying they are populated.
        assert_eq!(c.algorithm.name, "betweenness");
        assert_eq!(c.evaluated_scope, "full_graph");
    }

    #[test]
    fn centrality_output_roundtrips_serde() {
        let c = centrality("e1", algo("betweenness"), "full_graph", 2);
        let json = serde_json::to_string(&c).unwrap();
        let back: CentralityOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(c.record_id, back.record_id);
        assert_eq!(c.algorithm, back.algorithm);
        assert_eq!(c.evaluated_scope, back.evaluated_scope);
        assert_eq!(c.graph_revision, back.graph_revision);
    }

    // ── AnalyticsComparabilityGuard — community ───────────────────────────

    /// Same algorithm/version/params/predicate/revision → Ok.
    #[test]
    fn community_comparability_identical_is_ok() {
        let a = community(algo("louvain"), predicate("all_stored_edges", 4));
        let b = community(algo("louvain"), predicate("all_stored_edges", 4));
        assert!(AnalyticsComparabilityGuard::check_community_comparability(&a, &b).is_ok());
    }

    /// Different algorithm → Err with a reason mentioning "algorithm".
    #[test]
    fn community_comparability_different_algorithm_is_err() {
        let a = community(algo("louvain"), predicate("all_stored_edges", 4));
        let b = community(algo("label_propagation"), predicate("all_stored_edges", 4));
        let result = AnalyticsComparabilityGuard::check_community_comparability(&a, &b);
        assert!(result.is_err());
        let reason = result.unwrap_err();
        assert!(
            reason.contains("algorithm"),
            "reason must mention 'algorithm': {reason}"
        );
    }

    /// Different algorithm version → Err.
    #[test]
    fn community_comparability_different_version_is_err() {
        let mut b_algo = algo("louvain");
        b_algo.version = "v2".into();
        let a = community(algo("louvain"), predicate("all_stored_edges", 4));
        let b = community(b_algo, predicate("all_stored_edges", 4));
        let result = AnalyticsComparabilityGuard::check_community_comparability(&a, &b);
        assert!(result.is_err());
    }

    /// Different algorithm parameters → Err.
    #[test]
    fn community_comparability_different_params_is_err() {
        let mut b_algo = algo("louvain");
        b_algo.parameters = r#"{"resolution":1.5}"#.into();
        let a = community(algo("louvain"), predicate("all_stored_edges", 4));
        let b = community(b_algo, predicate("all_stored_edges", 4));
        let result = AnalyticsComparabilityGuard::check_community_comparability(&a, &b);
        assert!(result.is_err());
    }

    /// Different predicate → Err with a reason mentioning "predicate".
    #[test]
    fn community_comparability_different_predicate_is_err() {
        let a = community(algo("louvain"), predicate("all_stored_edges", 4));
        let b = community(algo("louvain"), predicate("positive_evidence_only", 4));
        let result = AnalyticsComparabilityGuard::check_community_comparability(&a, &b);
        assert!(result.is_err());
        let reason = result.unwrap_err();
        assert!(
            reason.contains("predicate"),
            "reason must mention 'predicate': {reason}"
        );
    }

    /// Different graph revision → Err with a reason mentioning "revision".
    #[test]
    fn community_comparability_different_revision_is_err() {
        let a = community(algo("louvain"), predicate("all_stored_edges", 4));
        let b = community(algo("louvain"), predicate("all_stored_edges", 5));
        let result = AnalyticsComparabilityGuard::check_community_comparability(&a, &b);
        assert!(result.is_err());
        let reason = result.unwrap_err();
        assert!(
            reason.contains("revision"),
            "reason must mention 'revision': {reason}"
        );
    }

    // ── AnalyticsComparabilityGuard — centrality ──────────────────────────

    /// Same algorithm/version/params/scope/revision → Ok.
    #[test]
    fn centrality_comparability_identical_is_ok() {
        let a = centrality("e1", algo("betweenness"), "full_graph", 3);
        let b = centrality("e1", algo("betweenness"), "full_graph", 3);
        assert!(AnalyticsComparabilityGuard::check_centrality_comparability(&a, &b).is_ok());
    }

    /// Different algorithm → Err.
    #[test]
    fn centrality_comparability_different_algorithm_is_err() {
        let a = centrality("e1", algo("betweenness"), "full_graph", 3);
        let b = centrality("e1", algo("pagerank"), "full_graph", 3);
        let result = AnalyticsComparabilityGuard::check_centrality_comparability(&a, &b);
        assert!(result.is_err());
        let reason = result.unwrap_err();
        assert!(reason.contains("algorithm"), "reason: {reason}");
    }

    /// Different scope → Err.
    #[test]
    fn centrality_comparability_different_scope_is_err() {
        let a = centrality("e1", algo("betweenness"), "full_graph", 3);
        let b = centrality("e1", algo("betweenness"), "subgraph_3_hops", 3);
        let result = AnalyticsComparabilityGuard::check_centrality_comparability(&a, &b);
        assert!(result.is_err());
        let reason = result.unwrap_err();
        assert!(reason.contains("scope"), "reason: {reason}");
    }

    /// Different revision → Err.
    #[test]
    fn centrality_comparability_different_revision_is_err() {
        let a = centrality("e1", algo("betweenness"), "full_graph", 3);
        let b = centrality("e1", algo("betweenness"), "full_graph", 4);
        let result = AnalyticsComparabilityGuard::check_centrality_comparability(&a, &b);
        assert!(result.is_err());
        let reason = result.unwrap_err();
        assert!(reason.contains("revision"), "reason: {reason}");
    }

    // ── AnalyticsQuality consistency ──────────────────────────────────────

    /// comparable_to_previous = false requires comparability_invalidated_reason.
    #[test]
    fn quality_not_comparable_requires_reason() {
        let q = AnalyticsQuality {
            is_stable: true,
            iterations: None,
            quality_metric: None,
            comparable_to_previous: false,
            comparability_invalidated_reason: None,
        };
        assert!(
            !q.is_consistent(),
            "comparable_to_previous=false without reason should be inconsistent"
        );
    }

    #[test]
    fn quality_not_comparable_with_reason_is_consistent() {
        let q = AnalyticsQuality {
            is_stable: true,
            iterations: None,
            quality_metric: None,
            comparable_to_previous: false,
            comparability_invalidated_reason: Some("algorithm version changed: v1 → v2".into()),
        };
        assert!(q.is_consistent());
    }

    #[test]
    fn quality_comparable_with_no_reason_is_consistent() {
        let q = AnalyticsQuality {
            is_stable: true,
            iterations: Some(5),
            quality_metric: Some(AnalyticsMetric {
                metric_name: "modularity".into(),
                value: 0.42,
                computed_by: "louvain-v1".into(),
            }),
            comparable_to_previous: true,
            comparability_invalidated_reason: None,
        };
        assert!(q.is_consistent());
    }

    // ── AnalyticsMetric naming ─────────────────────────────────────────────

    /// Technical metric names are used; the field name is metric_name (not
    /// "probability" or "confidence").
    #[test]
    fn analytics_metric_uses_technical_name() {
        let m = AnalyticsMetric {
            metric_name: "modularity".into(),
            value: 0.72,
            computed_by: "louvain-v1".into(),
        };
        assert_eq!(m.metric_name, "modularity");
        // The struct has no field named "probability" or "confidence".
        let json = serde_json::to_string(&m).unwrap();
        assert!(
            !json.contains("probability"),
            "must not contain 'probability'"
        );
        assert!(
            !json.contains("confidence"),
            "must not contain 'confidence'"
        );
    }
}
