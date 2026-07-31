//! `mg-policy-pairs-v2` deterministic paired-world oracle (task F0.2 / 0.2.4).
//!
//! Seed `0x4D475205`. Materializes **two policy worlds that differ ONLY by
//! protected content** so non-interference tests (MGR-004, design.md §5,
//! `validation.md` V-POLICY-02) can assert **zero protected-data leak**:
//!
//! * **World A** contains only the shared, low-sensitivity records/edges.
//! * **World B** contains everything in World A **plus** protected content that
//!   is unauthorized for the low-privilege ("observer") caller.
//!
//! The two worlds must be **observationally indistinguishable** to the observer
//! across every leakage channel — hidden **labels**, **IDs**, **counts**,
//! **topology**, and **timing** — while an **authorized** caller sees the
//! protected content in World B where explicitly authorized.
//!
//! The oracle records, for each caller/world, the exact observable projection
//! and, per channel, the authorized-vs-hidden answer. Every value is defined by
//! the generator — the independent oracle — never derived from a system under
//! test. All content is synthetic (no real private data).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{
    package_files_and_hash, sha256_hex, ExpectedAnswers, FixtureCounts, FixtureGenerator,
    FixtureManifest, FixturePackage, GeneratorMetadata, MemoryMode, ObservableProjection,
    PairedChannelCase, PairedWorldOracle, Policy, RecordKind, SchemaVersions, SplitMix64,
    TruthState, FIXTURE_MANIFEST_SCHEMA, GENERATOR_VERSION,
};

/// The frozen seed for `mg-policy-pairs-v2` (`validation.md` §2).
pub const SEED: u64 = 0x4D47_5205;

/// The fixture identifier.
pub const FIXTURE_ID: &str = "mg-policy-pairs-v2";

/// Maximum sensitivity the unauthorized ("observer") caller may read.
pub const OBSERVER_MAX_SENSITIVITY: i64 = 0;

/// Maximum sensitivity the authorized caller may read.
pub const AUTHORIZED_MAX_SENSITIVITY: i64 = 3;

/// The two world identifiers.
const WORLD_A: &str = "a";
const WORLD_B: &str = "b";

/// Fixed synthetic timestamps (RFC3339 UTC).
const T_FROM: &str = "2024-01-01T00:00:00Z";
const T_UNTIL: &str = "2025-01-01T00:00:00Z";
/// A distinct valid-from used by the protected timing record (a hidden timing
/// signal that must not surface to the observer).
const T_PROTECTED_FROM: &str = "2024-03-15T04:05:06Z";

// ---------------------------------------------------------------------------
// Planted row types
// ---------------------------------------------------------------------------

/// One planted authority record in a policy world.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedRecord {
    /// Canonical record ID.
    pub id: String,
    /// Record kind code.
    pub record_kind: String,
    /// Truth state code.
    pub truth_state: String,
    /// Memory mode code.
    pub memory_mode: String,
    /// Effective policy tuple.
    pub policy: Policy,
    /// Sensitivity layer: `shared` (authorized to observer) or `protected`.
    pub layer: String,
    /// Worlds this record is present in (`["a","b"]` for shared, `["b"]` for
    /// protected — the sole difference between the two worlds).
    pub worlds: Vec<String>,
    /// Human-facing label (synthetic).
    pub label: String,
    /// Whether the low-privilege observer is authorized to read this record.
    pub authorized_for_observer: bool,
    /// Half-open Valid-Time start (RFC3339 UTC).
    pub valid_from: Option<String>,
    /// Half-open Valid-Time end (RFC3339 UTC).
    pub valid_until: Option<String>,
    /// Synthetic content payload.
    pub content: String,
    /// SHA-256 of `content`.
    pub content_hash: String,
}

/// One planted directed edge in a policy world.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedLink {
    /// Canonical link ID.
    pub id: String,
    /// Link kind code.
    pub link_type: String,
    /// Source endpoint record ID.
    pub source_id: String,
    /// Target endpoint record ID.
    pub target_id: String,
    /// Sensitivity layer: `shared` or `protected`.
    pub layer: String,
    /// Worlds this edge is present in.
    pub worlds: Vec<String>,
    /// Truth state code.
    pub truth_state: String,
}

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

/// The `mg-policy-pairs-v2` generator.
#[derive(Debug, Default, Clone, Copy)]
pub struct PolicyPairsV2Generator;

impl FixtureGenerator for PolicyPairsV2Generator {
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

/// Serialize a value to canonical pretty JSON bytes with a trailing newline.
fn to_json_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    let mut s = serde_json::to_string_pretty(value).expect("serializes to JSON");
    s.push('\n');
    s.into_bytes()
}

const NAMESPACES: [&str; 3] = ["personal", "work", "shared"];
const SCOPES: [&str; 3] = ["private", "team", "public"];
const OWNERS: [&str; 2] = ["owner-alpha", "owner-beta"];

/// Number of shared (observer-authorized) records planted in both worlds.
const SHARED_RECORDS: usize = 6;

/// Accumulates the paired-world records/links while drawing IDs from the PRNG.
struct Builder {
    rng: SplitMix64,
    records: Vec<PairedRecord>,
    links: Vec<PairedLink>,
}

impl Builder {
    fn new(seed: u64) -> Self {
        Self {
            rng: SplitMix64::new(seed),
            records: Vec::new(),
            links: Vec::new(),
        }
    }

    fn policy(&mut self, sensitivity: i64) -> Policy {
        Policy {
            namespace: NAMESPACES[self.rng.below(NAMESPACES.len())].to_string(),
            owner: OWNERS[self.rng.below(OWNERS.len())].to_string(),
            scope: SCOPES[self.rng.below(SCOPES.len())].to_string(),
            sensitivity,
        }
    }

    /// Push a record; returns its ID.
    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        kind: RecordKind,
        layer: &str,
        worlds: &[&str],
        sensitivity: i64,
        label: String,
        valid_from: &str,
    ) -> String {
        let id = self.rng.next_uuid();
        let policy = self.policy(sensitivity);
        let content = format!("synthetic mg-policy-pairs-v2 {layer} record :: {label}");
        let authorized_for_observer = sensitivity <= OBSERVER_MAX_SENSITIVITY;
        self.records.push(PairedRecord {
            id: id.clone(),
            record_kind: kind.code().to_string(),
            truth_state: TruthState::Current.code().to_string(),
            memory_mode: MemoryMode::Permanent.code().to_string(),
            policy,
            layer: layer.to_string(),
            worlds: worlds.iter().map(|w| w.to_string()).collect(),
            label,
            authorized_for_observer,
            valid_from: Some(valid_from.to_string()),
            valid_until: Some(T_UNTIL.to_string()),
            content_hash: sha256_hex(content.as_bytes()),
            content,
        });
        id
    }

    fn edge(&mut self, src: &str, tgt: &str, layer: &str, worlds: &[&str]) {
        let id = self.rng.next_uuid();
        self.links.push(PairedLink {
            id,
            link_type: super::LinkKind::DerivedFrom.code().to_string(),
            source_id: src.to_string(),
            target_id: tgt.to_string(),
            layer: layer.to_string(),
            worlds: worlds.iter().map(|w| w.to_string()).collect(),
            truth_state: TruthState::Current.code().to_string(),
        });
    }
}

/// The planted protected content, tracked so per-channel cases can reference
/// the exact protected IDs/markers.
struct Protected {
    label_id: String,
    label_marker: String,
    id_id: String,
    count_ids: Vec<String>,
    topo_id: String,
    topo_edges: Vec<String>,
    timing_id: String,
    timing_marker: String,
}

/// Deterministically build every record and link for both worlds.
fn build_structs() -> (Vec<PairedRecord>, Vec<PairedLink>, Protected) {
    let mut b = Builder::new(SEED);

    // === Shared layer: present identically in BOTH worlds =====================
    // These are the only records/edges an unauthorized observer may read, so
    // the observer's projection of World A and World B must be identical.
    let mut shared_ids = Vec::with_capacity(SHARED_RECORDS);
    for i in 0..SHARED_RECORDS {
        let kind = RecordKind::ALL[i % RecordKind::ALL.len()];
        let label = format!("synthetic-public-{i:02}");
        let id = b.record(kind, "shared", &[WORLD_A, WORLD_B], 0, label, T_FROM);
        shared_ids.push(id);
    }
    // A shared topology visible to everyone: a short chain over shared nodes.
    for w in [0usize, 1, 2] {
        let src = shared_ids[w].clone();
        let tgt = shared_ids[w + 1].clone();
        b.edge(&src, &tgt, "shared", &[WORLD_A, WORLD_B]);
    }

    // === Protected layer: present ONLY in World B =============================
    // Each protected item is the sole difference between the two worlds on one
    // observable channel; none may leak to the unauthorized observer.

    // -- hidden label -------------------------------------------------------
    let label_marker = "synthetic-protected-label-medical-note".to_string();
    let label_id = b.record(
        RecordKind::Memory,
        "protected",
        &[WORLD_B],
        3,
        label_marker.clone(),
        T_FROM,
    );

    // -- hidden ID ----------------------------------------------------------
    let id_id = b.record(
        RecordKind::Entity,
        "protected",
        &[WORLD_B],
        3,
        "synthetic-protected-identity".to_string(),
        T_FROM,
    );

    // -- hidden count (extra protected records inflate the true cardinality) --
    let mut count_ids = Vec::new();
    for i in 0..2 {
        let id = b.record(
            RecordKind::Memory,
            "protected",
            &[WORLD_B],
            3,
            format!("synthetic-protected-count-{i}"),
            T_FROM,
        );
        count_ids.push(id);
    }

    // -- hidden topology (protected intermediary + protected edges) ----------
    // shared[0] -> protected_topo -> shared[4], a path that exists only in
    // World B and must be entirely omitted from the observer's topology.
    let topo_id = b.record(
        RecordKind::Memory,
        "protected",
        &[WORLD_B],
        3,
        "synthetic-protected-topology".to_string(),
        T_FROM,
    );
    b.edge(&shared_ids[0], &topo_id, "protected", &[WORLD_B]);
    b.edge(&topo_id, &shared_ids[4], "protected", &[WORLD_B]);
    let topo_edges = vec![
        format!("{}->{}", shared_ids[0], topo_id),
        format!("{}->{}", topo_id, shared_ids[4]),
    ];

    // -- hidden timing (a protected record whose timing signal must not leak) --
    let timing_marker = format!("valid_from={T_PROTECTED_FROM}");
    let timing_id = b.record(
        RecordKind::Episode,
        "protected",
        &[WORLD_B],
        3,
        "synthetic-protected-timing".to_string(),
        T_PROTECTED_FROM,
    );

    let protected = Protected {
        label_id,
        label_marker,
        id_id,
        count_ids,
        topo_id,
        topo_edges,
        timing_id,
        timing_marker,
    };
    (b.records, b.links, protected)
}

// ---------------------------------------------------------------------------
// Projections and oracle
// ---------------------------------------------------------------------------

/// Coarse timing bucket derived ONLY from authorized (visible) work, so it can
/// never encode hidden cardinality (V-POLICY-02).
fn timing_bucket(visible_count: usize) -> String {
    format!("authorized-work-bucket-{}", visible_count.div_ceil(4))
}

/// Compute a caller's observable projection of one world.
fn project(
    records: &[PairedRecord],
    links: &[PairedLink],
    world: &str,
    max_sensitivity: i64,
) -> ObservableProjection {
    let visible: Vec<&PairedRecord> = records
        .iter()
        .filter(|r| r.worlds.iter().any(|w| w == world) && r.policy.sensitivity <= max_sensitivity)
        .collect();
    let visible_ids: BTreeSet<String> = visible.iter().map(|r| r.id.clone()).collect();

    let mut visible_record_ids: Vec<String> = visible_ids.iter().cloned().collect();
    visible_record_ids.sort();

    let mut visible_labels: Vec<String> = visible.iter().map(|r| r.label.clone()).collect();
    visible_labels.sort();

    let mut visible_edges: Vec<String> = links
        .iter()
        .filter(|l| {
            l.worlds.iter().any(|w| w == world)
                && visible_ids.contains(&l.source_id)
                && visible_ids.contains(&l.target_id)
        })
        .map(|l| format!("{}->{}", l.source_id, l.target_id))
        .collect();
    visible_edges.sort();

    let visible_count = visible_record_ids.len();
    let timing = timing_bucket(visible_count);

    ObservableProjection {
        visible_record_ids,
        visible_labels,
        visible_count,
        visible_edges,
        timing_bucket: timing,
    }
}

/// Extract a single channel's observable value from a projection.
fn channel_value(dimension: &str, p: &ObservableProjection) -> String {
    match dimension {
        "label" => p.visible_labels.join("|"),
        "id" => p.visible_record_ids.join("|"),
        "count" => p.visible_count.to_string(),
        "topology" => p.visible_edges.join("|"),
        "timing" => p.timing_bucket.clone(),
        other => panic!("unknown channel dimension {other}"),
    }
}

fn build_oracle(
    records: &[PairedRecord],
    links: &[PairedLink],
    protected: &Protected,
) -> PairedWorldOracle {
    let unauthorized_a = project(records, links, WORLD_A, OBSERVER_MAX_SENSITIVITY);
    let unauthorized_b = project(records, links, WORLD_B, OBSERVER_MAX_SENSITIVITY);
    let authorized_b = project(records, links, WORLD_B, AUTHORIZED_MAX_SENSITIVITY);

    let case = |dimension: &str, protected_ids: Vec<String>, protected_markers: Vec<String>| {
        let a = channel_value(dimension, &unauthorized_a);
        let bb = channel_value(dimension, &unauthorized_b);
        PairedChannelCase {
            dimension: dimension.to_string(),
            protected_ids,
            protected_markers,
            leaks: a != bb,
            unauthorized_value_a: a,
            unauthorized_value_b: bb,
            authorized_value_b: channel_value(dimension, &authorized_b),
        }
    };

    let cases = vec![
        case(
            "label",
            vec![protected.label_id.clone()],
            vec![protected.label_marker.clone()],
        ),
        case(
            "id",
            vec![protected.id_id.clone()],
            vec![protected.id_id.clone()],
        ),
        case(
            "count",
            protected.count_ids.clone(),
            protected.count_ids.clone(),
        ),
        case(
            "topology",
            vec![protected.topo_id.clone()],
            protected.topo_edges.clone(),
        ),
        case(
            "timing",
            vec![protected.timing_id.clone()],
            vec![protected.timing_marker.clone()],
        ),
    ];

    // Forbidden tokens: every protected label, ID, and edge string that must
    // never appear in any unauthorized projection.
    let mut forbidden: BTreeSet<String> = BTreeSet::new();
    for r in records.iter().filter(|r| r.layer == "protected") {
        forbidden.insert(r.id.clone());
        forbidden.insert(r.label.clone());
    }
    for e in &protected.topo_edges {
        forbidden.insert(e.clone());
    }
    forbidden.insert(protected.timing_marker.clone());
    let forbidden_tokens: Vec<String> = forbidden.into_iter().collect();

    let non_interference_holds = unauthorized_a == unauthorized_b && cases.iter().all(|c| !c.leaks);

    PairedWorldOracle {
        observer_max_sensitivity: OBSERVER_MAX_SENSITIVITY,
        authorized_max_sensitivity: AUTHORIZED_MAX_SENSITIVITY,
        unauthorized_projection_a: unauthorized_a,
        unauthorized_projection_b: unauthorized_b,
        authorized_projection_b: authorized_b,
        cases,
        forbidden_tokens,
        non_interference_holds,
    }
}

// ---------------------------------------------------------------------------
// Counts / expected answers / package
// ---------------------------------------------------------------------------

fn compute_counts(records: &[PairedRecord], links: &[PairedLink]) -> FixtureCounts {
    let mut counts = FixtureCounts {
        total_records: records.len(),
        total_links: links.len(),
        valid_records: records.len(),
        invalid_records: 0,
        valid_links: links.len(),
        invalid_links: 0,
        records_by_kind: Default::default(),
        records_by_truth_state: Default::default(),
        records_by_memory_mode: Default::default(),
        records_by_sensitivity: Default::default(),
        links_by_kind: Default::default(),
        idempotency_collisions: 0,
    };
    for r in records {
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
    }
    for l in links {
        *counts.links_by_kind.entry(l.link_type.clone()).or_insert(0) += 1;
    }
    counts
}

fn compute_expected(records: &[PairedRecord]) -> ExpectedAnswers {
    let mut valid_record_ids: Vec<String> = records.iter().map(|r| r.id.clone()).collect();
    valid_record_ids.sort();
    let membership_hash = sha256_hex(valid_record_ids.join("\n").as_bytes());
    ExpectedAnswers {
        valid_record_ids,
        membership_hash,
        invalid_records: Vec::new(),
        invalid_links: Vec::new(),
        idempotency_collisions: Vec::new(),
    }
}

/// Deterministically build the in-memory `mg-policy-pairs-v2` package.
pub fn build() -> FixturePackage {
    let (records, links, protected) = build_structs();
    let oracle = build_oracle(&records, &links, &protected);

    let data_files = vec![
        ("records.json".to_string(), to_json_bytes(&records)),
        ("links.json".to_string(), to_json_bytes(&links)),
    ];
    let (files, package_sha256) = package_files_and_hash(&data_files);
    let counts = compute_counts(&records, &links);
    let expected = compute_expected(&records);

    let manifest = FixtureManifest {
        schema_version: FIXTURE_MANIFEST_SCHEMA.to_string(),
        fixture_id: FIXTURE_ID.to_string(),
        generator: GeneratorMetadata {
            name: "memory_graph::fixtures::policy_pairs_v2".to_string(),
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
        paired_world_oracle: Some(oracle),
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg() -> FixturePackage {
        build()
    }

    fn records_of(pkg: &FixturePackage) -> Vec<PairedRecord> {
        let (_, bytes) = pkg
            .data_files
            .iter()
            .find(|(n, _)| n == "records.json")
            .expect("records.json present");
        serde_json::from_slice(bytes).expect("records deserialize")
    }

    fn oracle_of(pkg: &FixturePackage) -> PairedWorldOracle {
        pkg.manifest
            .paired_world_oracle
            .clone()
            .expect("paired-world oracle present")
    }

    #[test]
    fn seed_and_id_match_validation_contract() {
        assert_eq!(SEED, 0x4D47_5205);
        assert_eq!(FIXTURE_ID, "mg-policy-pairs-v2");
        let m = PolicyPairsV2Generator.generate().manifest;
        assert_eq!(m.generator.seed, 0x4D47_5205);
        assert_eq!(m.generator.seed_hex, "0x4D475205");
        assert_eq!(m.fixture_id, "mg-policy-pairs-v2");
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
    fn worlds_differ_only_by_protected_content() {
        let records = records_of(&pkg());
        // Shared records live in both worlds; protected records only in World B.
        for r in &records {
            match r.layer.as_str() {
                "shared" => {
                    assert_eq!(r.worlds, vec!["a", "b"], "shared record not in both worlds");
                    assert!(r.authorized_for_observer);
                    assert_eq!(r.policy.sensitivity, 0);
                }
                "protected" => {
                    assert_eq!(r.worlds, vec!["b"], "protected record must be World B only");
                    assert!(!r.authorized_for_observer);
                    assert_eq!(r.policy.sensitivity, 3);
                }
                other => panic!("unexpected layer {other}"),
            }
        }
        // At least one protected record per hidden channel is present.
        assert!(records.iter().filter(|r| r.layer == "protected").count() >= 5);
    }

    #[test]
    fn all_five_hidden_channels_present_with_correct_answers() {
        let o = oracle_of(&pkg());
        let dims: BTreeSet<&str> = o.cases.iter().map(|c| c.dimension.as_str()).collect();
        for d in ["label", "id", "count", "topology", "timing"] {
            assert!(dims.contains(d), "missing hidden channel {d}");
        }
        // Every channel is non-interfering for the unauthorized observer.
        for c in &o.cases {
            assert!(!c.leaks, "channel {} leaks protected content", c.dimension);
            assert_eq!(
                c.unauthorized_value_a, c.unauthorized_value_b,
                "channel {} differs between worlds for the observer",
                c.dimension
            );
        }
    }

    #[test]
    fn authorized_caller_reveals_protected_on_every_channel() {
        let o = oracle_of(&pkg());
        for c in &o.cases {
            // The authorized caller sees strictly more on each channel: the
            // protected content really exists in World B where authorized.
            assert_ne!(
                c.authorized_value_b, c.unauthorized_value_b,
                "channel {} should reveal protected content to authorized caller",
                c.dimension
            );
        }
        // Concretely: authorized count exceeds observer count by the protected set.
        assert!(
            o.authorized_projection_b.visible_count > o.unauthorized_projection_b.visible_count
        );
    }

    #[test]
    fn unauthorized_projection_is_identical_across_worlds() {
        let o = oracle_of(&pkg());
        assert_eq!(
            o.unauthorized_projection_a, o.unauthorized_projection_b,
            "observer must not distinguish World A from World B"
        );
        assert!(o.non_interference_holds);
        // The observer only ever sees the shared cardinality.
        assert_eq!(o.unauthorized_projection_a.visible_count, SHARED_RECORDS);
    }

    #[test]
    fn no_protected_token_appears_in_any_unauthorized_projection() {
        let o = oracle_of(&pkg());
        assert!(!o.forbidden_tokens.is_empty());
        for token in &o.forbidden_tokens {
            for p in [&o.unauthorized_projection_a, &o.unauthorized_projection_b] {
                assert!(
                    !p.visible_record_ids.contains(token),
                    "protected token {token} leaked into visible IDs"
                );
                assert!(
                    !p.visible_labels.contains(token),
                    "protected token {token} leaked into visible labels"
                );
                assert!(
                    !p.visible_edges.iter().any(|e| e == token),
                    "protected token {token} leaked into visible topology"
                );
            }
        }
    }

    #[test]
    fn hidden_timing_does_not_encode_hidden_cardinality() {
        let o = oracle_of(&pkg());
        // Observer's timing bucket is derived only from authorized work, so it
        // is identical across worlds despite World B holding more records.
        assert_eq!(
            o.unauthorized_projection_a.timing_bucket,
            o.unauthorized_projection_b.timing_bucket
        );
        let timing_case = o.cases.iter().find(|c| c.dimension == "timing").unwrap();
        assert_eq!(
            timing_case.unauthorized_value_a,
            timing_case.unauthorized_value_b
        );
    }

    #[test]
    fn hidden_topology_path_is_omitted_for_observer() {
        let o = oracle_of(&pkg());
        // The protected path shared[0]->P->shared[4] must not appear as any
        // observable edge for the unauthorized caller.
        let topo_case = o.cases.iter().find(|c| c.dimension == "topology").unwrap();
        for edge in &topo_case.protected_markers {
            assert!(
                !o.unauthorized_projection_b.visible_edges.contains(edge),
                "protected edge {edge} surfaced to observer"
            );
        }
    }

    #[test]
    fn membership_hash_is_independent_and_stable() {
        let p = pkg();
        let records = records_of(&p);
        let mut ids: Vec<String> = records.iter().map(|r| r.id.clone()).collect();
        ids.sort();
        assert_eq!(
            p.manifest.expected.membership_hash,
            sha256_hex(ids.join("\n").as_bytes())
        );
        assert_eq!(p.manifest.expected.valid_record_ids, ids);
        assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), ids.len());
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
        assert!(m.vector_oracle.is_none());
        assert!(m.paired_world_oracle.is_some());
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
    fn no_record_content_contains_private_data_markers() {
        for r in &records_of(&pkg()) {
            assert!(
                r.content.starts_with("synthetic mg-policy-pairs-v2"),
                "unexpected content: {}",
                r.content
            );
            assert!(
                r.label.starts_with("synthetic-"),
                "unexpected label: {}",
                r.label
            );
        }
    }

    #[test]
    fn materializes_committed_package_to_repo() {
        let root = super::super::generated_root();
        let dir = pkg().materialize(&root).expect("materialize package");
        for f in ["records.json", "links.json", "fixture-manifest.json"] {
            assert!(dir.join(f).exists(), "missing {f}");
        }
        let on_disk = std::fs::read(dir.join("fixture-manifest.json")).unwrap();
        assert_eq!(on_disk, pkg().manifest_bytes());
    }
}
