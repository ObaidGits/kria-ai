//! `mg-unit-v2` deterministic fixture generator (task F0.2 / 0.2.1).
//!
//! Seed `0x4D475201`. Produces exactly 100 records that plant every required
//! unit-test case from `validation.md` §2:
//!
//! * **every record kind** ([`RecordKind::ALL`]) and **every link kind**
//!   ([`LinkKind::ALL`]),
//! * **all truth states** ([`TruthState::ALL`]) and **all memory modes**
//!   ([`MemoryMode::ALL`]),
//! * the **policy lattice** (every sensitivity `0..=3` across several
//!   namespace/scope/owner combinations),
//! * **invalid rows** that violate schema constraints (for negative testing),
//! * a **duplicate idempotency key** collision (same semantic identity).
//!
//! All content is synthetic — no real private data — and the expected answers
//! are defined here (the independent oracle), not derived from any system under
//! test, which does not yet exist.

use serde::{Deserialize, Serialize};

use super::{
    package_files_and_hash, sha256_hex, ExpectedAnswers, FixtureCounts, FixtureGenerator,
    FixtureManifest, FixturePackage, GeneratorMetadata, IdempotencyCollision, InvalidCase,
    LinkKind, MemoryMode, Policy, RecordKind, SchemaVersions, SplitMix64, TruthState,
    FIXTURE_MANIFEST_SCHEMA, GENERATOR_VERSION,
};

/// The frozen seed for `mg-unit-v2` (`validation.md` §2).
pub const SEED: u64 = 0x4D47_5201;

/// The fixture identifier.
pub const FIXTURE_ID: &str = "mg-unit-v2";

/// Total records planted (`validation.md`: "100 records").
pub const TOTAL_RECORDS: usize = 100;

/// One planted record row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureRecord {
    /// Canonical record ID.
    pub id: String,
    /// Record kind code (may be an invalid value for planted defects).
    pub record_kind: String,
    /// Truth state code (may be invalid for planted defects).
    pub truth_state: String,
    /// Memory mode code.
    pub memory_mode: String,
    /// Effective policy tuple.
    pub policy: Policy,
    /// Caller partition for idempotency accounting.
    pub caller_partition: String,
    /// Idempotency key (shared across a collision group).
    pub idempotency_key: String,
    /// Command hash bound to this record's write.
    pub command_hash: String,
    /// Half-open valid interval start (RFC3339 UTC), if present.
    pub valid_from: Option<String>,
    /// Half-open valid interval end (RFC3339 UTC), if present.
    pub valid_until: Option<String>,
    /// Synthetic content payload.
    pub content: String,
    /// SHA-256 of `content`.
    pub content_hash: String,
    /// Whether the row satisfies every schema constraint.
    pub valid: bool,
    /// Reason code when `valid == false`.
    pub invalid_reason: Option<String>,
}

/// One planted link (edge) row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureLink {
    /// Canonical link ID.
    pub id: String,
    /// Link kind code (may be invalid for planted defects).
    pub link_type: String,
    /// Source endpoint record kind.
    pub source_kind: String,
    /// Source endpoint record ID.
    pub source_id: String,
    /// Target endpoint record kind.
    pub target_kind: String,
    /// Target endpoint record ID.
    pub target_id: String,
    /// Truth state code.
    pub truth_state: String,
    /// Whether the row satisfies every schema/endpoint constraint.
    pub valid: bool,
    /// Reason code when `valid == false`.
    pub invalid_reason: Option<String>,
}

/// The `mg-unit-v2` generator.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnitV2Generator;

impl FixtureGenerator for UnitV2Generator {
    fn fixture_id(&self) -> &'static str {
        FIXTURE_ID
    }

    fn seed(&self) -> u64 {
        SEED
    }

    fn generate(&self) -> FixturePackage {
        let mut rng = SplitMix64::new(SEED);

        let records = build_records(&mut rng);
        let links = build_links(&mut rng, &records);

        // Serialize data files (canonical pretty JSON + trailing newline).
        let records_bytes = to_json_bytes(&records);
        let links_bytes = to_json_bytes(&links);
        let data_files = vec![
            ("records.json".to_string(), records_bytes),
            ("links.json".to_string(), links_bytes),
        ];

        let (files, package_sha256) = package_files_and_hash(&data_files);
        let counts = compute_counts(&records, &links);
        let expected = compute_expected(&records, &links);

        let manifest = FixtureManifest {
            schema_version: FIXTURE_MANIFEST_SCHEMA.to_string(),
            fixture_id: FIXTURE_ID.to_string(),
            generator: GeneratorMetadata {
                name: "memory_graph::fixtures::unit_v2".to_string(),
                version: GENERATOR_VERSION.to_string(),
                algorithm: "splitmix64".to_string(),
                seed_hex: format!("0x{SEED:08X}"),
                seed: SEED,
            },
            schema_versions: SchemaVersions::default(),
            counts,
            expected,
            files,
            package_sha256,
            contains_private_data: false,
            scene_coverage: None,
            release_oracle: None,
            paired_world_oracle: None,
            vector_oracle: None,
            judged_corpus_oracle: None,
            interchange_oracle: None,
            visual_scene_oracle: None,
        };

        FixturePackage {
            fixture_id: FIXTURE_ID.to_string(),
            data_files,
            manifest,
        }
    }
}

/// Serialize a value to canonical pretty JSON bytes with a trailing newline.
fn to_json_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    let mut s = serde_json::to_string_pretty(value).expect("serializes to JSON");
    s.push('\n');
    s.into_bytes()
}

/// A small deterministic policy lattice: namespaces × scopes × owners.
const NAMESPACES: [&str; 3] = ["personal", "work", "shared"];
const SCOPES: [&str; 3] = ["private", "team", "public"];
const OWNERS: [&str; 2] = ["owner-alpha", "owner-beta"];

/// Fixed synthetic timestamps (RFC3339 UTC).
const T0: &str = "2024-01-01T00:00:00Z";
const T1: &str = "2024-06-01T00:00:00Z";
const T2: &str = "2025-01-01T00:00:00Z";

/// Build exactly [`TOTAL_RECORDS`] rows: coverage + collision + invalid.
fn build_records(rng: &mut SplitMix64) -> Vec<FixtureRecord> {
    let mut records = Vec::with_capacity(TOTAL_RECORDS);

    // -- Coverage rows (valid) ------------------------------------------------
    // Cycling each enum guarantees every record kind (11), truth state (10),
    // memory mode (5), and sensitivity level (0..=3) appears at least once.
    const COVERAGE: usize = 93;
    for i in 0..COVERAGE {
        let kind = RecordKind::ALL[i % RecordKind::ALL.len()];
        let truth = TruthState::ALL[i % TruthState::ALL.len()];
        let mode = MemoryMode::ALL[i % MemoryMode::ALL.len()];
        let sensitivity = (i % 4) as i64;
        let policy = Policy {
            namespace: NAMESPACES[rng.below(NAMESPACES.len())].to_string(),
            owner: OWNERS[rng.below(OWNERS.len())].to_string(),
            scope: SCOPES[rng.below(SCOPES.len())].to_string(),
            sensitivity,
        };
        let content = format!("synthetic unit-v2 record {i} :: {}", kind.code());
        let id = rng.next_uuid();
        records.push(FixtureRecord {
            id,
            record_kind: kind.code().to_string(),
            truth_state: truth.code().to_string(),
            memory_mode: mode.code().to_string(),
            policy,
            caller_partition: "unit-v2/default".to_string(),
            idempotency_key: format!("idem-{i:04}"),
            command_hash: sha256_hex(content.as_bytes()),
            valid_from: Some(T0.to_string()),
            valid_until: Some(T2.to_string()),
            content_hash: sha256_hex(content.as_bytes()),
            content,
            valid: true,
            invalid_reason: None,
        });
    }

    // -- Idempotency collision (valid rows, shared semantic identity) ---------
    // Two records share `(caller_partition, idempotency_key)` but carry
    // different command hashes: a replay-conflict case.
    let shared_partition = "unit-v2/idempotent";
    let shared_key = "idem-collision-0001";
    for j in 0..2 {
        let content = format!("synthetic unit-v2 idempotency collision {j}");
        let id = rng.next_uuid();
        records.push(FixtureRecord {
            id,
            record_kind: RecordKind::Memory.code().to_string(),
            truth_state: TruthState::Current.code().to_string(),
            memory_mode: MemoryMode::Permanent.code().to_string(),
            policy: Policy {
                namespace: "personal".to_string(),
                owner: "owner-alpha".to_string(),
                scope: "private".to_string(),
                sensitivity: 1,
            },
            caller_partition: shared_partition.to_string(),
            idempotency_key: shared_key.to_string(),
            // Different command hashes → genuine conflict on replay.
            command_hash: sha256_hex(format!("collision-command-{j}").as_bytes()),
            valid_from: Some(T0.to_string()),
            valid_until: Some(T2.to_string()),
            content_hash: sha256_hex(content.as_bytes()),
            content,
            valid: true,
            invalid_reason: None,
        });
    }

    // -- Invalid rows (planted schema violations) -----------------------------
    let invalid_specs: [(&str, InvalidRowMutation); 5] = [
        ("unknown_record_kind", InvalidRowMutation::UnknownKind),
        (
            "sensitivity_out_of_range",
            InvalidRowMutation::SensitivityOutOfRange,
        ),
        (
            "invalid_valid_interval",
            InvalidRowMutation::InvertedInterval,
        ),
        ("empty_namespace", InvalidRowMutation::EmptyNamespace),
        ("unknown_truth_state", InvalidRowMutation::UnknownTruthState),
    ];
    for (reason, mutation) in invalid_specs {
        let content = format!("synthetic unit-v2 invalid row :: {reason}");
        let id = rng.next_uuid();
        let mut rec = FixtureRecord {
            id,
            record_kind: RecordKind::Memory.code().to_string(),
            truth_state: TruthState::Current.code().to_string(),
            memory_mode: MemoryMode::Permanent.code().to_string(),
            policy: Policy {
                namespace: "work".to_string(),
                owner: "owner-beta".to_string(),
                scope: "team".to_string(),
                sensitivity: 2,
            },
            caller_partition: "unit-v2/invalid".to_string(),
            idempotency_key: format!("idem-invalid-{reason}"),
            command_hash: sha256_hex(content.as_bytes()),
            valid_from: Some(T0.to_string()),
            valid_until: Some(T2.to_string()),
            content_hash: sha256_hex(content.as_bytes()),
            content,
            valid: false,
            invalid_reason: Some(reason.to_string()),
        };
        mutation.apply(&mut rec);
        records.push(rec);
    }

    debug_assert_eq!(records.len(), TOTAL_RECORDS);
    records
}

/// The specific schema-constraint violation planted in an invalid row.
enum InvalidRowMutation {
    /// `record_kind` outside `memory/summary/skill/rule/...`.
    UnknownKind,
    /// `sensitivity` outside `0..=3`.
    SensitivityOutOfRange,
    /// `valid_from > valid_until` (violates the interval check).
    InvertedInterval,
    /// Empty `namespace` (violates NOT NULL/nonempty policy).
    EmptyNamespace,
    /// Unknown `truth_state` value.
    UnknownTruthState,
}

impl InvalidRowMutation {
    fn apply(&self, rec: &mut FixtureRecord) {
        match self {
            InvalidRowMutation::UnknownKind => rec.record_kind = "widget".to_string(),
            InvalidRowMutation::SensitivityOutOfRange => rec.policy.sensitivity = 4,
            InvalidRowMutation::InvertedInterval => {
                rec.valid_from = Some(T2.to_string());
                rec.valid_until = Some(T1.to_string());
            }
            InvalidRowMutation::EmptyNamespace => rec.policy.namespace = String::new(),
            InvalidRowMutation::UnknownTruthState => rec.truth_state = "Bogus".to_string(),
        }
    }
}

/// Build links covering every link kind plus planted invalid edges.
fn build_links(rng: &mut SplitMix64, records: &[FixtureRecord]) -> Vec<FixtureLink> {
    let mut links = Vec::new();

    // First valid record of each kind, for well-typed endpoints.
    let first_of = |kind: RecordKind| -> &FixtureRecord {
        records
            .iter()
            .find(|r| r.valid && r.record_kind == kind.code())
            .expect("coverage guarantees at least one valid record per kind")
    };

    // -- Valid links: one per canonical link kind ----------------------------
    // Endpoints chosen to match the semantic intent of each relation.
    let valid_edges: [(LinkKind, RecordKind, RecordKind); 5] = [
        (
            LinkKind::DerivedFrom,
            RecordKind::Summary,
            RecordKind::Memory,
        ),
        (
            LinkKind::Supports,
            RecordKind::Evidence,
            RecordKind::Relationship,
        ),
        (
            LinkKind::Contradicts,
            RecordKind::Evidence,
            RecordKind::Memory,
        ),
        (
            LinkKind::MentionsEntity,
            RecordKind::Mention,
            RecordKind::Entity,
        ),
        (
            LinkKind::SupersededBy,
            RecordKind::Memory,
            RecordKind::Summary,
        ),
    ];
    for (link, src_kind, tgt_kind) in valid_edges {
        let src = first_of(src_kind);
        let tgt = first_of(tgt_kind);
        links.push(FixtureLink {
            id: rng.next_uuid(),
            link_type: link.code().to_string(),
            source_kind: src.record_kind.clone(),
            source_id: src.id.clone(),
            target_kind: tgt.record_kind.clone(),
            target_id: tgt.id.clone(),
            truth_state: TruthState::Current.code().to_string(),
            valid: true,
            invalid_reason: None,
        });
    }

    // -- Invalid links (planted defects) -------------------------------------
    // 1. Unknown link type (no relation-registry row).
    {
        let src = first_of(RecordKind::Memory);
        let tgt = first_of(RecordKind::Entity);
        links.push(FixtureLink {
            id: rng.next_uuid(),
            link_type: "relates_to".to_string(),
            source_kind: src.record_kind.clone(),
            source_id: src.id.clone(),
            target_kind: tgt.record_kind.clone(),
            target_id: tgt.id.clone(),
            truth_state: TruthState::Current.code().to_string(),
            valid: false,
            invalid_reason: Some("unknown_link_type".to_string()),
        });
    }
    // 2. Dangling endpoint (target ID references no record).
    {
        let src = first_of(RecordKind::Evidence);
        links.push(FixtureLink {
            id: rng.next_uuid(),
            link_type: LinkKind::Supports.code().to_string(),
            source_kind: src.record_kind.clone(),
            source_id: src.id.clone(),
            target_kind: RecordKind::Relationship.code().to_string(),
            target_id: "00000000-0000-0000-0000-000000000000".to_string(),
            truth_state: TruthState::Current.code().to_string(),
            valid: false,
            invalid_reason: Some("dangling_endpoint".to_string()),
        });
    }
    // 3. Reflexive directed edge (source == target on a directed relation).
    {
        let node = first_of(RecordKind::Memory);
        debug_assert!(LinkKind::SupersededBy.is_directed());
        links.push(FixtureLink {
            id: rng.next_uuid(),
            link_type: LinkKind::SupersededBy.code().to_string(),
            source_kind: node.record_kind.clone(),
            source_id: node.id.clone(),
            target_kind: node.record_kind.clone(),
            target_id: node.id.clone(),
            truth_state: TruthState::Current.code().to_string(),
            valid: false,
            invalid_reason: Some("reflexive_directed".to_string()),
        });
    }

    links
}

/// Compute the count breakdowns for the manifest.
fn compute_counts(records: &[FixtureRecord], links: &[FixtureLink]) -> FixtureCounts {
    let mut counts = FixtureCounts {
        total_records: records.len(),
        total_links: links.len(),
        valid_records: 0,
        invalid_records: 0,
        valid_links: 0,
        invalid_links: 0,
        records_by_kind: Default::default(),
        records_by_truth_state: Default::default(),
        records_by_memory_mode: Default::default(),
        records_by_sensitivity: Default::default(),
        links_by_kind: Default::default(),
        idempotency_collisions: 0,
    };

    for r in records {
        if r.valid {
            counts.valid_records += 1;
            *counts
                .records_by_kind
                .entry(r.record_kind.clone())
                .or_insert(0) += 1;
            *counts
                .records_by_truth_state
                .entry(r.truth_state.clone())
                .or_insert(0) += 1;
            *counts
                .records_by_memory_mode
                .entry(r.memory_mode.clone())
                .or_insert(0) += 1;
            *counts
                .records_by_sensitivity
                .entry(r.policy.sensitivity.to_string())
                .or_insert(0) += 1;
        } else {
            counts.invalid_records += 1;
        }
    }
    for l in links {
        if l.valid {
            counts.valid_links += 1;
            *counts.links_by_kind.entry(l.link_type.clone()).or_insert(0) += 1;
        } else {
            counts.invalid_links += 1;
        }
    }
    counts.idempotency_collisions = collision_groups(records).len();
    counts
}

/// Group valid records that share `(caller_partition, idempotency_key)`.
fn collision_groups(records: &[FixtureRecord]) -> Vec<IdempotencyCollision> {
    use std::collections::BTreeMap;
    let mut by_key: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for r in records.iter().filter(|r| r.valid) {
        by_key
            .entry((r.caller_partition.clone(), r.idempotency_key.clone()))
            .or_default()
            .push(r.id.clone());
    }
    by_key
        .into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|((caller_partition, idempotency_key), mut record_ids)| {
            record_ids.sort();
            IdempotencyCollision {
                caller_partition,
                idempotency_key,
                record_ids,
            }
        })
        .collect()
}

/// Compute the independent expected-answer oracle for the manifest.
fn compute_expected(records: &[FixtureRecord], links: &[FixtureLink]) -> ExpectedAnswers {
    let mut valid_record_ids: Vec<String> = records
        .iter()
        .filter(|r| r.valid)
        .map(|r| r.id.clone())
        .collect();
    valid_record_ids.sort();

    let membership_hash = sha256_hex(valid_record_ids.join("\n").as_bytes());

    let invalid_records = records
        .iter()
        .filter(|r| !r.valid)
        .map(|r| InvalidCase {
            id: r.id.clone(),
            reason: r.invalid_reason.clone().unwrap_or_default(),
        })
        .collect();

    let invalid_links = links
        .iter()
        .filter(|l| !l.valid)
        .map(|l| InvalidCase {
            id: l.id.clone(),
            reason: l.invalid_reason.clone().unwrap_or_default(),
        })
        .collect();

    ExpectedAnswers {
        valid_record_ids,
        membership_hash,
        invalid_records,
        invalid_links,
        idempotency_collisions: collision_groups(records),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn pkg() -> FixturePackage {
        UnitV2Generator.generate()
    }

    fn records_of(pkg: &FixturePackage) -> Vec<FixtureRecord> {
        let (_, bytes) = pkg
            .data_files
            .iter()
            .find(|(n, _)| n == "records.json")
            .expect("records.json present");
        serde_json::from_slice(bytes).expect("records deserialize")
    }

    fn links_of(pkg: &FixturePackage) -> Vec<FixtureLink> {
        let (_, bytes) = pkg
            .data_files
            .iter()
            .find(|(n, _)| n == "links.json")
            .expect("links.json present");
        serde_json::from_slice(bytes).expect("links deserialize")
    }

    #[test]
    fn seed_and_id_match_validation_contract() {
        assert_eq!(SEED, 0x4D47_5201);
        assert_eq!(FIXTURE_ID, "mg-unit-v2");
        let m = pkg().manifest;
        assert_eq!(m.generator.seed, 0x4D47_5201);
        assert_eq!(m.generator.seed_hex, "0x4D475201");
        assert_eq!(m.fixture_id, "mg-unit-v2");
    }

    #[test]
    fn produces_exactly_one_hundred_records() {
        assert_eq!(records_of(&pkg()).len(), TOTAL_RECORDS);
        assert_eq!(pkg().manifest.counts.total_records, TOTAL_RECORDS);
    }

    #[test]
    fn two_run_byte_and_hash_determinism() {
        let a = pkg();
        let b = pkg();
        // Byte equality across every file, including the manifest.
        assert_eq!(a.all_files(), b.all_files(), "files must be byte-identical");
        // Package-level hash equality.
        assert_eq!(a.manifest.package_sha256, b.manifest.package_sha256);
        assert!(!a.manifest.package_sha256.is_empty());
    }

    #[test]
    fn every_record_kind_is_present() {
        let records = records_of(&pkg());
        let kinds: BTreeSet<&str> = records
            .iter()
            .filter(|r| r.valid)
            .map(|r| r.record_kind.as_str())
            .collect();
        for k in RecordKind::ALL {
            assert!(kinds.contains(k.code()), "missing record kind {}", k.code());
        }
    }

    #[test]
    fn every_link_kind_is_present() {
        let links = links_of(&pkg());
        let kinds: BTreeSet<&str> = links
            .iter()
            .filter(|l| l.valid)
            .map(|l| l.link_type.as_str())
            .collect();
        for k in LinkKind::ALL {
            assert!(kinds.contains(k.code()), "missing link kind {}", k.code());
        }
    }

    #[test]
    fn every_truth_state_and_memory_mode_present() {
        let records = records_of(&pkg());
        let truths: BTreeSet<&str> = records
            .iter()
            .filter(|r| r.valid)
            .map(|r| r.truth_state.as_str())
            .collect();
        for t in TruthState::ALL {
            assert!(
                truths.contains(t.code()),
                "missing truth state {}",
                t.code()
            );
        }
        let modes: BTreeSet<&str> = records
            .iter()
            .filter(|r| r.valid)
            .map(|r| r.memory_mode.as_str())
            .collect();
        for m in MemoryMode::ALL {
            assert!(modes.contains(m.code()), "missing memory mode {}", m.code());
        }
    }

    #[test]
    fn policy_lattice_covers_every_sensitivity() {
        let records = records_of(&pkg());
        let sens: BTreeSet<i64> = records
            .iter()
            .filter(|r| r.valid)
            .map(|r| r.policy.sensitivity)
            .collect();
        for level in 0..=3 {
            assert!(sens.contains(&level), "missing sensitivity {level}");
        }
    }

    #[test]
    fn invalid_rows_are_present_and_flagged() {
        let records = records_of(&pkg());
        let reasons: BTreeSet<String> = records
            .iter()
            .filter(|r| !r.valid)
            .map(|r| r.invalid_reason.clone().unwrap_or_default())
            .collect();
        for expected in [
            "unknown_record_kind",
            "sensitivity_out_of_range",
            "invalid_valid_interval",
            "empty_namespace",
            "unknown_truth_state",
        ] {
            assert!(
                reasons.contains(expected),
                "missing invalid case {expected}"
            );
        }
        // Every invalid record carries a reason; every valid record does not.
        for r in &records {
            assert_eq!(r.valid, r.invalid_reason.is_none());
        }
    }

    #[test]
    fn invalid_links_are_present_and_flagged() {
        let links = links_of(&pkg());
        let reasons: BTreeSet<String> = links
            .iter()
            .filter(|l| !l.valid)
            .map(|l| l.invalid_reason.clone().unwrap_or_default())
            .collect();
        for expected in [
            "unknown_link_type",
            "dangling_endpoint",
            "reflexive_directed",
        ] {
            assert!(
                reasons.contains(expected),
                "missing invalid link {expected}"
            );
        }
    }

    #[test]
    fn idempotency_collision_is_present() {
        let m = pkg().manifest;
        assert_eq!(m.counts.idempotency_collisions, 1);
        let collision = &m.expected.idempotency_collisions[0];
        assert_eq!(collision.record_ids.len(), 2);
        assert_eq!(collision.idempotency_key, "idem-collision-0001");

        // The two colliding records share identity but differ by command hash.
        let records = records_of(&pkg());
        let colliding: Vec<&FixtureRecord> = records
            .iter()
            .filter(|r| collision.record_ids.contains(&r.id))
            .collect();
        assert_eq!(colliding.len(), 2);
        assert_eq!(colliding[0].caller_partition, colliding[1].caller_partition);
        assert_eq!(colliding[0].idempotency_key, colliding[1].idempotency_key);
        assert_ne!(colliding[0].command_hash, colliding[1].command_hash);
    }

    #[test]
    fn manifest_metadata_is_valid() {
        let m = pkg().manifest;
        assert_eq!(m.schema_version, FIXTURE_MANIFEST_SCHEMA);
        assert_eq!(m.generator.version, GENERATOR_VERSION);
        assert_eq!(m.generator.algorithm, "splitmix64");
        assert_eq!(m.schema_versions.authority_schema, 2);
        assert!(!m.contains_private_data);
        // Two data files with checksums; manifest is not self-referential.
        assert_eq!(m.files.len(), 2);
        for f in &m.files {
            assert_eq!(f.sha256.len(), 64, "sha256 hex length");
            assert!(f.size > 0);
            assert_eq!(f.media_type, "application/json");
        }
    }

    #[test]
    fn manifest_file_checksums_match_data_bytes() {
        let p = pkg();
        for (name, bytes) in &p.data_files {
            let entry = p
                .manifest
                .files
                .iter()
                .find(|f| &f.path == name)
                .expect("file entry present");
            assert_eq!(entry.sha256, sha256_hex(bytes), "checksum for {name}");
            assert_eq!(entry.size, bytes.len(), "size for {name}");
        }
    }

    #[test]
    fn membership_hash_is_independent_and_stable() {
        let p = pkg();
        let records = records_of(&p);
        let mut ids: Vec<String> = records
            .iter()
            .filter(|r| r.valid)
            .map(|r| r.id.clone())
            .collect();
        ids.sort();
        assert_eq!(
            p.manifest.expected.membership_hash,
            sha256_hex(ids.join("\n").as_bytes())
        );
        assert_eq!(p.manifest.expected.valid_record_ids, ids);
    }

    #[test]
    fn manifest_roundtrips_through_json() {
        let m = pkg().manifest;
        let bytes = pkg().manifest_bytes();
        let parsed: FixtureManifest = serde_json::from_slice(&bytes).expect("manifest parses");
        assert_eq!(parsed, m);
    }

    #[test]
    fn no_record_content_contains_private_data_markers() {
        // Synthetic content only: all payloads use the fixed synthetic prefix.
        let records = records_of(&pkg());
        for r in &records {
            assert!(
                r.content.starts_with("synthetic unit-v2"),
                "unexpected content: {}",
                r.content
            );
        }
    }

    #[test]
    fn materializes_committed_package_to_repo() {
        // Materialize the frozen package into the committed fixtures tree.
        // Deterministic output keeps the working tree clean on re-runs.
        let root = super::super::generated_root();
        let dir = pkg().materialize(&root).expect("materialize package");
        assert!(dir.join("records.json").exists());
        assert!(dir.join("links.json").exists());
        assert!(dir.join("fixture-manifest.json").exists());

        // Re-materialize and confirm byte-identical manifest on disk.
        let manifest_bytes = std::fs::read(dir.join("fixture-manifest.json")).unwrap();
        assert_eq!(manifest_bytes, pkg().manifest_bytes());
    }
}
