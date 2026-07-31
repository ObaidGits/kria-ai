//! `mg-release-v2` deterministic fixture generator (task F0.2 / 0.2.3).
//!
//! Seed `0x4D475204`. At **release scale** this generator materializes
//! **100,000 authority records** wired into a graph that plants every case
//! required by `validation.md` §2 and the ≤3-hop traversal contract
//! (`V-GRAPH-01`, design.md §6.2/§6.5, A6/A7, MGR-007):
//!
//! * a **fixed degree distribution** over graph nodes,
//! * **cycles** (directed rings + back edges) so BFS must be cycle-safe,
//! * **hidden intermediaries** — unauthorized nodes whose presence omits the
//!   *entire* path from policy-gated results,
//! * **temporal boundaries** — exact half-open Valid-Time boundary records,
//! * **1/2/3/4-hop paths** with exact independent expected memberships, where
//!   the **4-hop** target is unreachable under the ≤3-hop boundedness limit,
//! * **exact independent memberships** — the oracle, defined here and never
//!   derived from a system under test.
//!
//! ## Frozen contract, deferred materialization
//!
//! Per parent task 0.2 ("generate 100/1k metadata now, defer expensive 100k
//! materialization to F3/F5 while freezing its generator/hash contract"), this
//! module **freezes** the generator and its hash/membership contract but does
//! **not** commit the full 100,000-record package (its `records.json` alone is
//! ~60 MB). Instead it commits:
//!
//! * a cheap, deterministic **sample slice** ([`SAMPLE_PARAMS`],
//!   [`SAMPLE_TOTAL_RECORDS`] records) whose planted graph structure (anchors,
//!   hidden intermediaries, temporal boundaries, cycle probes) is **byte-for-byte
//!   identical** to the full run, because the special region is generated before
//!   any scale-dependent bulk records; and
//! * a metadata-only [`ReleaseFrozenContract`] (`frozen-contract.json`) pinning
//!   the full-run parameters, expected 100k counts, the exact full membership
//!   hash, and the membership-hashing method, so F3/F5 can materialize the full
//!   corpus and verify it byte-for-byte.
//!
//! [`build`]`(&`[`FULL_PARAMS`]`)` produces the full corpus on demand; F3/F5
//! call it to materialize `tests/fixtures/memory-graph/generated/mg-release-v2/`.
//!
//! All content is synthetic — no real private data.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    package_files_and_hash, sha256_hex, CycleProbe, ExpectedAnswers, FixtureCounts,
    FixtureGenerator, FixtureManifest, FixturePackage, GeneratorMetadata, HiddenIntermediaryCase,
    InvalidCase, LinkKind, MemoryMode, PathAnchor, Policy, RecordKind, ReleaseGraphOracle,
    SchemaVersions, SplitMix64, TemporalCase, TruthState, FIXTURE_MANIFEST_SCHEMA,
    GENERATOR_VERSION,
};

/// The frozen seed for `mg-release-v2` (`validation.md` §2).
pub const SEED: u64 = 0x4D47_5204;

/// The fixture identifier.
pub const FIXTURE_ID: &str = "mg-release-v2";

/// Frozen full-run record count (`validation.md`: "100,000 authority records").
pub const FULL_TOTAL_RECORDS: usize = 100_000;

/// Committed cheap-slice record count. Large enough to exercise the bulk
/// degree distribution and cycles while staying small to commit and test.
pub const SAMPLE_TOTAL_RECORDS: usize = 2_000;

/// The ≤3-hop boundedness limit under which reachability is defined
/// (design.md §6.2 A6; `neighborhood`/`path` APIs are 1/3 and 3/3 hops).
pub const HOP_LIMIT: u32 = 3;

/// Frozen-contract schema identifier.
pub const RELEASE_CONTRACT_SCHEMA: &str = "memory-graph-release-contract/v1";

/// The membership-hashing method F3/F5 must reproduce byte-for-byte.
pub const MEMBERSHIP_HASH_METHOD: &str =
    "sha256(hex) over newline-joined lexicographically-sorted valid record IDs";

/// Human-readable description of the frozen degree distribution.
pub const DEGREE_DISTRIBUTION_SPEC: &str =
    "bulk out-degree drawn uniformly from buckets [0,1,1,1,2,2,3,5,8] (mean 2.55, hubs at 5/8); \
     special region uses exact planted degrees";

// ---------------------------------------------------------------------------
// Fixed synthetic timestamps (RFC3339 UTC). Lexicographic order == chronology.
// ---------------------------------------------------------------------------

const T_PAST: &str = "2023-01-01T00:00:00Z";
const T_PAST_MID: &str = "2023-06-01T00:00:00Z";
/// The fixed query instant temporal membership is resolved against.
const T_QUERY: &str = "2024-06-01T00:00:00Z";
const T_FUTURE: &str = "2025-01-01T00:00:00Z";

const NAMESPACES: [&str; 3] = ["personal", "work", "shared"];
const SCOPES: [&str; 3] = ["private", "team", "public"];
const OWNERS: [&str; 2] = ["owner-alpha", "owner-beta"];

/// Fixed bulk out-degree buckets (the frozen degree distribution).
const DEGREE_BUCKETS: [u32; 9] = [0, 1, 1, 1, 2, 2, 3, 5, 8];

/// The dangling target used by the planted invalid link.
const ZERO_UUID: &str = "00000000-0000-0000-0000-000000000000";

// ---------------------------------------------------------------------------
// Planted row types
// ---------------------------------------------------------------------------

/// One planted authority record (a graph node).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseRecord {
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
    /// Region tag: `anchor`/`hidden`/`temporal`/`cycle`/`bulk`/`invalid`.
    pub region: String,
    /// Whether the caller is authorized to see this node. Hidden intermediaries
    /// are `false` and must be omitted (with their whole path) from results.
    pub authorized: bool,
    /// Planted out-degree (number of outgoing links from this node).
    pub out_degree: u32,
    /// Half-open Valid-Time start (RFC3339 UTC), if bounded.
    pub valid_from: Option<String>,
    /// Half-open Valid-Time end (RFC3339 UTC), if bounded.
    pub valid_until: Option<String>,
    /// Temporal boundary case code, if this row is a planted boundary case.
    pub temporal_case: Option<String>,
    /// Synthetic content payload.
    pub content: String,
    /// SHA-256 of `content`.
    pub content_hash: String,
    /// Whether the row satisfies every schema constraint.
    pub valid: bool,
    /// Reason code when `valid == false`.
    pub invalid_reason: Option<String>,
}

/// One planted semantic link (a directed graph edge).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseLink {
    /// Canonical link ID.
    pub id: String,
    /// Link kind code.
    pub link_type: String,
    /// Source endpoint record ID.
    pub source_id: String,
    /// Target endpoint record ID.
    pub target_id: String,
    /// Truth state code.
    pub truth_state: String,
    /// Whether this edge closes a cycle (a back edge).
    pub cycle_edge: bool,
    /// Whether an endpoint is an unauthorized hidden intermediary.
    pub crosses_hidden: bool,
    /// Whether the row satisfies every schema/endpoint constraint.
    pub valid: bool,
    /// Reason code when `valid == false`.
    pub invalid_reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Generator parameters
// ---------------------------------------------------------------------------

/// Per-run parameters. Only `total_records` differs between the committed slice
/// and the frozen full corpus.
#[derive(Debug, Clone, Copy)]
pub struct ReleaseParams {
    /// Fixture ID.
    pub fixture_id: &'static str,
    /// Frozen seed.
    pub seed: u64,
    /// Generator module name recorded in the manifest.
    pub generator_name: &'static str,
    /// Total authority records to materialize (special region + bulk).
    pub total_records: usize,
}

/// The frozen **full** release contract: 100,000 records (deferred to F3/F5).
pub const FULL_PARAMS: ReleaseParams = ReleaseParams {
    fixture_id: FIXTURE_ID,
    seed: SEED,
    generator_name: "memory_graph::fixtures::release_v2",
    total_records: FULL_TOTAL_RECORDS,
};

/// The committed cheap **slice** used for determinism/structure tests now.
pub const SAMPLE_PARAMS: ReleaseParams = ReleaseParams {
    fixture_id: FIXTURE_ID,
    seed: SEED,
    generator_name: "memory_graph::fixtures::release_v2",
    total_records: SAMPLE_TOTAL_RECORDS,
};

// ---------------------------------------------------------------------------
// Frozen contract
// ---------------------------------------------------------------------------

/// Metadata-only freeze of the full 100k contract. Committed as
/// `frozen-contract.json` so F3/F5 can materialize and verify byte-for-byte
/// without this task committing the ~60 MB corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseFrozenContract {
    /// Contract schema identifier ([`RELEASE_CONTRACT_SCHEMA`]).
    pub schema_version: String,
    /// Fixture ID.
    pub fixture_id: String,
    /// Versioned generator metadata.
    pub generator: GeneratorMetadata,
    /// Frozen full-run record count (`100_000`).
    pub full_total_records: usize,
    /// Frozen full-run link count.
    pub full_total_links: usize,
    /// Frozen full-run count breakdowns (independent oracle).
    pub full_counts: FixtureCounts,
    /// Frozen full-run membership hash over sorted valid record IDs.
    pub full_membership_hash: String,
    /// The exact method used to compute the membership hash.
    pub membership_hash_method: String,
    /// Description of the frozen degree distribution.
    pub degree_distribution_spec: String,
    /// Frozen full-run graph traversal oracle.
    pub full_oracle: ReleaseGraphOracle,
    /// Record count of the committed sample slice.
    pub sample_total_records: usize,
    /// Package SHA-256 of the committed sample slice (ties slice to contract).
    pub sample_package_sha256: String,
    /// Whether the full corpus data files are materialized in the repo now.
    pub full_data_materialized: bool,
    /// Gate that owns full materialization.
    pub deferred_to: String,
    /// Human-readable rationale for the deferral.
    pub note: String,
}

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

/// The `mg-release-v2` generator. [`FixtureGenerator::generate`] produces the
/// cheap committed **slice**; call [`build`]`(&`[`FULL_PARAMS`]`)` for the full
/// corpus at F3/F5.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReleaseV2Generator;

impl FixtureGenerator for ReleaseV2Generator {
    fn fixture_id(&self) -> &'static str {
        FIXTURE_ID
    }

    fn seed(&self) -> u64 {
        SEED
    }

    fn generate(&self) -> FixturePackage {
        build(&SAMPLE_PARAMS)
    }
}

/// Serialize a value to canonical pretty JSON bytes with a trailing newline.
fn to_json_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    let mut s = serde_json::to_string_pretty(value).expect("serializes to JSON");
    s.push('\n');
    s.into_bytes()
}

/// Is a record with the given half-open interval current at [`T_QUERY`]?
/// Half-open `[valid_from, valid_until)`: start inclusive, end exclusive.
fn current_at_query(valid_from: Option<&str>, valid_until: Option<&str>) -> bool {
    let from_ok = valid_from.map(|f| f <= T_QUERY).unwrap_or(true);
    let until_ok = valid_until.map(|u| T_QUERY < u).unwrap_or(true);
    from_ok && until_ok
}

// ---------------------------------------------------------------------------
// Structural builder
// ---------------------------------------------------------------------------

/// Accumulates records and links while drawing IDs from the shared PRNG.
struct Builder {
    rng: SplitMix64,
    records: Vec<ReleaseRecord>,
    links: Vec<ReleaseLink>,
    /// Running policy-sensitivity index for deterministic lattice coverage.
    seq: usize,
}

impl Builder {
    fn new(seed: u64) -> Self {
        Self {
            rng: SplitMix64::new(seed),
            records: Vec::new(),
            links: Vec::new(),
            seq: 0,
        }
    }

    /// Deterministic policy tuple; `seq` advances the sensitivity lattice.
    fn next_policy(&mut self) -> Policy {
        let ns = NAMESPACES[self.rng.below(NAMESPACES.len())].to_string();
        let owner = OWNERS[self.rng.below(OWNERS.len())].to_string();
        let scope = SCOPES[self.rng.below(SCOPES.len())].to_string();
        let sensitivity = (self.seq % 4) as i64;
        self.seq += 1;
        Policy {
            namespace: ns,
            owner,
            scope,
            sensitivity,
        }
    }

    /// Push a valid authority record; returns its ID.
    #[allow(clippy::too_many_arguments)]
    fn node(
        &mut self,
        region: &str,
        kind: RecordKind,
        authorized: bool,
        out_degree: u32,
        valid_from: Option<&str>,
        valid_until: Option<&str>,
        temporal_case: Option<&str>,
    ) -> String {
        let id = self.rng.next_uuid();
        let policy = self.next_policy();
        let content = format!(
            "synthetic mg-release-v2 {region} node {} :: {}",
            self.records.len(),
            kind.code()
        );
        self.records.push(ReleaseRecord {
            id: id.clone(),
            record_kind: kind.code().to_string(),
            truth_state: TruthState::Current.code().to_string(),
            memory_mode: MemoryMode::Permanent.code().to_string(),
            policy,
            region: region.to_string(),
            authorized,
            out_degree,
            valid_from: valid_from.map(str::to_string),
            valid_until: valid_until.map(str::to_string),
            temporal_case: temporal_case.map(str::to_string),
            content_hash: sha256_hex(content.as_bytes()),
            content,
            valid: true,
            invalid_reason: None,
        });
        id
    }

    /// Push a directed edge.
    fn edge(
        &mut self,
        link: LinkKind,
        src: &str,
        tgt: &str,
        cycle_edge: bool,
        crosses_hidden: bool,
    ) {
        let id = self.rng.next_uuid();
        self.links.push(ReleaseLink {
            id,
            link_type: link.code().to_string(),
            source_id: src.to_string(),
            target_id: tgt.to_string(),
            truth_state: TruthState::Current.code().to_string(),
            cycle_edge,
            crosses_hidden,
            valid: true,
            invalid_reason: None,
        });
    }
}

/// Deterministically build every record, link, and the independent oracle for
/// `params`. The **special region** (anchors, hidden, temporal, cycle, invalid)
/// is emitted first with a fixed number of PRNG draws, so it is byte-identical
/// at every scale; the scale-dependent **bulk region** follows.
pub fn build_structs(
    params: &ReleaseParams,
) -> (Vec<ReleaseRecord>, Vec<ReleaseLink>, ReleaseGraphOracle) {
    let mut b = Builder::new(params.seed);

    // === Anchor region: one source with 1/2/3/4-hop branches ================
    // BFS ≤3 from S reaches the 1/2/3-hop targets but never the 4-hop target.
    let s = b.node(
        "anchor",
        RecordKind::Memory,
        true,
        4,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    // 1-hop branch: S -> t1
    let t1 = b.node(
        "anchor",
        RecordKind::Summary,
        true,
        0,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    // 2-hop branch: S -> a1 -> t2
    let a1 = b.node(
        "anchor",
        RecordKind::Memory,
        true,
        1,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    let t2 = b.node(
        "anchor",
        RecordKind::Summary,
        true,
        0,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    // 3-hop branch: S -> b1 -> b2 -> t3
    let b1 = b.node(
        "anchor",
        RecordKind::Memory,
        true,
        1,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    let b2 = b.node(
        "anchor",
        RecordKind::Memory,
        true,
        1,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    let t3 = b.node(
        "anchor",
        RecordKind::Summary,
        true,
        0,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    // 4-hop branch: S -> c1 -> c2 -> c3 -> t4  (t4 is unreachable at ≤3 hops)
    let c1 = b.node(
        "anchor",
        RecordKind::Memory,
        true,
        1,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    let c2 = b.node(
        "anchor",
        RecordKind::Memory,
        true,
        1,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    let c3 = b.node(
        "anchor",
        RecordKind::Memory,
        true,
        1,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    let t4 = b.node(
        "anchor",
        RecordKind::Summary,
        true,
        0,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );

    b.edge(LinkKind::DerivedFrom, &s, &t1, false, false);
    b.edge(LinkKind::DerivedFrom, &s, &a1, false, false);
    b.edge(LinkKind::DerivedFrom, &a1, &t2, false, false);
    b.edge(LinkKind::DerivedFrom, &s, &b1, false, false);
    b.edge(LinkKind::DerivedFrom, &b1, &b2, false, false);
    b.edge(LinkKind::DerivedFrom, &b2, &t3, false, false);
    b.edge(LinkKind::DerivedFrom, &s, &c1, false, false);
    b.edge(LinkKind::DerivedFrom, &c1, &c2, false, false);
    b.edge(LinkKind::DerivedFrom, &c2, &c3, false, false);
    b.edge(LinkKind::DerivedFrom, &c3, &t4, false, false);

    let path_anchors = vec![
        PathAnchor {
            source_id: s.clone(),
            target_id: t1.clone(),
            hop_distance: 1,
            reachable_within_limit: true,
            path_ids: vec![s.clone(), t1.clone()],
        },
        PathAnchor {
            source_id: s.clone(),
            target_id: t2.clone(),
            hop_distance: 2,
            reachable_within_limit: true,
            path_ids: vec![s.clone(), a1.clone(), t2.clone()],
        },
        PathAnchor {
            source_id: s.clone(),
            target_id: t3.clone(),
            hop_distance: 3,
            reachable_within_limit: true,
            path_ids: vec![s.clone(), b1.clone(), b2.clone(), t3.clone()],
        },
        PathAnchor {
            source_id: s.clone(),
            target_id: t4.clone(),
            hop_distance: 4,
            // Exceeds the ≤3-hop boundedness limit — MUST be unreachable.
            reachable_within_limit: false,
            path_ids: vec![s.clone(), c1.clone(), c2.clone(), c3.clone(), t4.clone()],
        },
    ];

    // === Hidden-intermediary region: sh -> H(hidden) -> th ==================
    let sh = b.node(
        "hidden",
        RecordKind::Memory,
        true,
        1,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    let hidden = b.node(
        "hidden",
        RecordKind::Memory,
        false,
        1,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    let th = b.node(
        "hidden",
        RecordKind::Summary,
        true,
        0,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    b.edge(LinkKind::DerivedFrom, &sh, &hidden, false, true);
    b.edge(LinkKind::DerivedFrom, &hidden, &th, false, true);
    let hidden_intermediary_cases = vec![HiddenIntermediaryCase {
        source_id: sh,
        target_id: th,
        hidden_intermediary_id: hidden,
        topological_hop_distance: 2,
        reachable_ignoring_policy: true,
        // Hidden intermediary removes the whole path (design §6.5 / V-GRAPH-01).
        reachable_with_policy: false,
    }];

    // === Temporal-boundary region ===========================================
    let temporal_specs: [(&str, Option<&str>, Option<&str>); 6] = [
        (
            "valid_from_inclusive_boundary",
            Some(T_QUERY),
            Some(T_FUTURE),
        ),
        (
            "valid_until_exclusive_boundary",
            Some(T_PAST),
            Some(T_QUERY),
        ),
        ("open_ended_current", Some(T_PAST), None),
        ("future_not_yet", Some(T_FUTURE), None),
        ("empty_instant", Some(T_QUERY), Some(T_QUERY)),
        ("past_closed", Some(T_PAST), Some(T_PAST_MID)),
    ];
    let mut temporal_cases = Vec::with_capacity(temporal_specs.len());
    for (case, from, until) in temporal_specs {
        let id = b.node(
            "temporal",
            RecordKind::Memory,
            true,
            0,
            from,
            until,
            Some(case),
        );
        temporal_cases.push(TemporalCase {
            record_id: id,
            case: case.to_string(),
            valid_from: from.map(str::to_string),
            valid_until: until.map(str::to_string),
            current_at_query_instant: current_at_query(from, until),
        });
    }

    // === Cycle region: entry Sc -> ring d1 -> d2 -> d3 -> d1 ================
    let sc = b.node(
        "cycle",
        RecordKind::Memory,
        true,
        1,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    let d1 = b.node(
        "cycle",
        RecordKind::Memory,
        true,
        1,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    let d2 = b.node(
        "cycle",
        RecordKind::Memory,
        true,
        1,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    let d3 = b.node(
        "cycle",
        RecordKind::Memory,
        true,
        1,
        Some(T_PAST),
        Some(T_FUTURE),
        None,
    );
    b.edge(LinkKind::DerivedFrom, &sc, &d1, false, false);
    b.edge(LinkKind::DerivedFrom, &d1, &d2, false, false);
    b.edge(LinkKind::DerivedFrom, &d2, &d3, false, false);
    // Back edge closes the ring: BFS must terminate and repeat no path node.
    b.edge(LinkKind::DerivedFrom, &d3, &d1, true, false);
    let cycle_probes = vec![CycleProbe {
        source_id: sc,
        ring_ids: vec![d1.clone(), d2.clone(), d3.clone()],
        reachable_within_limit: vec![d1, d2, d3],
    }];

    // === Invalid region (planted schema violations) =========================
    push_invalid(&mut b);

    let special_records = b.records.len();
    let special_links = b.links.len();

    // === Bulk region ========================================================
    // Fill up to `total_records` with ordinary authorized nodes wired by the
    // frozen degree distribution, including deliberate back edges (cycles).
    build_bulk(&mut b, params.total_records.saturating_sub(special_records));

    // === Oracle: degree distribution + cycle totals =========================
    let mut degree_distribution: BTreeMap<String, usize> = BTreeMap::new();
    for r in b.records.iter().filter(|r| r.valid) {
        *degree_distribution
            .entry(r.out_degree.to_string())
            .or_insert(0) += 1;
    }
    let cycle_edges = b.links.iter().filter(|l| l.cycle_edge).count();

    let oracle = ReleaseGraphOracle {
        full_total_records: FULL_TOTAL_RECORDS,
        materialized_full: params.total_records == FULL_TOTAL_RECORDS,
        hop_limit: HOP_LIMIT,
        degree_distribution,
        path_anchors,
        hidden_intermediary_cases,
        temporal_cases,
        cycle_probes,
        cycle_edges,
        temporal_query_instant: T_QUERY.to_string(),
    };

    debug_assert!(b.records.len() >= special_records);
    debug_assert!(b.links.len() >= special_links);
    (b.records, b.links, oracle)
}

/// Plant a fixed set of invalid records and one invalid link.
fn push_invalid(b: &mut Builder) {
    // sensitivity out of range (0..=3 required)
    {
        let id = b.rng.next_uuid();
        let content = "synthetic mg-release-v2 invalid :: sensitivity_out_of_range".to_string();
        b.records.push(ReleaseRecord {
            id,
            record_kind: RecordKind::Memory.code().to_string(),
            truth_state: TruthState::Current.code().to_string(),
            memory_mode: MemoryMode::Permanent.code().to_string(),
            policy: Policy {
                namespace: "work".to_string(),
                owner: "owner-beta".to_string(),
                scope: "team".to_string(),
                sensitivity: 4,
            },
            region: "invalid".to_string(),
            authorized: true,
            out_degree: 0,
            valid_from: Some(T_PAST.to_string()),
            valid_until: Some(T_FUTURE.to_string()),
            temporal_case: None,
            content_hash: sha256_hex(content.as_bytes()),
            content,
            valid: false,
            invalid_reason: Some("sensitivity_out_of_range".to_string()),
        });
    }
    // inverted valid interval (valid_from > valid_until)
    {
        let id = b.rng.next_uuid();
        let content = "synthetic mg-release-v2 invalid :: invalid_valid_interval".to_string();
        b.records.push(ReleaseRecord {
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
            region: "invalid".to_string(),
            authorized: true,
            out_degree: 0,
            valid_from: Some(T_FUTURE.to_string()),
            valid_until: Some(T_PAST.to_string()),
            temporal_case: None,
            content_hash: sha256_hex(content.as_bytes()),
            content,
            valid: false,
            invalid_reason: Some("invalid_valid_interval".to_string()),
        });
    }
    // empty namespace
    {
        let id = b.rng.next_uuid();
        let content = "synthetic mg-release-v2 invalid :: empty_namespace".to_string();
        b.records.push(ReleaseRecord {
            id,
            record_kind: RecordKind::Memory.code().to_string(),
            truth_state: TruthState::Current.code().to_string(),
            memory_mode: MemoryMode::Permanent.code().to_string(),
            policy: Policy {
                namespace: String::new(),
                owner: "owner-beta".to_string(),
                scope: "team".to_string(),
                sensitivity: 1,
            },
            region: "invalid".to_string(),
            authorized: true,
            out_degree: 0,
            valid_from: Some(T_PAST.to_string()),
            valid_until: Some(T_FUTURE.to_string()),
            temporal_case: None,
            content_hash: sha256_hex(content.as_bytes()),
            content,
            valid: false,
            invalid_reason: Some("empty_namespace".to_string()),
        });
    }
    // dangling-endpoint invalid link (target references no record)
    {
        let src = b.records[0].id.clone();
        let id = b.rng.next_uuid();
        b.links.push(ReleaseLink {
            id,
            link_type: LinkKind::Supports.code().to_string(),
            source_id: src,
            target_id: ZERO_UUID.to_string(),
            truth_state: TruthState::Current.code().to_string(),
            cycle_edge: false,
            crosses_hidden: false,
            valid: false,
            invalid_reason: Some("dangling_endpoint".to_string()),
        });
    }
}

/// Build `count` ordinary bulk nodes and wire them by the frozen degree
/// distribution, threading a forward chain (with a wrap-around back edge) so the
/// bulk region always contains cycles.
fn build_bulk(b: &mut Builder, count: usize) {
    if count == 0 {
        return;
    }
    // First draw per-node out-degrees and create nodes, so `out_degree` is
    // authoritative and the degree histogram is exact.
    let mut ids = Vec::with_capacity(count);
    let mut degrees = Vec::with_capacity(count);
    for i in 0..count {
        let degree = DEGREE_BUCKETS[b.rng.below(DEGREE_BUCKETS.len())];
        let kind = RecordKind::ALL[i % RecordKind::ALL.len()];
        let id = b.node(
            "bulk",
            kind,
            true,
            degree,
            Some(T_PAST),
            Some(T_FUTURE),
            None,
        );
        ids.push(id);
        degrees.push(degree);
    }
    // Wire edges. Edge k from node i targets (i + 1 + k*step) % count; when the
    // target index is not strictly ahead it is a back edge closing a cycle.
    for i in 0..count {
        let degree = degrees[i] as usize;
        for k in 0..degree {
            let step = 1 + (k * 7);
            let tgt_idx = (i + step) % count;
            let cycle_edge = tgt_idx <= i;
            let link = LinkKind::ALL[k % LinkKind::ALL.len()];
            let src = ids[i].clone();
            let tgt = ids[tgt_idx].clone();
            b.edge(link, &src, &tgt, cycle_edge, false);
        }
    }
}

// ---------------------------------------------------------------------------
// Counts / oracle / package
// ---------------------------------------------------------------------------

fn compute_counts(records: &[ReleaseRecord], links: &[ReleaseLink]) -> FixtureCounts {
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
    counts
}

fn compute_expected(records: &[ReleaseRecord], links: &[ReleaseLink]) -> ExpectedAnswers {
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
        idempotency_collisions: Vec::new(),
    }
}

/// Deterministically build the full in-memory fixture package for `params`.
///
/// At [`FULL_PARAMS`] this materializes the full 100k corpus (F3/F5). At
/// [`SAMPLE_PARAMS`] it produces the cheap committed slice.
pub fn build(params: &ReleaseParams) -> FixturePackage {
    let (records, links, oracle) = build_structs(params);

    let data_files = vec![
        ("records.json".to_string(), to_json_bytes(&records)),
        ("links.json".to_string(), to_json_bytes(&links)),
    ];
    let (files, package_sha256) = package_files_and_hash(&data_files);
    let counts = compute_counts(&records, &links);
    let expected = compute_expected(&records, &links);

    let manifest = FixtureManifest {
        schema_version: FIXTURE_MANIFEST_SCHEMA.to_string(),
        fixture_id: params.fixture_id.to_string(),
        generator: GeneratorMetadata {
            name: params.generator_name.to_string(),
            version: GENERATOR_VERSION.to_string(),
            algorithm: "splitmix64".to_string(),
            seed_hex: format!("0x{:08X}", params.seed),
            seed: params.seed,
        },
        schema_versions: SchemaVersions::default(),
        counts,
        expected,
        files,
        package_sha256,
        contains_private_data: false,
        scene_coverage: None,
        release_oracle: Some(oracle),
        paired_world_oracle: None,
        vector_oracle: None,
        judged_corpus_oracle: None,
        interchange_oracle: None,
        visual_scene_oracle: None,
    };

    FixturePackage {
        fixture_id: params.fixture_id.to_string(),
        data_files,
        manifest,
    }
}

/// Compute the frozen full-run contract (metadata only — no data-file bytes).
///
/// Builds the full 100k graph in memory to freeze exact counts, the membership
/// hash, and the traversal oracle, but never serializes the ~60 MB data files.
pub fn frozen_contract() -> ReleaseFrozenContract {
    let (records, links, full_oracle) = build_structs(&FULL_PARAMS);
    let full_counts = compute_counts(&records, &links);
    let expected = compute_expected(&records, &links);

    let sample = build(&SAMPLE_PARAMS);

    ReleaseFrozenContract {
        schema_version: RELEASE_CONTRACT_SCHEMA.to_string(),
        fixture_id: FIXTURE_ID.to_string(),
        generator: GeneratorMetadata {
            name: FULL_PARAMS.generator_name.to_string(),
            version: GENERATOR_VERSION.to_string(),
            algorithm: "splitmix64".to_string(),
            seed_hex: format!("0x{:08X}", SEED),
            seed: SEED,
        },
        full_total_records: records.len(),
        full_total_links: links.len(),
        full_counts,
        full_membership_hash: expected.membership_hash,
        membership_hash_method: MEMBERSHIP_HASH_METHOD.to_string(),
        degree_distribution_spec: DEGREE_DISTRIBUTION_SPEC.to_string(),
        full_oracle,
        sample_total_records: SAMPLE_TOTAL_RECORDS,
        sample_package_sha256: sample.manifest.package_sha256,
        full_data_materialized: false,
        deferred_to: "F3/F5".to_string(),
        note: "Full 100k data files (~60 MB) are deferred to F3/F5; the generator \
               and its membership/hash contract are frozen here. Materialize with \
               release_v2::build(&FULL_PARAMS) and verify against this contract."
            .to_string(),
    }
}

/// Serialize the frozen contract to canonical pretty JSON bytes (trailing newline).
pub fn frozen_contract_bytes(contract: &ReleaseFrozenContract) -> Vec<u8> {
    to_json_bytes(contract)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

    fn pkg() -> FixturePackage {
        build(&SAMPLE_PARAMS)
    }

    fn records_of(pkg: &FixturePackage) -> Vec<ReleaseRecord> {
        let (_, bytes) = pkg
            .data_files
            .iter()
            .find(|(n, _)| n == "records.json")
            .expect("records.json present");
        serde_json::from_slice(bytes).expect("records deserialize")
    }

    fn links_of(pkg: &FixturePackage) -> Vec<ReleaseLink> {
        let (_, bytes) = pkg
            .data_files
            .iter()
            .find(|(n, _)| n == "links.json")
            .expect("links.json present");
        serde_json::from_slice(bytes).expect("links deserialize")
    }

    /// Build a forward adjacency map from valid links.
    fn adjacency(links: &[ReleaseLink]) -> HashMap<String, Vec<String>> {
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();
        for l in links.iter().filter(|l| l.valid) {
            adj.entry(l.source_id.clone())
                .or_default()
                .push(l.target_id.clone());
        }
        adj
    }

    /// Independent cycle-safe BFS bounded by `limit` hops. `allow` gates which
    /// nodes may be entered (used to model hidden-intermediary policy). Returns
    /// reachable node -> shortest hop distance (excludes `start`).
    fn bfs(
        adj: &HashMap<String, Vec<String>>,
        start: &str,
        limit: u32,
        allow: &dyn Fn(&str) -> bool,
    ) -> HashMap<String, u32> {
        let mut dist: HashMap<String, u32> = HashMap::new();
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(start.to_string());
        let mut q: VecDeque<(String, u32)> = VecDeque::new();
        q.push_back((start.to_string(), 0));
        while let Some((node, d)) = q.pop_front() {
            if d == limit {
                continue;
            }
            if let Some(neighbors) = adj.get(&node) {
                for n in neighbors {
                    if !allow(n) || !seen.insert(n.clone()) {
                        continue;
                    }
                    dist.insert(n.clone(), d + 1);
                    q.push_back((n.clone(), d + 1));
                }
            }
        }
        dist
    }

    #[test]
    fn seed_and_id_match_validation_contract() {
        assert_eq!(SEED, 0x4D47_5204);
        assert_eq!(FIXTURE_ID, "mg-release-v2");
        assert_eq!(FULL_TOTAL_RECORDS, 100_000);
        let m = ReleaseV2Generator.generate().manifest;
        assert_eq!(m.generator.seed, 0x4D47_5204);
        assert_eq!(m.generator.seed_hex, "0x4D475204");
        assert_eq!(m.fixture_id, "mg-release-v2");
        assert_eq!(m.generator.algorithm, "splitmix64");
    }

    #[test]
    fn sample_slice_has_expected_size() {
        let p = pkg();
        assert_eq!(records_of(&p).len(), SAMPLE_TOTAL_RECORDS);
        assert_eq!(p.manifest.counts.total_records, SAMPLE_TOTAL_RECORDS);
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
    fn special_region_is_scale_invariant() {
        // The planted graph structure must be byte-identical regardless of the
        // bulk scale (the frozen-contract relies on this).
        let (small, _, o_small) = build_structs(&SAMPLE_PARAMS);
        let bigger = ReleaseParams {
            total_records: SAMPLE_TOTAL_RECORDS + 5_000,
            ..SAMPLE_PARAMS
        };
        let (big, _, o_big) = build_structs(&bigger);
        // Same special records prefix (anchors/hidden/temporal/cycle/invalid).
        let special_len = small.iter().filter(|r| r.region != "bulk").count();
        assert_eq!(small[..special_len], big[..special_len]);
        // Oracle special-region answers are identical across scales.
        assert_eq!(o_small.path_anchors, o_big.path_anchors);
        assert_eq!(
            o_small.hidden_intermediary_cases,
            o_big.hidden_intermediary_cases
        );
        assert_eq!(o_small.temporal_cases, o_big.temporal_cases);
        assert_eq!(o_small.cycle_probes, o_big.cycle_probes);
    }

    #[test]
    fn degree_distribution_is_present_and_spread() {
        let o = pkg().manifest.release_oracle.expect("release oracle");
        // A genuine distribution: multiple distinct out-degrees, including hubs.
        assert!(
            o.degree_distribution.len() >= 4,
            "degree distribution too flat"
        );
        assert!(o.degree_distribution.contains_key("0"));
        assert!(
            o.degree_distribution.contains_key("5") || o.degree_distribution.contains_key("8"),
            "expected high-degree hubs"
        );
        // Histogram totals equal the valid-record count.
        let total: usize = o.degree_distribution.values().sum();
        assert_eq!(total, pkg().manifest.counts.valid_records);
    }

    #[test]
    fn cycles_are_present_and_bfs_terminates() {
        let p = pkg();
        let o = p.manifest.release_oracle.clone().expect("release oracle");
        assert!(o.cycle_edges >= 1, "no cycle back edges planted");
        let links = links_of(&p);
        let adj = adjacency(&links);
        // The planted cycle probe: BFS from the entry terminates and reaches
        // exactly the ring nodes within the limit (no infinite loop).
        let probe = &o.cycle_probes[0];
        let reach = bfs(&adj, &probe.source_id, HOP_LIMIT, &|_| true);
        for ring in &probe.reachable_within_limit {
            assert!(reach.contains_key(ring), "ring node {ring} unreachable");
        }
    }

    #[test]
    fn hidden_intermediary_removes_whole_path() {
        let p = pkg();
        let o = p.manifest.release_oracle.clone().expect("release oracle");
        let records = records_of(&p);
        let links = links_of(&p);
        let adj = adjacency(&links);
        let authorized: HashSet<String> = records
            .iter()
            .filter(|r| r.valid && r.authorized)
            .map(|r| r.id.clone())
            .collect();

        let case = &o.hidden_intermediary_cases[0];
        // Ignoring policy the target is reachable at its topological distance.
        let open = bfs(&adj, &case.source_id, HOP_LIMIT, &|_| true);
        assert_eq!(
            open.get(&case.target_id).copied(),
            Some(case.topological_hop_distance)
        );
        assert!(case.reachable_ignoring_policy);
        // With policy, entering the hidden node is forbidden → path omitted.
        let gated = bfs(&adj, &case.source_id, HOP_LIMIT, &|n| {
            authorized.contains(n)
        });
        assert!(
            !gated.contains_key(&case.target_id),
            "hidden path not omitted"
        );
        assert!(!gated.contains_key(&case.hidden_intermediary_id));
        assert!(!case.reachable_with_policy);
    }

    #[test]
    fn temporal_boundaries_resolve_exactly() {
        let o = pkg().manifest.release_oracle.expect("release oracle");
        let by_case: HashMap<&str, &TemporalCase> = o
            .temporal_cases
            .iter()
            .map(|t| (t.case.as_str(), t))
            .collect();
        // Inclusive start boundary is current; exclusive end boundary is not.
        assert!(by_case["valid_from_inclusive_boundary"].current_at_query_instant);
        assert!(!by_case["valid_until_exclusive_boundary"].current_at_query_instant);
        assert!(by_case["open_ended_current"].current_at_query_instant);
        assert!(!by_case["future_not_yet"].current_at_query_instant);
        assert!(!by_case["empty_instant"].current_at_query_instant);
        assert!(!by_case["past_closed"].current_at_query_instant);
        // Every case is backed by a real planted record carrying that interval.
        let records = records_of(&pkg());
        for t in &o.temporal_cases {
            let rec = records
                .iter()
                .find(|r| r.id == t.record_id)
                .expect("temporal record present");
            assert_eq!(rec.temporal_case.as_deref(), Some(t.case.as_str()));
            assert_eq!(rec.valid_from, t.valid_from);
            assert_eq!(rec.valid_until, t.valid_until);
        }
    }

    #[test]
    fn four_hop_classes_present_and_four_hop_unreachable() {
        let p = pkg();
        let o = p.manifest.release_oracle.clone().expect("release oracle");
        // All four hop classes 1/2/3/4 are planted.
        let hops: BTreeSet<u32> = o.path_anchors.iter().map(|a| a.hop_distance).collect();
        assert_eq!(hops, BTreeSet::from([1, 2, 3, 4]));

        let links = links_of(&p);
        let adj = adjacency(&links);
        for anchor in &o.path_anchors {
            let reach = bfs(&adj, &anchor.source_id, HOP_LIMIT, &|_| true);
            if anchor.hop_distance <= HOP_LIMIT {
                assert_eq!(
                    reach.get(&anchor.target_id).copied(),
                    Some(anchor.hop_distance),
                    "hop-{} target should be reachable at exact distance",
                    anchor.hop_distance
                );
                assert!(anchor.reachable_within_limit);
            } else {
                // 4-hop: MUST be unreachable under the ≤3-hop boundedness limit.
                assert!(
                    !reach.contains_key(&anchor.target_id),
                    "hop-4 target must be unreachable within ≤3 hops"
                );
                assert!(!anchor.reachable_within_limit);
            }
        }
    }

    #[test]
    fn invalid_rows_and_links_are_present_and_flagged() {
        let p = pkg();
        let records = records_of(&p);
        let links = links_of(&p);
        let reasons: BTreeSet<String> = records
            .iter()
            .filter(|r| !r.valid)
            .map(|r| r.invalid_reason.clone().unwrap_or_default())
            .collect();
        for expected in [
            "sensitivity_out_of_range",
            "invalid_valid_interval",
            "empty_namespace",
        ] {
            assert!(
                reasons.contains(expected),
                "missing invalid record {expected}"
            );
        }
        assert!(links
            .iter()
            .any(|l| !l.valid && l.invalid_reason.as_deref() == Some("dangling_endpoint")));
        // valid flag and reason presence agree.
        for r in &records {
            assert_eq!(r.valid, r.invalid_reason.is_none());
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
        // IDs are unique across the valid set.
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
        assert!(m.release_oracle.is_some());
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
                r.content.starts_with("synthetic mg-release-v2"),
                "unexpected content: {}",
                r.content
            );
        }
    }

    #[test]
    fn frozen_contract_metadata_is_valid() {
        let c = frozen_contract();
        assert_eq!(c.schema_version, RELEASE_CONTRACT_SCHEMA);
        assert_eq!(c.fixture_id, FIXTURE_ID);
        assert_eq!(c.generator.seed, SEED);
        assert_eq!(c.generator.seed_hex, "0x4D475204");
        // The frozen full-run size is exactly 100k.
        assert_eq!(c.full_total_records, FULL_TOTAL_RECORDS);
        assert_eq!(c.full_counts.total_records, FULL_TOTAL_RECORDS);
        // Full corpus data is deferred, not materialized now.
        assert!(!c.full_data_materialized);
        assert_eq!(c.deferred_to, "F3/F5");
        assert!(!c.membership_hash_method.is_empty());
        assert_eq!(c.full_membership_hash.len(), 64);
        // The frozen oracle marks itself as the full run and keeps the ≤3-hop
        // 4-hop-unreachable contract.
        assert!(c.full_oracle.materialized_full);
        assert_eq!(c.full_oracle.hop_limit, HOP_LIMIT);
        let four = c
            .full_oracle
            .path_anchors
            .iter()
            .find(|a| a.hop_distance == 4)
            .expect("4-hop anchor");
        assert!(!four.reachable_within_limit);
        // Contract serializes/deserializes losslessly.
        let bytes = frozen_contract_bytes(&c);
        let parsed: ReleaseFrozenContract = serde_json::from_slice(&bytes).expect("parses");
        assert_eq!(parsed, c);
        // Two runs freeze the same full membership hash (determinism at scale).
        assert_eq!(
            frozen_contract().full_membership_hash,
            c.full_membership_hash
        );
    }

    #[test]
    fn materializes_slice_and_frozen_contract_to_repo() {
        let root = super::super::generated_root();
        // Commit the cheap deterministic slice.
        let p = build(&SAMPLE_PARAMS);
        let dir = p.materialize(&root).expect("materialize slice");
        for f in ["records.json", "links.json", "fixture-manifest.json"] {
            assert!(dir.join(f).exists(), "missing {f}");
        }
        // Commit the metadata-only frozen contract under the fixture root.
        let contract = frozen_contract();
        let bytes = frozen_contract_bytes(&contract);
        let contract_path = root.join(FIXTURE_ID).join("frozen-contract.json");
        std::fs::write(&contract_path, &bytes).expect("write frozen contract");
        assert!(contract_path.exists());

        // Re-materialization is byte-stable.
        let on_disk = std::fs::read(dir.join("fixture-manifest.json")).unwrap();
        assert_eq!(on_disk, build(&SAMPLE_PARAMS).manifest_bytes());
        assert_eq!(
            std::fs::read(&contract_path).unwrap(),
            frozen_contract_bytes(&frozen_contract())
        );
    }
}
