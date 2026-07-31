//! Programmatic fixture builders for interchange import/export testing.
//!
//! Design §A12, MGR-027: "Fixtures cover every record/link/truth/time/entity/
//! source state, unknown values, malformed rows, cyclic graph, and
//! policy-paired world."
//!
//! This module provides in-memory fixture builders only — no files are written
//! to disk. All fixtures are constructed deterministically for unit tests.

use sha2::{Digest, Sha256};

use super::interchange_export::ExportRecord;

// ── sha256_hex (private helper) ───────────────────────────────────────────

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

// ── FixtureRecordKind ─────────────────────────────────────────────────────

/// All record kinds available in interchange fixtures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureRecordKind {
    Memory,
    Entity,
    Relationship,
    Evidence,
    Source,
    Goal,
    Episode,
}

impl FixtureRecordKind {
    /// The canonical string form used in interchange records.
    pub fn as_str(self) -> &'static str {
        match self {
            FixtureRecordKind::Memory => "memory",
            FixtureRecordKind::Entity => "entity",
            FixtureRecordKind::Relationship => "relationship",
            FixtureRecordKind::Evidence => "evidence",
            FixtureRecordKind::Source => "source",
            FixtureRecordKind::Goal => "goal",
            FixtureRecordKind::Episode => "episode",
        }
    }

    /// All record kinds in declaration order.
    pub fn all() -> &'static [FixtureRecordKind] {
        &[
            FixtureRecordKind::Memory,
            FixtureRecordKind::Entity,
            FixtureRecordKind::Relationship,
            FixtureRecordKind::Evidence,
            FixtureRecordKind::Source,
            FixtureRecordKind::Goal,
            FixtureRecordKind::Episode,
        ]
    }
}

// ── FixtureTruthState ─────────────────────────────────────────────────────

/// All truth states available in interchange fixtures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureTruthState {
    Current,
    Unverified,
    Stale,
    Contradicted,
    Superseded,
    Inferred,
    Confirmed,
    Forgotten,
    Deleted,
    Unavailable,
}

impl FixtureTruthState {
    /// The canonical string form used in interchange records.
    pub fn as_str(self) -> &'static str {
        match self {
            FixtureTruthState::Current => "current",
            FixtureTruthState::Unverified => "unverified",
            FixtureTruthState::Stale => "stale",
            FixtureTruthState::Contradicted => "contradicted",
            FixtureTruthState::Superseded => "superseded",
            FixtureTruthState::Inferred => "inferred",
            FixtureTruthState::Confirmed => "confirmed",
            FixtureTruthState::Forgotten => "forgotten",
            FixtureTruthState::Deleted => "deleted",
            FixtureTruthState::Unavailable => "unavailable",
        }
    }

    /// All truth states in declaration order (10 variants).
    pub fn all() -> &'static [FixtureTruthState] {
        &[
            FixtureTruthState::Current,
            FixtureTruthState::Unverified,
            FixtureTruthState::Stale,
            FixtureTruthState::Contradicted,
            FixtureTruthState::Superseded,
            FixtureTruthState::Inferred,
            FixtureTruthState::Confirmed,
            FixtureTruthState::Forgotten,
            FixtureTruthState::Deleted,
            FixtureTruthState::Unavailable,
        ]
    }
}

// ── InterchangeFixtureSet ─────────────────────────────────────────────────

/// A complete fixture set for interchange testing.
pub struct InterchangeFixtureSet {
    /// The records in this fixture set.
    pub records: Vec<ExportRecord>,
    /// A short human-readable label.
    pub label: String,
    /// A description of what this fixture tests.
    pub description: String,
}

// ── FixtureRecordFactory ──────────────────────────────────────────────────

/// Builds single [`ExportRecord`] instances for use in fixtures.
pub struct FixtureRecordFactory;

impl FixtureRecordFactory {
    /// Build a valid [`ExportRecord`] with the given parameters.
    ///
    /// The `content_json` is constructed from the provided fields, and the
    /// `content_hash` is computed from the JSON so `verify_hash()` passes.
    /// If `extra_fields` is `Some`, those fields are merged into the JSON object.
    pub fn build(
        record_id: &str,
        record_kind: &str,
        truth_state: &str,
        sensitivity: u8,
        policy_namespace: &str,
        policy_scope: &str,
        extra_fields: Option<serde_json::Value>,
    ) -> ExportRecord {
        let mut obj = serde_json::json!({
            "id": record_id,
            "kind": record_kind,
            "truth_state": truth_state,
            "content": format!("fixture content for {} {}", record_kind, record_id)
        });

        // Merge extra fields into the JSON object
        if let Some(extra) = extra_fields {
            if let (Some(obj_map), Some(extra_map)) = (obj.as_object_mut(), extra.as_object()) {
                for (k, v) in extra_map {
                    obj_map.insert(k.clone(), v.clone());
                }
            }
        }

        let content_json =
            serde_json::to_string(&obj).expect("fixture JSON serialization must not fail");
        let content_hash = sha256_hex(content_json.as_bytes());

        ExportRecord {
            record_kind: record_kind.to_string(),
            record_id: record_id.to_string(),
            content_json,
            content_hash,
            revision: 1,
            policy_namespace: policy_namespace.to_string(),
            policy_scope: policy_scope.to_string(),
            sensitivity,
        }
    }

    /// Build a record with a broken content hash (for malformed-row tests).
    ///
    /// The `content_hash` is deliberately set to an all-zeros placeholder so
    /// that `verify_hash()` returns `Err(HashMismatch)`.
    pub fn build_with_bad_hash(record_id: &str) -> ExportRecord {
        let content_json = serde_json::to_string(&serde_json::json!({
            "id": record_id,
            "kind": "memory",
            "content": "malformed hash fixture"
        }))
        .expect("fixture JSON serialization must not fail");

        ExportRecord {
            record_kind: "memory".to_string(),
            record_id: record_id.to_string(),
            content_json,
            content_hash: "0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            revision: 1,
            policy_namespace: "default".to_string(),
            policy_scope: "personal".to_string(),
            sensitivity: 0,
        }
    }
}

// ── InterchangeFixtureBuilder ─────────────────────────────────────────────

/// Generates complete [`InterchangeFixtureSet`] instances for testing.
pub struct InterchangeFixtureBuilder;

impl InterchangeFixtureBuilder {
    /// Build a fixture with one record per truth state.
    ///
    /// Total: 10 records (one per [`FixtureTruthState`] variant).
    /// All records have valid hashes and pass `verify_hash()`.
    pub fn all_truth_states(policy_namespace: &str, policy_scope: &str) -> InterchangeFixtureSet {
        let records = FixtureTruthState::all()
            .iter()
            .enumerate()
            .map(|(i, &state)| {
                FixtureRecordFactory::build(
                    &format!("truth-{}", state.as_str()),
                    "memory",
                    state.as_str(),
                    (i % 3) as u8, // sensitivity cycles 0,1,2
                    policy_namespace,
                    policy_scope,
                    None,
                )
            })
            .collect();

        InterchangeFixtureSet {
            records,
            label: "all_truth_states".to_string(),
            description: "One record per FixtureTruthState (10 total)".to_string(),
        }
    }

    /// Build a fixture with one record per record kind.
    ///
    /// Total: 7 records (one per [`FixtureRecordKind`] variant).
    /// All records have valid hashes and pass `verify_hash()`.
    pub fn all_record_kinds(policy_namespace: &str, policy_scope: &str) -> InterchangeFixtureSet {
        let records = FixtureRecordKind::all()
            .iter()
            .map(|&kind| {
                FixtureRecordFactory::build(
                    &format!("kind-{}", kind.as_str()),
                    kind.as_str(),
                    "current",
                    0,
                    policy_namespace,
                    policy_scope,
                    None,
                )
            })
            .collect();

        InterchangeFixtureSet {
            records,
            label: "all_record_kinds".to_string(),
            description: "One record per FixtureRecordKind (7 total)".to_string(),
        }
    }

    /// Build a fixture with unknown/extension fields in the content JSON.
    ///
    /// Records include extra fields that v1 parsers don't know about. These
    /// must parse correctly and survive a JSON round-trip unchanged. All
    /// records have valid hashes.
    pub fn with_unknown_optional_fields() -> InterchangeFixtureSet {
        let records = vec![
            FixtureRecordFactory::build(
                "unknown-fields-1",
                "memory",
                "current",
                0,
                "default",
                "personal",
                Some(serde_json::json!({
                    "future_v2_field": "some_future_value",
                    "another_extension": 42,
                    "nested_unknown": {"foo": "bar", "baz": [1, 2, 3]}
                })),
            ),
            FixtureRecordFactory::build(
                "unknown-fields-2",
                "entity",
                "confirmed",
                1,
                "default",
                "personal",
                Some(serde_json::json!({
                    "v3_extension_flag": true,
                    "experimental_score": 0.95
                })),
            ),
        ];

        InterchangeFixtureSet {
            records,
            label: "with_unknown_optional_fields".to_string(),
            description: "Records with extra JSON fields that v1 parsers don't know about; \
                          must round-trip unchanged"
                .to_string(),
        }
    }

    /// Build a fixture containing a malformed row (invalid content_hash).
    ///
    /// The single record's `content_hash` does not match its `content_json`,
    /// so `verify_hash()` returns `Err(HashMismatch)`. Used to test that
    /// malformed records are rejected at import.
    pub fn with_malformed_hash() -> InterchangeFixtureSet {
        let records = vec![FixtureRecordFactory::build_with_bad_hash(
            "malformed-hash-record",
        )];

        InterchangeFixtureSet {
            records,
            label: "with_malformed_hash".to_string(),
            description: "One record with a deliberately broken content_hash; \
                          verify_hash() must return Err(HashMismatch)"
                .to_string(),
        }
    }

    /// Build a fixture containing a cyclic graph (A→B→C→A relationships).
    ///
    /// Contains 3 entity records (A, B, C) and 3 relationship records
    /// (A→B, B→C, C→A), totaling 6 records. All records have valid hashes.
    pub fn cyclic_graph(policy_namespace: &str, policy_scope: &str) -> InterchangeFixtureSet {
        let mut records = Vec::with_capacity(6);

        // 3 entity records
        for label in &["entity-A", "entity-B", "entity-C"] {
            records.push(FixtureRecordFactory::build(
                label,
                "entity",
                "current",
                0,
                policy_namespace,
                policy_scope,
                Some(serde_json::json!({
                    "display_name": label
                })),
            ));
        }

        // 3 relationship records forming a cycle A→B→C→A
        let edges = [
            ("rel-A-to-B", "entity-A", "entity-B"),
            ("rel-B-to-C", "entity-B", "entity-C"),
            ("rel-C-to-A", "entity-C", "entity-A"),
        ];
        for (rel_id, source, target) in &edges {
            records.push(FixtureRecordFactory::build(
                rel_id,
                "relationship",
                "current",
                0,
                policy_namespace,
                policy_scope,
                Some(serde_json::json!({
                    "source_id": source,
                    "target_id": target,
                    "relation_name": "related_to"
                })),
            ));
        }

        InterchangeFixtureSet {
            records,
            label: "cyclic_graph".to_string(),
            description: "3 entity records (A, B, C) and 3 relationship records (A→B, B→C, C→A) \
                          forming a cyclic graph"
                .to_string(),
        }
    }

    /// Build a policy-paired world: two fixture sets with different namespaces.
    ///
    /// Returns `(world_a, world_b)` where `world_a` uses namespace `"world_a"`
    /// and `world_b` uses namespace `"world_b"`. Used to verify policy
    /// isolation in round-trip tests.
    pub fn policy_paired_world() -> (InterchangeFixtureSet, InterchangeFixtureSet) {
        let world_a_records = vec![
            FixtureRecordFactory::build(
                "wa-mem-1", "memory", "current", 0, "world_a", "personal", None,
            ),
            FixtureRecordFactory::build(
                "wa-ent-1",
                "entity",
                "confirmed",
                1,
                "world_a",
                "personal",
                None,
            ),
        ];

        let world_b_records = vec![
            FixtureRecordFactory::build(
                "wb-mem-1", "memory", "current", 0, "world_b", "personal", None,
            ),
            FixtureRecordFactory::build(
                "wb-ent-1",
                "entity",
                "confirmed",
                1,
                "world_b",
                "personal",
                None,
            ),
        ];

        (
            InterchangeFixtureSet {
                records: world_a_records,
                label: "world_a".to_string(),
                description: "Policy-paired world A (namespace=world_a)".to_string(),
            },
            InterchangeFixtureSet {
                records: world_b_records,
                label: "world_b".to_string(),
                description: "Policy-paired world B (namespace=world_b)".to_string(),
            },
        )
    }

    /// Build a fixture with valid time boundaries (past, future, open).
    ///
    /// Produces records with three distinct temporal boundary patterns:
    /// - Past: `valid_from` in 2020, `valid_until` in 2023
    /// - Future: `valid_from` in 2030, `valid_until` open
    /// - Open: no `valid_from` or `valid_until`
    ///
    /// All records have valid hashes.
    pub fn temporal_boundaries(
        policy_namespace: &str,
        policy_scope: &str,
    ) -> InterchangeFixtureSet {
        let records = vec![
            // Past-bounded record
            FixtureRecordFactory::build(
                "temporal-past",
                "memory",
                "stale",
                0,
                policy_namespace,
                policy_scope,
                Some(serde_json::json!({
                    "valid_from": "2020-01-01T00:00:00+00:00",
                    "valid_until": "2023-12-31T23:59:59+00:00"
                })),
            ),
            // Future-bounded record
            FixtureRecordFactory::build(
                "temporal-future",
                "memory",
                "current",
                0,
                policy_namespace,
                policy_scope,
                Some(serde_json::json!({
                    "valid_from": "2030-01-01T00:00:00+00:00",
                    "valid_until": null
                })),
            ),
            // Fully open record
            FixtureRecordFactory::build(
                "temporal-open",
                "memory",
                "current",
                0,
                policy_namespace,
                policy_scope,
                Some(serde_json::json!({
                    "valid_from": null,
                    "valid_until": null
                })),
            ),
        ];

        InterchangeFixtureSet {
            records,
            label: "temporal_boundaries".to_string(),
            description: "Records with past-bounded, future-bounded, and open valid time intervals"
                .to_string(),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── FixtureRecordKind ─────────────────────────────────────────────────

    #[test]
    fn fixture_record_kind_all_has_7_variants() {
        assert_eq!(FixtureRecordKind::all().len(), 7);
    }

    #[test]
    fn fixture_record_kind_as_str_is_nonempty() {
        for kind in FixtureRecordKind::all() {
            assert!(!kind.as_str().is_empty());
        }
    }

    #[test]
    fn fixture_record_kind_strings_are_distinct() {
        let strs: Vec<_> = FixtureRecordKind::all()
            .iter()
            .map(|k| k.as_str())
            .collect();
        let mut sorted = strs.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            strs.len(),
            "all kind strings must be distinct"
        );
    }

    // ── FixtureTruthState ─────────────────────────────────────────────────

    #[test]
    fn fixture_truth_state_all_has_10_variants() {
        assert_eq!(FixtureTruthState::all().len(), 10);
    }

    #[test]
    fn fixture_truth_state_as_str_is_nonempty() {
        for state in FixtureTruthState::all() {
            assert!(!state.as_str().is_empty());
        }
    }

    #[test]
    fn fixture_truth_state_strings_are_distinct() {
        let strs: Vec<_> = FixtureTruthState::all()
            .iter()
            .map(|s| s.as_str())
            .collect();
        let mut sorted = strs.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            strs.len(),
            "all truth state strings must be distinct"
        );
    }

    // ── FixtureRecordFactory ──────────────────────────────────────────────

    #[test]
    fn factory_build_produces_valid_hash() {
        let record = FixtureRecordFactory::build(
            "test-id", "memory", "current", 0, "default", "personal", None,
        );
        assert!(
            record.verify_hash().is_ok(),
            "FixtureRecordFactory::build must produce valid hash; got: {:?}",
            record.verify_hash()
        );
    }

    #[test]
    fn factory_build_with_extra_fields_has_valid_hash() {
        let record = FixtureRecordFactory::build(
            "extra-id",
            "entity",
            "confirmed",
            1,
            "ns",
            "scope",
            Some(serde_json::json!({ "custom_field": "custom_value" })),
        );
        assert!(record.verify_hash().is_ok());
        // Extra field should be present in the JSON
        let parsed: serde_json::Value = serde_json::from_str(&record.content_json).unwrap();
        assert_eq!(parsed["custom_field"], "custom_value");
    }

    #[test]
    fn factory_build_with_bad_hash_fails_verify() {
        let record = FixtureRecordFactory::build_with_bad_hash("bad-hash");
        assert!(
            record.verify_hash().is_err(),
            "build_with_bad_hash must produce a record that fails verify_hash()"
        );
    }

    // ── InterchangeFixtureBuilder::all_truth_states ───────────────────────

    #[test]
    fn all_truth_states_produces_10_records() {
        let fixture = InterchangeFixtureBuilder::all_truth_states("default", "personal");
        assert_eq!(fixture.records.len(), 10);
    }

    #[test]
    fn all_truth_states_all_records_have_valid_hash() {
        let fixture = InterchangeFixtureBuilder::all_truth_states("default", "personal");
        for record in &fixture.records {
            assert!(
                record.verify_hash().is_ok(),
                "record {} has invalid hash",
                record.record_id
            );
        }
    }

    #[test]
    fn all_truth_states_covers_all_truth_state_strings() {
        let fixture = InterchangeFixtureBuilder::all_truth_states("default", "personal");
        let expected: Vec<_> = FixtureTruthState::all()
            .iter()
            .map(|s| s.as_str())
            .collect();
        for expected_state in &expected {
            let found = fixture.records.iter().any(|r| {
                let v: serde_json::Value = serde_json::from_str(&r.content_json).unwrap();
                v["truth_state"].as_str() == Some(expected_state)
            });
            assert!(found, "truth state {expected_state:?} not found in fixture");
        }
    }

    // ── InterchangeFixtureBuilder::all_record_kinds ───────────────────────

    #[test]
    fn all_record_kinds_produces_7_records() {
        let fixture = InterchangeFixtureBuilder::all_record_kinds("default", "personal");
        assert_eq!(fixture.records.len(), 7);
    }

    #[test]
    fn all_record_kinds_all_records_have_valid_hash() {
        let fixture = InterchangeFixtureBuilder::all_record_kinds("default", "personal");
        for record in &fixture.records {
            assert!(
                record.verify_hash().is_ok(),
                "record {} has invalid hash",
                record.record_id
            );
        }
    }

    #[test]
    fn all_record_kinds_covers_all_kind_strings() {
        let fixture = InterchangeFixtureBuilder::all_record_kinds("default", "personal");
        for kind in FixtureRecordKind::all() {
            let found = fixture
                .records
                .iter()
                .any(|r| r.record_kind == kind.as_str());
            assert!(
                found,
                "record kind {:?} not found in fixture",
                kind.as_str()
            );
        }
    }

    // ── InterchangeFixtureBuilder::with_unknown_optional_fields ──────────

    #[test]
    fn with_unknown_optional_fields_records_have_extra_json_fields() {
        let fixture = InterchangeFixtureBuilder::with_unknown_optional_fields();
        assert!(!fixture.records.is_empty());

        // Every record should have at least one "extra" (non-standard) field
        for record in &fixture.records {
            let v: serde_json::Value = serde_json::from_str(&record.content_json).unwrap();
            let obj = v.as_object().unwrap();
            // Standard fields: id, kind, truth_state, content — extra fields are anything else
            let standard = ["id", "kind", "truth_state", "content"];
            let has_extra = obj.keys().any(|k| !standard.contains(&k.as_str()));
            assert!(has_extra, "record {} has no extra fields", record.record_id);
        }
    }

    #[test]
    fn with_unknown_optional_fields_records_have_valid_hash() {
        let fixture = InterchangeFixtureBuilder::with_unknown_optional_fields();
        for record in &fixture.records {
            assert!(
                record.verify_hash().is_ok(),
                "record {} has invalid hash",
                record.record_id
            );
        }
    }

    #[test]
    fn with_unknown_optional_fields_extra_fields_survive_json_roundtrip() {
        let fixture = InterchangeFixtureBuilder::with_unknown_optional_fields();
        for record in &fixture.records {
            let v1: serde_json::Value = serde_json::from_str(&record.content_json).unwrap();
            let serialized = serde_json::to_string(&v1).unwrap();
            let v2: serde_json::Value = serde_json::from_str(&serialized).unwrap();
            assert_eq!(
                v1, v2,
                "record {} extra fields changed after round-trip",
                record.record_id
            );
        }
    }

    // ── InterchangeFixtureBuilder::with_malformed_hash ────────────────────

    #[test]
    fn with_malformed_hash_record_fails_verify_hash() {
        let fixture = InterchangeFixtureBuilder::with_malformed_hash();
        assert_eq!(fixture.records.len(), 1);
        assert!(
            fixture.records[0].verify_hash().is_err(),
            "malformed hash record must fail verify_hash()"
        );
    }

    // ── InterchangeFixtureBuilder::cyclic_graph ───────────────────────────

    #[test]
    fn cyclic_graph_has_6_records() {
        let fixture = InterchangeFixtureBuilder::cyclic_graph("default", "personal");
        assert_eq!(fixture.records.len(), 6);
    }

    #[test]
    fn cyclic_graph_has_3_entities_and_3_relationships() {
        let fixture = InterchangeFixtureBuilder::cyclic_graph("default", "personal");
        let entity_count = fixture
            .records
            .iter()
            .filter(|r| r.record_kind == "entity")
            .count();
        let rel_count = fixture
            .records
            .iter()
            .filter(|r| r.record_kind == "relationship")
            .count();
        assert_eq!(entity_count, 3, "expected 3 entity records");
        assert_eq!(rel_count, 3, "expected 3 relationship records");
    }

    #[test]
    fn cyclic_graph_all_records_have_valid_hash() {
        let fixture = InterchangeFixtureBuilder::cyclic_graph("default", "personal");
        for record in &fixture.records {
            assert!(
                record.verify_hash().is_ok(),
                "record {} has invalid hash",
                record.record_id
            );
        }
    }

    #[test]
    fn cyclic_graph_relationships_form_cycle() {
        let fixture = InterchangeFixtureBuilder::cyclic_graph("default", "personal");
        let rels: Vec<_> = fixture
            .records
            .iter()
            .filter(|r| r.record_kind == "relationship")
            .map(|r| {
                let v: serde_json::Value = serde_json::from_str(&r.content_json).unwrap();
                (
                    v["source_id"].as_str().unwrap().to_string(),
                    v["target_id"].as_str().unwrap().to_string(),
                )
            })
            .collect();

        // Expect edges A→B, B→C, C→A
        let expected_edges = [
            ("entity-A", "entity-B"),
            ("entity-B", "entity-C"),
            ("entity-C", "entity-A"),
        ];
        for (src, tgt) in &expected_edges {
            let found = rels.iter().any(|(s, t)| s == src && t == tgt);
            assert!(found, "expected edge {src}→{tgt} not found in cyclic_graph");
        }
    }

    // ── InterchangeFixtureBuilder::policy_paired_world ────────────────────

    #[test]
    fn policy_paired_world_has_different_namespaces() {
        let (world_a, world_b) = InterchangeFixtureBuilder::policy_paired_world();
        assert_eq!(world_a.label, "world_a");
        assert_eq!(world_b.label, "world_b");

        for r in &world_a.records {
            assert_eq!(r.policy_namespace, "world_a");
        }
        for r in &world_b.records {
            assert_eq!(r.policy_namespace, "world_b");
        }
    }

    #[test]
    fn policy_paired_world_records_have_valid_hash() {
        let (world_a, world_b) = InterchangeFixtureBuilder::policy_paired_world();
        for record in world_a.records.iter().chain(world_b.records.iter()) {
            assert!(
                record.verify_hash().is_ok(),
                "record {} has invalid hash",
                record.record_id
            );
        }
    }

    #[test]
    fn policy_paired_world_namespaces_are_disjoint() {
        let (world_a, world_b) = InterchangeFixtureBuilder::policy_paired_world();
        let ns_a: std::collections::HashSet<_> = world_a
            .records
            .iter()
            .map(|r| r.policy_namespace.as_str())
            .collect();
        let ns_b: std::collections::HashSet<_> = world_b
            .records
            .iter()
            .map(|r| r.policy_namespace.as_str())
            .collect();
        // No namespace in world_a should appear in world_b
        assert!(
            ns_a.is_disjoint(&ns_b),
            "world_a and world_b must have disjoint policy namespaces"
        );
    }

    // ── InterchangeFixtureBuilder::temporal_boundaries ───────────────────

    #[test]
    fn temporal_boundaries_produces_3_records() {
        let fixture = InterchangeFixtureBuilder::temporal_boundaries("default", "personal");
        assert_eq!(fixture.records.len(), 3);
    }

    #[test]
    fn temporal_boundaries_all_records_have_valid_hash() {
        let fixture = InterchangeFixtureBuilder::temporal_boundaries("default", "personal");
        for record in &fixture.records {
            assert!(
                record.verify_hash().is_ok(),
                "record {} has invalid hash",
                record.record_id
            );
        }
    }

    #[test]
    fn temporal_boundaries_has_past_future_and_open_records() {
        let fixture = InterchangeFixtureBuilder::temporal_boundaries("default", "personal");
        let ids: Vec<_> = fixture
            .records
            .iter()
            .map(|r| r.record_id.as_str())
            .collect();
        assert!(
            ids.contains(&"temporal-past"),
            "missing past temporal record"
        );
        assert!(
            ids.contains(&"temporal-future"),
            "missing future temporal record"
        );
        assert!(
            ids.contains(&"temporal-open"),
            "missing open temporal record"
        );
    }
}
