//! `mg-interchange-v2` deterministic portable-interchange contract fixture
//! (task F0.2 / 0.2.6).
//!
//! Seed `0x4D475208`. Materializes a **self-describing, checksummed, secret-free
//! interchange package** plus a set of must-reject negative cases, modelling the
//! v2 interchange export/import contract (design.md §14, §8; `validation.md`
//! `mg-interchange-v2`, MGR-006/MGR-036).
//!
//! ## What the package proves
//!
//! * **Self-describing / versioned.** The package header pins schema, ontology,
//!   relation-registry, algorithm and model versions (design.md §14: "open
//!   canonical-JSON manifest … schema/ontology/relation/algorithm/model
//!   versions").
//! * **Checksummed.** Every record/event/link/provenance item carries a
//!   per-item SHA-256 `checksum`, and the package carries a `package_checksum`
//!   over the ordered per-item checksums.
//! * **Known required + known optional + unknown optional fields.** Each item
//!   has known required fields and a known optional (`note`) field. Unknown
//!   *optional* fields live under an `ext` object and MUST be preserved for
//!   re-export (design.md §14: "unknown optional fields are retained for
//!   re-export").
//! * **Unknown required field is rejected.** The strict item schema uses
//!   `deny_unknown_fields`, so an item carrying an unexpected top-level key
//!   fails to parse — modelling "unknown required semantics reject atomically".
//! * **No secrets.** No package byte matches any secret-like pattern
//!   (design.md §14: "no unauthorized secrets"; §8: "no secrets beyond
//!   authorization").
//! * **Empty-store round trip.** Export → import → re-export preserves semantic
//!   IDs, order, links, provenance, and state (design.md §14).
//!
//! All content is synthetic; the package contains no private data. Two runs at
//! the same [`GENERATOR_VERSION`] produce byte-identical files and hashes.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    package_files_and_hash, sha256_hex, ExpectedAnswers, FixtureCounts, FixtureGenerator,
    FixtureManifest, FixturePackage, GeneratorMetadata, InterchangeNegativeCase, InterchangeOracle,
    InvalidCase, LinkKind, RecordKind, RoundTripExpectation, SchemaVersions, SplitMix64,
    FIXTURE_MANIFEST_SCHEMA, GENERATOR_VERSION,
};

/// The frozen seed for `mg-interchange-v2` (`validation.md` §2).
pub const SEED: u64 = 0x4D47_5208;

/// The fixture identifier.
pub const FIXTURE_ID: &str = "mg-interchange-v2";

/// The interchange package schema identifier (design.md §14 "Interchange v1").
pub const INTERCHANGE_SCHEMA: &str = "memory-graph-interchange/v1";

/// The object key under which unknown *optional* extension fields are carried
/// and preserved for re-export.
pub const EXT_KEY: &str = "ext";

/// The number of records exported.
pub const RECORD_COUNT: usize = 10;

/// The number of events exported.
pub const EVENT_COUNT: usize = 6;

/// The number of provenance entries exported.
pub const PROVENANCE_COUNT: usize = 6;

/// Secret-like patterns asserted absent from every package byte (case-insensitive).
pub const SECRET_PATTERNS: [&str; 11] = [
    "password",
    "passwd",
    "api_key",
    "apikey",
    "access_key",
    "private key",
    "begin rsa",
    "authorization:",
    "bearer ",
    "aws_secret",
    "akia",
];

/// A fixed synthetic timestamp (no wall-clock is read).
const T_GENERATED: &str = "2024-06-01T00:00:00Z";

// ---------------------------------------------------------------------------
// Package item types (strict: deny_unknown_fields → unknown required rejects)
// ---------------------------------------------------------------------------

/// One exported record. `#[serde(deny_unknown_fields)]` makes an unexpected
/// top-level key (an unknown *required* field) fail to parse, while unknown
/// *optional* fields carried under `ext` are preserved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterchangeRecord {
    /// Canonical position in the package (order preservation).
    pub order: u32,
    /// Semantic record ID.
    pub id: String,
    /// Record kind code (required).
    pub record_kind: String,
    /// Truth-state code (required).
    pub truth_state: String,
    /// Memory-mode code (required).
    pub memory_mode: String,
    /// Explicit export scope (required).
    pub scope: String,
    /// Provenance entry ID (required).
    pub provenance_id: String,
    /// Human-facing label (required, synthetic).
    pub label: String,
    /// Known optional annotation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Unknown *optional* extension fields, preserved verbatim for re-export.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
    /// Per-item SHA-256 checksum over the canonical content.
    pub checksum: String,
}

/// One exported event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterchangeEvent {
    /// Canonical position in the package.
    pub order: u32,
    /// Event ID.
    pub id: String,
    /// Event kind (`start`/`completion`/`observation`).
    pub event_kind: String,
    /// Typed outcome code.
    pub outcome: String,
    /// Source kind.
    pub source_kind: String,
    /// Invocation ID binding the event to a call.
    pub invocation_id: String,
    /// Occurrence time (RFC3339 UTC, fixed).
    pub occurred_at: String,
    /// Policy hash bound to the event.
    pub policy_hash: String,
    /// Unknown *optional* extension fields, preserved verbatim.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
    /// Per-item SHA-256 checksum.
    pub checksum: String,
}

/// One exported link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterchangeLink {
    /// Canonical position in the package.
    pub order: u32,
    /// Link ID.
    pub id: String,
    /// Link kind code.
    pub link_kind: String,
    /// Source endpoint record ID.
    pub source_id: String,
    /// Target endpoint record ID.
    pub target_id: String,
    /// Truth-state code.
    pub truth_state: String,
    /// Unknown *optional* extension fields, preserved verbatim.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
    /// Per-item SHA-256 checksum.
    pub checksum: String,
}

/// One exported provenance entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterchangeProvenance {
    /// Canonical position in the package.
    pub order: u32,
    /// Provenance ID.
    pub id: String,
    /// Source kind.
    pub source_kind: String,
    /// Source ID.
    pub source_id: String,
    /// Source revision.
    pub source_revision: String,
    /// Derivation method.
    pub method: String,
    /// Confidence percent (`0..=100`, integer to keep the fixture exact).
    pub confidence_pct: u32,
    /// Unknown *optional* extension fields, preserved verbatim.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
    /// Per-item SHA-256 checksum.
    pub checksum: String,
}

/// The self-describing interchange package (design.md §14).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterchangePackage {
    /// Interchange schema identifier.
    pub schema_version: String,
    /// Ontology version.
    pub ontology_version: u32,
    /// Relation-registry version.
    pub relation_registry_version: u32,
    /// Algorithm version.
    pub algorithm_version: u32,
    /// Embedding/model identity.
    pub model_version: String,
    /// Explicit export scope.
    pub scope: String,
    /// Fixed generation time (no wall-clock read).
    pub generated_at: String,
    /// Ordered exported records.
    pub records: Vec<InterchangeRecord>,
    /// Ordered exported events.
    pub events: Vec<InterchangeEvent>,
    /// Ordered exported links.
    pub links: Vec<InterchangeLink>,
    /// Ordered exported provenance entries.
    pub provenance: Vec<InterchangeProvenance>,
    /// SHA-256 over the ordered per-item checksums.
    pub package_checksum: String,
}

/// One raw must-reject negative case carried in `negative-cases.json`. The
/// `raw` payload is deliberately ill-formed so import validation rejects it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NegativeCaseData {
    /// Stable case ID.
    pub case_id: String,
    /// Case kind (`unknown_required_field`/`checksum_mismatch`/`unknown_required_enum`).
    pub kind: String,
    /// The item type the raw payload targets (`record`).
    pub target_type: String,
    /// Machine-stable reason code the importer would emit.
    pub reason_code: String,
    /// The offending raw item.
    pub raw: Value,
}

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

/// The `mg-interchange-v2` generator.
#[derive(Debug, Default, Clone, Copy)]
pub struct InterchangeV2Generator;

impl FixtureGenerator for InterchangeV2Generator {
    fn fixture_id(&self) -> &'static str {
        FIXTURE_ID
    }

    fn seed(&self) -> u64 {
        SEED
    }

    fn generate(&self) -> FixturePackage {
        build()
    }
}

fn to_json_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    let mut s = serde_json::to_string_pretty(value).expect("serializes to JSON");
    s.push('\n');
    s.into_bytes()
}

/// Canonical serialization of an `ext` map (BTreeMap keeps keys sorted).
fn ext_canon(ext: &BTreeMap<String, Value>) -> String {
    serde_json::to_string(ext).expect("ext serializes")
}

fn record_checksum(r: &InterchangeRecord) -> String {
    let canon = format!(
        "record|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        r.order,
        r.id,
        r.record_kind,
        r.truth_state,
        r.memory_mode,
        r.scope,
        r.provenance_id,
        r.label,
        r.note.clone().unwrap_or_default(),
        ext_canon(&r.ext),
    );
    sha256_hex(canon.as_bytes())
}

fn event_checksum(e: &InterchangeEvent) -> String {
    let canon = format!(
        "event|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        e.order,
        e.id,
        e.event_kind,
        e.outcome,
        e.source_kind,
        e.invocation_id,
        e.occurred_at,
        e.policy_hash,
        ext_canon(&e.ext),
    );
    sha256_hex(canon.as_bytes())
}

fn link_checksum(l: &InterchangeLink) -> String {
    let canon = format!(
        "link|{}|{}|{}|{}|{}|{}|{}",
        l.order,
        l.id,
        l.link_kind,
        l.source_id,
        l.target_id,
        l.truth_state,
        ext_canon(&l.ext),
    );
    sha256_hex(canon.as_bytes())
}

fn provenance_checksum(p: &InterchangeProvenance) -> String {
    let canon = format!(
        "provenance|{}|{}|{}|{}|{}|{}|{}|{}",
        p.order,
        p.id,
        p.source_kind,
        p.source_id,
        p.source_revision,
        p.method,
        p.confidence_pct,
        ext_canon(&p.ext),
    );
    sha256_hex(canon.as_bytes())
}

// ---------------------------------------------------------------------------
// Fixed synthetic constants (no secrets)
// ---------------------------------------------------------------------------

const SCOPE: &str = "namespace:personal;scope:private";
const SOURCE_KINDS: [&str; 3] = ["filesystem", "repository", "manual_entry"];
const OUTCOMES: [&str; 3] = ["success", "partial", "correction"];
const EVENT_KINDS: [&str; 3] = ["start", "completion", "observation"];

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn build_provenance(rng: &mut SplitMix64) -> Vec<InterchangeProvenance> {
    let mut out = Vec::with_capacity(PROVENANCE_COUNT);
    for i in 0..PROVENANCE_COUNT {
        let mut ext = BTreeMap::new();
        // Every third provenance carries an unknown-optional field.
        if i % 3 == 0 {
            ext.insert(
                "x_ingest_batch".to_string(),
                Value::from(format!("batch-{i:03}")),
            );
        }
        let mut p = InterchangeProvenance {
            order: i as u32,
            id: rng.next_uuid(),
            source_kind: SOURCE_KINDS[i % SOURCE_KINDS.len()].to_string(),
            source_id: format!("src-{i:03}"),
            source_revision: format!("rev-{}", i + 1),
            method: "deterministic_import".to_string(),
            confidence_pct: 70 + (i as u32 % 30),
            ext,
            checksum: String::new(),
        };
        p.checksum = provenance_checksum(&p);
        out.push(p);
    }
    out
}

fn build_records(
    rng: &mut SplitMix64,
    provenance: &[InterchangeProvenance],
) -> Vec<InterchangeRecord> {
    // Record kinds that are valid `records`-table kinds for the interchange.
    let kinds = [
        RecordKind::Memory,
        RecordKind::Summary,
        RecordKind::Skill,
        RecordKind::Rule,
    ];
    let truths = ["Current", "Confirmed", "Inferred", "Superseded"];
    let modes = ["Permanent", "Temporary", "Session_Only"];
    let mut out = Vec::with_capacity(RECORD_COUNT);
    for i in 0..RECORD_COUNT {
        let mut ext = BTreeMap::new();
        // Plant unknown-optional fields on a deterministic subset.
        if i % 2 == 0 {
            ext.insert("x_vendor_tag".to_string(), Value::from("kria"));
        }
        if i % 3 == 0 {
            ext.insert("x_render_hint".to_string(), Value::from("pinned"));
        }
        if i % 5 == 0 {
            ext.insert("x_experimental_weight".to_string(), Value::from(i as u64));
        }
        let note = (i % 4 == 0).then(|| format!("known optional note {i}"));
        let mut r = InterchangeRecord {
            order: i as u32,
            id: rng.next_uuid(),
            record_kind: kinds[i % kinds.len()].code().to_string(),
            truth_state: truths[i % truths.len()].to_string(),
            memory_mode: modes[i % modes.len()].to_string(),
            scope: SCOPE.to_string(),
            provenance_id: provenance[i % provenance.len()].id.clone(),
            label: format!("interchange record {i}"),
            note,
            ext,
            checksum: String::new(),
        };
        r.checksum = record_checksum(&r);
        out.push(r);
    }
    out
}

fn build_events(rng: &mut SplitMix64) -> Vec<InterchangeEvent> {
    let mut out = Vec::with_capacity(EVENT_COUNT);
    for i in 0..EVENT_COUNT {
        let mut ext = BTreeMap::new();
        if i % 3 == 1 {
            ext.insert("x_trace_span".to_string(), Value::from(format!("span-{i}")));
        }
        let mut e = InterchangeEvent {
            order: i as u32,
            id: rng.next_uuid(),
            event_kind: EVENT_KINDS[i % EVENT_KINDS.len()].to_string(),
            outcome: OUTCOMES[i % OUTCOMES.len()].to_string(),
            source_kind: SOURCE_KINDS[i % SOURCE_KINDS.len()].to_string(),
            invocation_id: format!("inv-{i:03}"),
            occurred_at: T_GENERATED.to_string(),
            policy_hash: sha256_hex(format!("policy-{i}").as_bytes()),
            ext,
            checksum: String::new(),
        };
        e.checksum = event_checksum(&e);
        out.push(e);
    }
    out
}

fn build_links(rng: &mut SplitMix64, records: &[InterchangeRecord]) -> Vec<InterchangeLink> {
    let mut out = Vec::with_capacity(LinkKind::ALL.len());
    for (i, kind) in LinkKind::ALL.iter().enumerate() {
        let mut ext = BTreeMap::new();
        if i % 2 == 0 {
            ext.insert("x_edge_weight".to_string(), Value::from((i as u64) + 1));
        }
        let src = &records[i * 2 % records.len()];
        let tgt = &records[(i * 2 + 1) % records.len()];
        let mut l = InterchangeLink {
            order: i as u32,
            id: rng.next_uuid(),
            link_kind: kind.code().to_string(),
            source_id: src.id.clone(),
            target_id: tgt.id.clone(),
            truth_state: "Current".to_string(),
            ext,
            checksum: String::new(),
        };
        l.checksum = link_checksum(&l);
        out.push(l);
    }
    out
}

/// Compute the package checksum over the ordered per-item checksums, and the
/// canonical ordered item-ID list.
fn package_checksum_and_order(pkg: &InterchangePackage) -> (String, Vec<String>) {
    let mut acc = String::new();
    let mut ids = Vec::new();
    for r in &pkg.records {
        acc.push_str(&r.checksum);
        acc.push('\n');
        ids.push(r.id.clone());
    }
    for e in &pkg.events {
        acc.push_str(&e.checksum);
        acc.push('\n');
        ids.push(e.id.clone());
    }
    for l in &pkg.links {
        acc.push_str(&l.checksum);
        acc.push('\n');
        ids.push(l.id.clone());
    }
    for p in &pkg.provenance {
        acc.push_str(&p.checksum);
        acc.push('\n');
        ids.push(p.id.clone());
    }
    (sha256_hex(acc.as_bytes()), ids)
}

/// Build the three deterministic must-reject negative cases.
fn build_negative_cases(records: &[InterchangeRecord]) -> Vec<NegativeCaseData> {
    // Base a well-formed record we then mutate into each defect.
    let base = &records[0];
    let mut cases = Vec::new();

    // 1) Unknown REQUIRED field: an unexpected top-level key. Because the item
    //    schema is deny_unknown_fields, strict parsing rejects it atomically.
    let mut v1 = serde_json::to_value(base).expect("record to value");
    v1["required_unsupported_semantic"] = Value::from("must_reject");
    cases.push(NegativeCaseData {
        case_id: "neg-unknown-required-field".to_string(),
        kind: "unknown_required_field".to_string(),
        target_type: "record".to_string(),
        reason_code: "UnsupportedSchema".to_string(),
        raw: v1,
    });

    // 2) Checksum mismatch: a well-formed record whose stored checksum is wrong.
    let mut v2 = serde_json::to_value(base).expect("record to value");
    v2["checksum"] = Value::from("0".repeat(64));
    cases.push(NegativeCaseData {
        case_id: "neg-checksum-mismatch".to_string(),
        kind: "checksum_mismatch".to_string(),
        target_type: "record".to_string(),
        reason_code: "ChecksumMismatch".to_string(),
        raw: v2,
    });

    // 3) Unknown REQUIRED enum/version: an unrecognized record_kind value.
    let mut v3 = serde_json::to_value(base).expect("record to value");
    v3["record_kind"] = Value::from("quantum_glyph");
    cases.push(NegativeCaseData {
        case_id: "neg-unknown-required-enum".to_string(),
        kind: "unknown_required_enum".to_string(),
        target_type: "record".to_string(),
        reason_code: "UnsupportedSchema".to_string(),
        raw: v3,
    });

    cases
}

// ---------------------------------------------------------------------------
// Oracle / counts / expected answers
// ---------------------------------------------------------------------------

fn collect_unknown_optional_fields(pkg: &InterchangePackage) -> (Vec<String>, usize) {
    let mut keys: BTreeSet<String> = BTreeSet::new();
    let mut records_with = 0usize;
    for r in &pkg.records {
        if !r.ext.is_empty() {
            records_with += 1;
        }
        keys.extend(r.ext.keys().cloned());
    }
    for e in &pkg.events {
        keys.extend(e.ext.keys().cloned());
    }
    for l in &pkg.links {
        keys.extend(l.ext.keys().cloned());
    }
    for p in &pkg.provenance {
        keys.extend(p.ext.keys().cloned());
    }
    (keys.into_iter().collect(), records_with)
}

fn build_oracle(
    pkg: &InterchangePackage,
    ordered_item_ids: &[String],
    negatives: &[NegativeCaseData],
) -> InterchangeOracle {
    let (unknown_optional_fields, records_with_unknown_optional) =
        collect_unknown_optional_fields(pkg);
    let negative_cases = negatives
        .iter()
        .map(|n| InterchangeNegativeCase {
            case_id: n.case_id.clone(),
            kind: n.kind.clone(),
            description: match n.kind.as_str() {
                "unknown_required_field" => {
                    "unexpected top-level field; strict schema rejects atomically".to_string()
                }
                "checksum_mismatch" => {
                    "stored checksum does not match recomputed item checksum".to_string()
                }
                _ => "unrecognized required enum/version; UnsupportedSchema".to_string(),
            },
            reason_code: n.reason_code.clone(),
            expected_disposition: "reject_atomic".to_string(),
        })
        .collect();

    InterchangeOracle {
        interchange_schema: INTERCHANGE_SCHEMA.to_string(),
        ontology_version: 1,
        relation_registry_version: 1,
        algorithm_version: 1,
        model_version: "all-MiniLM-L6-v2".to_string(),
        scope: SCOPE.to_string(),
        package_checksum: pkg.package_checksum.clone(),
        total_records: pkg.records.len(),
        total_events: pkg.events.len(),
        total_links: pkg.links.len(),
        total_provenance: pkg.provenance.len(),
        ordered_item_ids: ordered_item_ids.to_vec(),
        optional_extension_key: EXT_KEY.to_string(),
        known_optional_fields: vec!["note".to_string()],
        unknown_optional_fields,
        records_with_unknown_optional,
        negative_cases,
        no_secrets: true,
        secret_scan_patterns: SECRET_PATTERNS.iter().map(|s| s.to_string()).collect(),
        empty_store_round_trip: RoundTripExpectation {
            preserves_ids: true,
            preserves_order: true,
            preserves_links: true,
            preserves_provenance: true,
            preserves_state: true,
            reexport_matches_import: true,
        },
    }
}

fn compute_counts(pkg: &InterchangePackage, negatives: usize) -> FixtureCounts {
    let mut records_by_kind = BTreeMap::new();
    let mut records_by_truth_state = BTreeMap::new();
    let mut records_by_memory_mode = BTreeMap::new();
    let mut records_by_sensitivity = BTreeMap::new();
    for r in &pkg.records {
        *records_by_kind.entry(r.record_kind.clone()).or_insert(0) += 1;
        *records_by_truth_state
            .entry(r.truth_state.clone())
            .or_insert(0) += 1;
        *records_by_memory_mode
            .entry(r.memory_mode.clone())
            .or_insert(0) += 1;
        // Scope is fixed private → sensitivity 1 for the whole package.
        *records_by_sensitivity.entry("1".to_string()).or_insert(0) += 1;
    }
    let mut links_by_kind = BTreeMap::new();
    for l in &pkg.links {
        *links_by_kind.entry(l.link_kind.clone()).or_insert(0) += 1;
    }
    FixtureCounts {
        total_records: pkg.records.len() + negatives,
        total_links: pkg.links.len(),
        valid_records: pkg.records.len(),
        invalid_records: negatives,
        valid_links: pkg.links.len(),
        invalid_links: 0,
        records_by_kind,
        records_by_truth_state,
        records_by_memory_mode,
        records_by_sensitivity,
        links_by_kind,
        idempotency_collisions: 0,
    }
}

fn compute_expected(pkg: &InterchangePackage, negatives: &[NegativeCaseData]) -> ExpectedAnswers {
    let mut valid_record_ids: Vec<String> = pkg.records.iter().map(|r| r.id.clone()).collect();
    valid_record_ids.sort();
    let membership_hash = sha256_hex(valid_record_ids.join("\n").as_bytes());
    let invalid_records = negatives
        .iter()
        .map(|n| InvalidCase {
            id: n.case_id.clone(),
            reason: n.reason_code.clone(),
        })
        .collect();
    ExpectedAnswers {
        valid_record_ids,
        membership_hash,
        invalid_records,
        invalid_links: Vec::new(),
        idempotency_collisions: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Top-level build
// ---------------------------------------------------------------------------

/// Deterministically build the in-memory `mg-interchange-v2` package.
pub fn build() -> FixturePackage {
    let mut rng = SplitMix64::new(SEED);

    let provenance = build_provenance(&mut rng);
    let records = build_records(&mut rng, &provenance);
    let events = build_events(&mut rng);
    let links = build_links(&mut rng, &records);

    let mut package = InterchangePackage {
        schema_version: INTERCHANGE_SCHEMA.to_string(),
        ontology_version: 1,
        relation_registry_version: 1,
        algorithm_version: 1,
        model_version: "all-MiniLM-L6-v2".to_string(),
        scope: SCOPE.to_string(),
        generated_at: T_GENERATED.to_string(),
        records,
        events,
        links,
        provenance,
        package_checksum: String::new(),
    };
    let (checksum, ordered_item_ids) = package_checksum_and_order(&package);
    package.package_checksum = checksum;

    let negatives = build_negative_cases(&package.records);

    let data_files = vec![
        (
            "interchange-package.json".to_string(),
            to_json_bytes(&package),
        ),
        ("negative-cases.json".to_string(), to_json_bytes(&negatives)),
    ];
    let (files, package_sha256) = package_files_and_hash(&data_files);
    let counts = compute_counts(&package, negatives.len());
    let expected = compute_expected(&package, &negatives);
    let oracle = build_oracle(&package, &ordered_item_ids, &negatives);

    let manifest = FixtureManifest {
        schema_version: FIXTURE_MANIFEST_SCHEMA.to_string(),
        fixture_id: FIXTURE_ID.to_string(),
        generator: GeneratorMetadata {
            name: "memory_graph::fixtures::interchange_v2".to_string(),
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
        interchange_oracle: Some(oracle),
        visual_scene_oracle: None,
    };

    FixturePackage {
        fixture_id: FIXTURE_ID.to_string(),
        data_files,
        manifest,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg() -> FixturePackage {
        build()
    }

    fn package_of(p: &FixturePackage) -> InterchangePackage {
        let (_, bytes) = p
            .data_files
            .iter()
            .find(|(n, _)| n == "interchange-package.json")
            .expect("interchange-package.json present");
        serde_json::from_slice(bytes).expect("package deserializes")
    }

    fn negatives_of(p: &FixturePackage) -> Vec<NegativeCaseData> {
        let (_, bytes) = p
            .data_files
            .iter()
            .find(|(n, _)| n == "negative-cases.json")
            .expect("negative-cases.json present");
        serde_json::from_slice(bytes).expect("negatives deserialize")
    }

    fn oracle_of(p: &FixturePackage) -> InterchangeOracle {
        p.manifest
            .interchange_oracle
            .clone()
            .expect("interchange oracle present")
    }

    #[test]
    fn seed_and_id_match_validation_contract() {
        assert_eq!(SEED, 0x4D47_5208);
        assert_eq!(FIXTURE_ID, "mg-interchange-v2");
        let m = InterchangeV2Generator.generate().manifest;
        assert_eq!(m.generator.seed, 0x4D47_5208);
        assert_eq!(m.generator.seed_hex, "0x4D475208");
        assert_eq!(m.fixture_id, "mg-interchange-v2");
        assert_eq!(m.generator.algorithm, "splitmix64");
    }

    #[test]
    fn two_run_byte_and_hash_determinism() {
        let a = pkg();
        let b = pkg();
        assert_eq!(a.all_files(), b.all_files(), "files must be byte-identical");
        assert_eq!(a.manifest.package_sha256, b.manifest.package_sha256);
        assert!(!a.manifest.package_sha256.is_empty());
    }

    #[test]
    fn package_is_self_describing_and_versioned() {
        let p = package_of(&pkg());
        assert_eq!(p.schema_version, INTERCHANGE_SCHEMA);
        assert_eq!(p.ontology_version, 1);
        assert_eq!(p.relation_registry_version, 1);
        assert_eq!(p.algorithm_version, 1);
        assert!(!p.model_version.is_empty());
        assert_eq!(p.scope, SCOPE);
        assert_eq!(p.records.len(), RECORD_COUNT);
        assert_eq!(p.events.len(), EVENT_COUNT);
        assert_eq!(p.links.len(), LinkKind::ALL.len());
        assert_eq!(p.provenance.len(), PROVENANCE_COUNT);
    }

    #[test]
    fn every_item_carries_a_valid_checksum() {
        let p = package_of(&pkg());
        for r in &p.records {
            assert_eq!(r.checksum.len(), 64);
            assert_eq!(r.checksum, record_checksum(r), "record {} checksum", r.id);
        }
        for e in &p.events {
            assert_eq!(e.checksum, event_checksum(e), "event {} checksum", e.id);
        }
        for l in &p.links {
            assert_eq!(l.checksum, link_checksum(l), "link {} checksum", l.id);
        }
        for pv in &p.provenance {
            assert_eq!(
                pv.checksum,
                provenance_checksum(pv),
                "prov {} checksum",
                pv.id
            );
        }
        // Package checksum matches the ordered per-item checksums.
        let (expected, _) = package_checksum_and_order(&p);
        assert_eq!(p.package_checksum, expected);
        assert_eq!(oracle_of(&pkg()).package_checksum, p.package_checksum);
    }

    #[test]
    fn unknown_optional_fields_are_present_and_preserved() {
        let p = package_of(&pkg());
        // At least some records carry unknown-optional `ext` fields.
        let with_ext = p.records.iter().filter(|r| !r.ext.is_empty()).count();
        assert!(with_ext > 0, "expected unknown-optional fields on records");

        // Round-trip: serialize the whole package and parse it back; the ext
        // maps must survive verbatim (preserved for re-export).
        let bytes = to_json_bytes(&p);
        let reparsed: InterchangePackage =
            serde_json::from_slice(&bytes).expect("package re-parses");
        assert_eq!(reparsed, p, "unknown-optional fields must be preserved");
        assert!(reparsed
            .records
            .iter()
            .any(|r| r.ext.contains_key("x_vendor_tag")));

        // Oracle records the observed unknown-optional keys.
        let o = oracle_of(&pkg());
        assert!(o
            .unknown_optional_fields
            .contains(&"x_vendor_tag".to_string()));
        assert_eq!(o.optional_extension_key, "ext");
        assert!(o.known_optional_fields.contains(&"note".to_string()));
        assert_eq!(o.records_with_unknown_optional, with_ext);
    }

    #[test]
    fn unknown_required_field_is_rejected() {
        let negs = negatives_of(&pkg());
        let case = negs
            .iter()
            .find(|n| n.kind == "unknown_required_field")
            .expect("unknown_required_field case present");
        // Strict parsing of the raw item must FAIL (deny_unknown_fields).
        let parsed: Result<InterchangeRecord, _> = serde_json::from_value(case.raw.clone());
        assert!(
            parsed.is_err(),
            "unknown required field must be rejected atomically"
        );

        // A valid record (unknown-*optional* only) still parses fine.
        let p = package_of(&pkg());
        let good = serde_json::to_value(&p.records[0]).unwrap();
        assert!(serde_json::from_value::<InterchangeRecord>(good).is_ok());
    }

    #[test]
    fn checksum_mismatch_case_is_detectable() {
        let negs = negatives_of(&pkg());
        let case = negs
            .iter()
            .find(|n| n.kind == "checksum_mismatch")
            .expect("checksum_mismatch case present");
        // The raw item parses (well-formed) but its checksum is wrong.
        let rec: InterchangeRecord =
            serde_json::from_value(case.raw.clone()).expect("well-formed record");
        assert_ne!(
            rec.checksum,
            record_checksum(&rec),
            "checksum mismatch must be detectable"
        );
    }

    #[test]
    fn unknown_required_enum_case_is_unrecognized() {
        let negs = negatives_of(&pkg());
        let case = negs
            .iter()
            .find(|n| n.kind == "unknown_required_enum")
            .expect("unknown_required_enum case present");
        let rec: InterchangeRecord =
            serde_json::from_value(case.raw.clone()).expect("well-formed record");
        let known: BTreeSet<&str> = RecordKind::ALL.iter().map(|k| k.code()).collect();
        assert!(
            !known.contains(rec.record_kind.as_str()),
            "record_kind must be an unrecognized enum value"
        );
    }

    #[test]
    fn negative_cases_all_expect_atomic_rejection() {
        let o = oracle_of(&pkg());
        assert_eq!(o.negative_cases.len(), 3);
        for c in &o.negative_cases {
            assert_eq!(c.expected_disposition, "reject_atomic");
            assert!(!c.reason_code.is_empty());
        }
        // The negative cases appear as invalid_records in the expected answers.
        let ids: BTreeSet<String> = pkg()
            .manifest
            .expected
            .invalid_records
            .iter()
            .map(|c| c.id.clone())
            .collect();
        for c in &o.negative_cases {
            assert!(
                ids.contains(&c.case_id),
                "negative case must be an invalid record"
            );
        }
    }

    #[test]
    fn package_contains_no_secrets() {
        let o = oracle_of(&pkg());
        assert!(o.no_secrets);
        // Scan every DATA file byte (not the manifest, which names the patterns).
        for (name, bytes) in &pkg().data_files {
            let text = String::from_utf8_lossy(bytes).to_lowercase();
            for pat in SECRET_PATTERNS {
                assert!(
                    !text.contains(pat),
                    "secret-like pattern {pat:?} found in {name}"
                );
            }
        }
        assert!(!pkg().manifest.contains_private_data);
    }

    #[test]
    fn empty_store_round_trip_preserves_ids_order_links_provenance_state() {
        let p = package_of(&pkg());

        // Export → import → re-export: parse then re-serialize; must be identical.
        let bytes = to_json_bytes(&p);
        let imported: InterchangePackage = serde_json::from_slice(&bytes).expect("import");
        let reexported = to_json_bytes(&imported);
        assert_eq!(bytes, reexported, "re-export must be byte-identical");

        // IDs & order preserved.
        let (_, order_a) = package_checksum_and_order(&p);
        let (_, order_b) = package_checksum_and_order(&imported);
        assert_eq!(order_a, order_b, "item order preserved");

        // Links & provenance references still resolve.
        let record_ids: BTreeSet<&str> = imported.records.iter().map(|r| r.id.as_str()).collect();
        for l in &imported.links {
            assert!(record_ids.contains(l.source_id.as_str()));
            assert!(record_ids.contains(l.target_id.as_str()));
        }
        let prov_ids: BTreeSet<&str> = imported.provenance.iter().map(|p| p.id.as_str()).collect();
        for r in &imported.records {
            assert!(
                prov_ids.contains(r.provenance_id.as_str()),
                "provenance preserved"
            );
        }

        // State (truth/mode) preserved.
        for (a, b) in p.records.iter().zip(imported.records.iter()) {
            assert_eq!(a.truth_state, b.truth_state);
            assert_eq!(a.memory_mode, b.memory_mode);
        }

        // Oracle expectation matches.
        let rt = oracle_of(&pkg()).empty_store_round_trip;
        assert!(rt.preserves_ids && rt.preserves_order && rt.preserves_links);
        assert!(rt.preserves_provenance && rt.preserves_state && rt.reexport_matches_import);
    }

    #[test]
    fn ordered_item_ids_cover_all_items() {
        let o = oracle_of(&pkg());
        let expected = o.total_records + o.total_events + o.total_links + o.total_provenance;
        assert_eq!(o.ordered_item_ids.len(), expected);
        // No duplicate IDs across the package.
        let unique: BTreeSet<&String> = o.ordered_item_ids.iter().collect();
        assert_eq!(unique.len(), o.ordered_item_ids.len());
    }

    #[test]
    fn membership_hash_is_independent_and_stable() {
        let p = pkg();
        let package = package_of(&p);
        let mut ids: Vec<String> = package.records.iter().map(|r| r.id.clone()).collect();
        ids.sort();
        assert_eq!(
            p.manifest.expected.membership_hash,
            sha256_hex(ids.join("\n").as_bytes())
        );
        assert_eq!(p.manifest.expected.valid_record_ids, ids);
        // Stable across rebuilds.
        assert_eq!(
            pkg().manifest.expected.membership_hash,
            p.manifest.expected.membership_hash
        );
    }

    #[test]
    fn manifest_metadata_is_valid_and_roundtrips() {
        let p = pkg();
        let m = &p.manifest;
        assert_eq!(m.schema_version, FIXTURE_MANIFEST_SCHEMA);
        assert_eq!(m.generator.version, GENERATOR_VERSION);
        assert_eq!(m.schema_versions.authority_schema, 2);
        assert!(!m.contains_private_data);
        assert!(m.scene_coverage.is_none());
        assert!(m.release_oracle.is_none());
        assert!(m.paired_world_oracle.is_none());
        assert!(m.vector_oracle.is_none());
        assert!(m.judged_corpus_oracle.is_none());
        assert!(m.interchange_oracle.is_some());
        assert!(m.visual_scene_oracle.is_none());
        assert_eq!(m.files.len(), p.data_files.len());
        for (name, bytes) in &p.data_files {
            let entry = m.files.iter().find(|f| &f.path == name).expect("entry");
            assert_eq!(entry.sha256, sha256_hex(bytes), "checksum for {name}");
            assert_eq!(entry.size, bytes.len());
            assert_eq!(entry.media_type, "application/json");
        }
        let parsed: FixtureManifest =
            serde_json::from_slice(&p.manifest_bytes()).expect("manifest parses");
        assert_eq!(parsed, *m);
    }

    #[test]
    fn materializes_committed_package_to_repo() {
        let root = super::super::generated_root();
        let dir = pkg().materialize(&root).expect("materialize package");
        for f in [
            "interchange-package.json",
            "negative-cases.json",
            "fixture-manifest.json",
        ] {
            assert!(dir.join(f).exists(), "missing {f}");
        }
        let on_disk = std::fs::read(dir.join("fixture-manifest.json")).unwrap();
        assert_eq!(on_disk, pkg().manifest_bytes());
    }
}
