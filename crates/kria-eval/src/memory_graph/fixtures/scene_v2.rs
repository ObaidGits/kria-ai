//! Shared deterministic builder for the API/UI + fault/rebuild fixtures
//! (`mg-small-v2` seed `0x4D475202`, `mg-medium-v2` seed `0x4D475203`).
//!
//! Both fixtures share the exact same planted-case *shape* required by task
//! 0.2.2 and differ only in **scale** (1,000 vs 10,000 records) and in the
//! backlog/partition/corruption/import/source-cancel counts. This module hosts
//! the single generator so the two concrete generators
//! ([`super::small_v2`], [`super::medium_v2`]) stay thin and cannot drift apart.
//!
//! Every package plants, per `validation.md` §2:
//!
//! * **Seven-destination states** — data exercising every Memory Control Center
//!   destination (Overview/Recall/Knowledge/Timeline/Goals/Sources/Health),
//!   each carrying the full set of rendered UI states
//!   (empty/partial/stale/offline/recovery/ready).
//! * **Long / RTL / CJK labels** — Unicode-robustness labels alongside ASCII.
//! * **Outbox cases** — `derived_outbox` work items (a backlog with mixed
//!   status/attempt/error states).
//! * **Model cases** — embedding/model-version partitions.
//! * **Corruption cases** — sentinels simulating row/manifest integrity failures.
//! * **Import cases** — interchange import candidates (incl. unknown optional
//!   fields).
//! * **Source-cancel cases** — cancellable source ingestion that commits no
//!   partial semantic record.
//!
//! All content is synthetic (no real private data) and every expected answer is
//! defined here — the independent oracle — never derived from a system under
//! test.

use serde::{Deserialize, Serialize};

use super::{
    package_files_and_hash, sha256_hex, ExpectedAnswers, FixtureCounts, FixtureManifest,
    FixturePackage, GeneratorMetadata, LinkKind, MemoryMode, Policy, RecordKind, SceneCoverage,
    SchemaVersions, SplitMix64, TruthState, FIXTURE_MANIFEST_SCHEMA, GENERATOR_VERSION,
};

// ---------------------------------------------------------------------------
// Domain enumerations specific to the API/UI fixtures
// ---------------------------------------------------------------------------

/// The seven Memory Control Center destinations
/// (requirements.md `Memory_Control_Center`; design.md §2 `destinations/*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Destination {
    /// Overview destination.
    Overview,
    /// Recall (retrieval) destination.
    Recall,
    /// Knowledge (graph/list) destination.
    Knowledge,
    /// Timeline (temporal) destination.
    Timeline,
    /// Goals destination.
    Goals,
    /// Sources (ingestion) destination.
    Sources,
    /// Health destination.
    Health,
}

impl Destination {
    /// All seven destinations, in canonical order.
    pub const ALL: [Destination; 7] = [
        Destination::Overview,
        Destination::Recall,
        Destination::Knowledge,
        Destination::Timeline,
        Destination::Goals,
        Destination::Sources,
        Destination::Health,
    ];

    /// Stable snake_case code stored in fixture rows.
    pub fn code(self) -> &'static str {
        match self {
            Destination::Overview => "overview",
            Destination::Recall => "recall",
            Destination::Knowledge => "knowledge",
            Destination::Timeline => "timeline",
            Destination::Goals => "goals",
            Destination::Sources => "sources",
            Destination::Health => "health",
        }
    }
}

/// Unicode label style, exercised for renderer robustness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LabelStyle {
    /// Plain ASCII.
    Ascii,
    /// Very long label (> 256 chars).
    Long,
    /// Right-to-left script (Arabic/Hebrew).
    Rtl,
    /// CJK script (Chinese/Japanese/Korean).
    Cjk,
}

impl LabelStyle {
    /// All label styles, in canonical order.
    pub const ALL: [LabelStyle; 4] = [
        LabelStyle::Ascii,
        LabelStyle::Long,
        LabelStyle::Rtl,
        LabelStyle::Cjk,
    ];

    /// Stable code stored in fixture rows.
    pub fn code(self) -> &'static str {
        match self {
            LabelStyle::Ascii => "ascii",
            LabelStyle::Long => "long",
            LabelStyle::Rtl => "rtl",
            LabelStyle::Cjk => "cjk",
        }
    }
}

/// Rendered UI state a destination section can be in (design.md §16.x
/// "each section owns idle/loading/ready/empty/partial/stale/offline/error").
/// The fixture plants the observable data-bearing states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UiState {
    /// No authorized data for the selection.
    Empty,
    /// Partial/truncated result set.
    Partial,
    /// Stale snapshot pending refresh.
    Stale,
    /// Offline/degraded core.
    Offline,
    /// Recovery-mode disclosure.
    Recovery,
    /// Fully ready.
    Ready,
}

impl UiState {
    /// All UI states, in canonical order.
    pub const ALL: [UiState; 6] = [
        UiState::Empty,
        UiState::Partial,
        UiState::Stale,
        UiState::Offline,
        UiState::Recovery,
        UiState::Ready,
    ];

    /// Stable code stored in fixture rows.
    pub fn code(self) -> &'static str {
        match self {
            UiState::Empty => "empty",
            UiState::Partial => "partial",
            UiState::Stale => "stale",
            UiState::Offline => "offline",
            UiState::Recovery => "recovery",
            UiState::Ready => "ready",
        }
    }
}

/// The kind of record planted in `records.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CaseClass {
    /// A plain destination-state record.
    State,
    /// A retrieval trace (Recall workflow).
    Trace,
    /// A correction record.
    Correction,
}

impl CaseClass {
    /// Stable code stored in fixture rows.
    pub fn code(self) -> &'static str {
        match self {
            CaseClass::State => "state",
            CaseClass::Trace => "trace",
            CaseClass::Correction => "correction",
        }
    }
}

// ---------------------------------------------------------------------------
// Planted row types
// ---------------------------------------------------------------------------

/// One planted destination-state record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneRecord {
    /// Canonical record ID.
    pub id: String,
    /// Record kind code.
    pub record_kind: String,
    /// Owning Memory Control Center destination.
    pub destination: String,
    /// Case class (state/trace/correction).
    pub case_class: String,
    /// Rendered UI state.
    pub ui_state: String,
    /// Truth state code.
    pub truth_state: String,
    /// Memory mode code.
    pub memory_mode: String,
    /// Effective policy tuple.
    pub policy: Policy,
    /// Human-facing label (may be long/RTL/CJK).
    pub label: String,
    /// Unicode label style code.
    pub label_style: String,
    /// SHA-256 of `label`.
    pub content_hash: String,
    /// Whether the row satisfies every schema constraint.
    pub valid: bool,
    /// Reason code when `valid == false`.
    pub invalid_reason: Option<String>,
}

/// One planted semantic link across destination records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneLink {
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
}

/// One `derived_outbox` work item (design.md §4.3 `derived_outbox`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxItem {
    /// Outbox item ID.
    pub id: String,
    /// Projection target (e.g. `vector_index`, `fts5`, `analytics_cache`).
    pub target: String,
    /// Operation (`upsert`/`delete`).
    pub op: String,
    /// Source record kind.
    pub record_kind: String,
    /// Source record ID.
    pub record_id: String,
    /// Content hash bound to the work item.
    pub content_hash: String,
    /// Model partition (only for vector targets).
    pub model_partition: Option<String>,
    /// Authority revision at enqueue time.
    pub authority_revision: u64,
    /// Delivery attempts so far.
    pub attempts: u32,
    /// Status (`pending`/`inflight`/`failed`/`done`).
    pub status: String,
    /// Next attempt time (RFC3339 UTC), if scheduled.
    pub next_attempt_at: Option<String>,
    /// Error code for the last failed attempt.
    pub error_code: Option<String>,
    /// Enqueue time (RFC3339 UTC).
    pub created_at: String,
}

/// One embedding/model partition (design.md §4.3 `embedding_partitions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPartition {
    /// Partition ID.
    pub partition_id: String,
    /// Exact model identity.
    pub model_identity: String,
    /// Model revision.
    pub model_revision: String,
    /// Model weights hash.
    pub model_hash: String,
    /// Embedding dimension (pinned 384).
    pub dim: u32,
    /// Vector dtype (`f32le`).
    pub dtype: String,
    /// Whether vectors are normalized.
    pub normalized: bool,
    /// Tokenizer hash.
    pub tokenizer_hash: String,
    /// Partition status (`active`/`building`/`retired`).
    pub status: String,
    /// Build time (RFC3339 UTC).
    pub build_time: String,
    /// Manifest checksum.
    pub manifest_checksum: String,
}

/// One corruption sentinel simulating an integrity failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorruptionSentinel {
    /// Sentinel ID.
    pub id: String,
    /// What is corrupt (`row`/`manifest`/`index`/`vector_blob`).
    pub target: String,
    /// Failure kind code.
    pub failure_kind: String,
    /// Affected record kind, if applicable.
    pub record_kind: Option<String>,
    /// Affected record ID, if applicable.
    pub record_id: Option<String>,
    /// Expected (correct) hash.
    pub expected_hash: String,
    /// Actual (corrupt) hash observed.
    pub actual_hash: String,
    /// Authority revision where detected.
    pub revision: u64,
    /// Detection time (RFC3339 UTC).
    pub detected_at: String,
}

/// One interchange import candidate (design.md §14 `interchange_imports`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportCandidate {
    /// Candidate ID.
    pub id: String,
    /// Source interchange package ID.
    pub package_id: String,
    /// Package schema version.
    pub schema_version: String,
    /// Candidate record kind.
    pub record_kind: String,
    /// External record ID in the package.
    pub external_id: String,
    /// Proposed label after import.
    pub proposed_label: String,
    /// Package member checksum.
    pub checksum: String,
    /// Import status (`candidate`/`accepted`/`rejected`/`conflict`).
    pub status: String,
    /// Whether the package carried unknown *optional* fields (tolerated).
    pub has_unknown_optional_fields: bool,
    /// Creation time (RFC3339 UTC).
    pub created_at: String,
}

/// One cancellable source ingestion case (design.md §14, §19.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCancellation {
    /// Source ingestion job ID.
    pub id: String,
    /// Source kind (`filesystem`/`repository`/`shell_history`/`library`).
    pub source_kind: String,
    /// Source ID.
    pub source_id: String,
    /// Consent state (`granted`/`pending`/`denied`).
    pub consent_state: String,
    /// Ingest status (`running`/`cancelled`/`completed`).
    pub ingest_status: String,
    /// Resumable cursor, if any.
    pub cursor: Option<String>,
    /// Items durably committed before cancellation.
    pub committed_items: u32,
    /// MUST be false: cancellation commits no partial semantic record.
    pub partial_semantic_record: bool,
    /// Cancellation time (RFC3339 UTC), if cancelled.
    pub cancelled_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Generator parameters
// ---------------------------------------------------------------------------

/// Per-fixture parameters. Only these differ between small and medium.
#[derive(Debug, Clone, Copy)]
pub struct SceneParams {
    /// Fixture ID (e.g. `mg-small-v2`).
    pub fixture_id: &'static str,
    /// Frozen seed from `validation.md`.
    pub seed: u64,
    /// Generator module name recorded in the manifest.
    pub generator_name: &'static str,
    /// Total records in `records.json` (validation.md size contract).
    pub total_records: usize,
    /// Number of planted invalid records (subset of `total_records`).
    pub invalid_records: usize,
    /// `derived_outbox` items to plant.
    pub outbox_items: usize,
    /// Model partitions to plant.
    pub model_partitions: usize,
    /// Corruption sentinels to plant.
    pub corruption_sentinels: usize,
    /// Interchange import candidates to plant.
    pub import_candidates: usize,
    /// Source-cancellation cases to plant.
    pub source_cancellations: usize,
}

// ---------------------------------------------------------------------------
// Fixed synthetic constants
// ---------------------------------------------------------------------------

const NAMESPACES: [&str; 3] = ["personal", "work", "shared"];
const SCOPES: [&str; 3] = ["private", "team", "public"];
const OWNERS: [&str; 2] = ["owner-alpha", "owner-beta"];

const RTL_SAMPLES: [&str; 2] = ["ذاكرة المعرفة", "זיכרון ידע"];
const CJK_SAMPLES: [&str; 3] = ["记忆图谱节点", "知識グラフ", "지식 그래프"];

const T_BUILD: &str = "2024-01-01T00:00:00Z";
const T_EVENT: &str = "2024-06-01T00:00:00Z";

/// Serialize a value to canonical pretty JSON bytes with a trailing newline.
fn to_json_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    let mut s = serde_json::to_string_pretty(value).expect("serializes to JSON");
    s.push('\n');
    s.into_bytes()
}

/// Build a label of the requested style for record index `i`.
fn build_label(style: LabelStyle, dest: Destination, i: usize) -> String {
    match style {
        LabelStyle::Ascii => format!("{} node {i}", dest.code()),
        LabelStyle::Long => {
            // Deterministic long label well over 256 characters.
            let unit = format!("{}-segment-{i:05}-", dest.code());
            unit.repeat(12)
        }
        LabelStyle::Rtl => {
            let s = RTL_SAMPLES[i % RTL_SAMPLES.len()];
            format!("{s} {i}")
        }
        LabelStyle::Cjk => {
            let s = CJK_SAMPLES[i % CJK_SAMPLES.len()];
            format!("{s}{i}")
        }
    }
}

/// Assign a case class: mostly `State`, with Recall→some `Trace` and
/// Knowledge→some `Correction`, so both extra classes are always present.
fn case_class_for(dest: Destination, i: usize) -> CaseClass {
    match dest {
        Destination::Recall if i.is_multiple_of(5) => CaseClass::Trace,
        Destination::Knowledge if i.is_multiple_of(5) => CaseClass::Correction,
        _ => CaseClass::State,
    }
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn build_records(rng: &mut SplitMix64, params: &SceneParams) -> Vec<SceneRecord> {
    let total = params.total_records;
    let valid_count = total - params.invalid_records;
    let mut records = Vec::with_capacity(total);

    for i in 0..valid_count {
        let dest = Destination::ALL[i % Destination::ALL.len()];
        let style = LabelStyle::ALL[i % LabelStyle::ALL.len()];
        let ui_state = UiState::ALL[i % UiState::ALL.len()];
        let case_class = case_class_for(dest, i);
        let truth = TruthState::ALL[i % TruthState::ALL.len()];
        let mode = MemoryMode::ALL[i % MemoryMode::ALL.len()];
        let sensitivity = (i % 4) as i64;
        let label = build_label(style, dest, i);
        let policy = Policy {
            namespace: NAMESPACES[rng.below(NAMESPACES.len())].to_string(),
            owner: OWNERS[rng.below(OWNERS.len())].to_string(),
            scope: SCOPES[rng.below(SCOPES.len())].to_string(),
            sensitivity,
        };
        let id = rng.next_uuid();
        records.push(SceneRecord {
            id,
            record_kind: RecordKind::ALL[i % RecordKind::ALL.len()]
                .code()
                .to_string(),
            destination: dest.code().to_string(),
            case_class: case_class.code().to_string(),
            ui_state: ui_state.code().to_string(),
            truth_state: truth.code().to_string(),
            memory_mode: mode.code().to_string(),
            policy,
            content_hash: sha256_hex(label.as_bytes()),
            label,
            label_style: style.code().to_string(),
            valid: true,
            invalid_reason: None,
        });
    }

    // Planted invalid rows (round-robin over a fixed reason set).
    let reasons = [
        "sensitivity_out_of_range",
        "unknown_truth_state",
        "empty_namespace",
        "unknown_destination",
    ];
    for k in 0..params.invalid_records {
        let reason = reasons[k % reasons.len()];
        let dest = Destination::ALL[k % Destination::ALL.len()];
        let label = format!("invalid {} row {k} :: {reason}", dest.code());
        let id = rng.next_uuid();
        let mut rec = SceneRecord {
            id,
            record_kind: RecordKind::Memory.code().to_string(),
            destination: dest.code().to_string(),
            case_class: CaseClass::State.code().to_string(),
            ui_state: UiState::Ready.code().to_string(),
            truth_state: TruthState::Current.code().to_string(),
            memory_mode: MemoryMode::Permanent.code().to_string(),
            policy: Policy {
                namespace: "work".to_string(),
                owner: "owner-beta".to_string(),
                scope: "team".to_string(),
                sensitivity: 2,
            },
            content_hash: sha256_hex(label.as_bytes()),
            label,
            label_style: LabelStyle::Ascii.code().to_string(),
            valid: false,
            invalid_reason: Some(reason.to_string()),
        };
        match reason {
            "sensitivity_out_of_range" => rec.policy.sensitivity = 4,
            "unknown_truth_state" => rec.truth_state = "Bogus".to_string(),
            "empty_namespace" => rec.policy.namespace = String::new(),
            "unknown_destination" => rec.destination = "widgets".to_string(),
            _ => {}
        }
        records.push(rec);
    }

    debug_assert_eq!(records.len(), total);
    records
}

/// One valid link per canonical link kind, wired between early valid records.
fn build_links(rng: &mut SplitMix64, records: &[SceneRecord]) -> Vec<SceneLink> {
    let valid: Vec<&SceneRecord> = records.iter().filter(|r| r.valid).collect();
    let mut links = Vec::with_capacity(LinkKind::ALL.len());
    for (i, link) in LinkKind::ALL.iter().enumerate() {
        // Distinct endpoints from the valid pool (pool is far larger than 2*5).
        let src = valid[i * 2];
        let tgt = valid[i * 2 + 1];
        links.push(SceneLink {
            id: rng.next_uuid(),
            link_type: link.code().to_string(),
            source_id: src.id.clone(),
            target_id: tgt.id.clone(),
            truth_state: TruthState::Current.code().to_string(),
        });
    }
    links
}

fn build_outbox(rng: &mut SplitMix64, params: &SceneParams) -> Vec<OutboxItem> {
    const TARGETS: [&str; 3] = ["vector_index", "fts5", "analytics_cache"];
    const STATUSES: [&str; 4] = ["pending", "inflight", "failed", "done"];
    let mut items = Vec::with_capacity(params.outbox_items);
    for i in 0..params.outbox_items {
        let target = TARGETS[i % TARGETS.len()];
        let status = STATUSES[i % STATUSES.len()];
        let is_vector = target == "vector_index";
        // A backlog: failed items accumulate attempts, some at the max.
        let attempts = match status {
            "pending" => 0,
            "inflight" => 1,
            "failed" => 3 + (i % 3) as u32,
            _ => 1,
        };
        let content = format!("outbox-{i}");
        items.push(OutboxItem {
            id: rng.next_uuid(),
            target: target.to_string(),
            op: if i.is_multiple_of(4) {
                "delete"
            } else {
                "upsert"
            }
            .to_string(),
            record_kind: RecordKind::ALL[i % RecordKind::ALL.len()]
                .code()
                .to_string(),
            record_id: rng.next_uuid(),
            content_hash: sha256_hex(content.as_bytes()),
            model_partition: is_vector.then(|| "all-MiniLM-L6-v2@r1".to_string()),
            authority_revision: (i as u64) + 1,
            attempts,
            status: status.to_string(),
            next_attempt_at: (status == "pending" || status == "failed")
                .then(|| T_EVENT.to_string()),
            error_code: (status == "failed").then(|| "projection_write_failed".to_string()),
            created_at: T_EVENT.to_string(),
        });
    }
    items
}

fn build_partitions(params: &SceneParams) -> Vec<ModelPartition> {
    const STATUSES: [&str; 3] = ["active", "building", "retired"];
    let mut parts = Vec::with_capacity(params.model_partitions);
    for i in 0..params.model_partitions {
        let revision = format!("r{}", i + 1);
        let content = format!("partition-{i}");
        parts.push(ModelPartition {
            partition_id: format!("all-MiniLM-L6-v2@{revision}"),
            model_identity: "all-MiniLM-L6-v2".to_string(),
            model_revision: revision,
            model_hash: sha256_hex(format!("model-{i}").as_bytes()),
            dim: 384,
            dtype: "f32le".to_string(),
            normalized: true,
            tokenizer_hash: sha256_hex(format!("tokenizer-{i}").as_bytes()),
            status: STATUSES[i % STATUSES.len()].to_string(),
            build_time: T_BUILD.to_string(),
            manifest_checksum: sha256_hex(content.as_bytes()),
        });
    }
    parts
}

fn build_corruption(rng: &mut SplitMix64, params: &SceneParams) -> Vec<CorruptionSentinel> {
    const TARGETS: [&str; 4] = ["row", "manifest", "index", "vector_blob"];
    const KINDS: [&str; 4] = [
        "checksum_mismatch",
        "manifest_membership_drift",
        "dangling_reference",
        "truncated_vector_blob",
    ];
    let mut out = Vec::with_capacity(params.corruption_sentinels);
    for i in 0..params.corruption_sentinels {
        let target = TARGETS[i % TARGETS.len()];
        let has_record = target != "manifest";
        out.push(CorruptionSentinel {
            id: rng.next_uuid(),
            target: target.to_string(),
            failure_kind: KINDS[i % KINDS.len()].to_string(),
            record_kind: has_record.then(|| {
                RecordKind::ALL[i % RecordKind::ALL.len()]
                    .code()
                    .to_string()
            }),
            record_id: has_record.then(|| rng.next_uuid()),
            expected_hash: sha256_hex(format!("expected-{i}").as_bytes()),
            actual_hash: sha256_hex(format!("actual-{i}").as_bytes()),
            revision: (i as u64) + 1,
            detected_at: T_EVENT.to_string(),
        });
    }
    out
}

fn build_imports(rng: &mut SplitMix64, params: &SceneParams) -> Vec<ImportCandidate> {
    const STATUSES: [&str; 4] = ["candidate", "accepted", "rejected", "conflict"];
    let mut out = Vec::with_capacity(params.import_candidates);
    for i in 0..params.import_candidates {
        let content = format!("import-{i}");
        out.push(ImportCandidate {
            id: rng.next_uuid(),
            package_id: format!("mg-interchange-{:03}", i / 8),
            schema_version: "memory-graph-interchange/v2".to_string(),
            record_kind: RecordKind::ALL[i % RecordKind::ALL.len()]
                .code()
                .to_string(),
            external_id: rng.next_uuid(),
            proposed_label: format!("imported node {i}"),
            checksum: sha256_hex(content.as_bytes()),
            status: STATUSES[i % STATUSES.len()].to_string(),
            has_unknown_optional_fields: i.is_multiple_of(3),
            created_at: T_EVENT.to_string(),
        });
    }
    out
}

fn build_source_cancels(rng: &mut SplitMix64, params: &SceneParams) -> Vec<SourceCancellation> {
    const SOURCE_KINDS: [&str; 4] = ["filesystem", "repository", "shell_history", "library"];
    const INGEST: [&str; 3] = ["running", "cancelled", "completed"];
    let mut out = Vec::with_capacity(params.source_cancellations);
    for i in 0..params.source_cancellations {
        let ingest = INGEST[i % INGEST.len()];
        let cancelled = ingest == "cancelled";
        out.push(SourceCancellation {
            id: rng.next_uuid(),
            source_kind: SOURCE_KINDS[i % SOURCE_KINDS.len()].to_string(),
            source_id: rng.next_uuid(),
            consent_state: if i.is_multiple_of(4) {
                "pending"
            } else {
                "granted"
            }
            .to_string(),
            ingest_status: ingest.to_string(),
            cursor: (ingest != "completed").then(|| format!("cursor:{}", (i + 1) * 100)),
            committed_items: ((i as u32) * 7) % 250,
            // Invariant (design §14/§19.6): cancellation commits no partial record.
            partial_semantic_record: false,
            cancelled_at: cancelled.then(|| T_EVENT.to_string()),
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Counts / oracle / coverage
// ---------------------------------------------------------------------------

fn compute_counts(records: &[SceneRecord], links: &[SceneLink]) -> FixtureCounts {
    let mut counts = FixtureCounts {
        total_records: records.len(),
        total_links: links.len(),
        valid_records: 0,
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
        *counts.links_by_kind.entry(l.link_type.clone()).or_insert(0) += 1;
    }
    counts
}

fn compute_expected(records: &[SceneRecord]) -> ExpectedAnswers {
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
        .map(|r| super::InvalidCase {
            id: r.id.clone(),
            reason: r.invalid_reason.clone().unwrap_or_default(),
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

fn compute_coverage(records: &[SceneRecord], params: &SceneParams) -> SceneCoverage {
    let mut cov = SceneCoverage {
        records_by_destination: Default::default(),
        records_by_label_style: Default::default(),
        records_by_ui_state: Default::default(),
        records_by_case_class: Default::default(),
        outbox_items: params.outbox_items,
        model_partitions: params.model_partitions,
        corruption_sentinels: params.corruption_sentinels,
        import_candidates: params.import_candidates,
        source_cancellations: params.source_cancellations,
    };
    for r in records.iter().filter(|r| r.valid) {
        *cov.records_by_destination
            .entry(r.destination.clone())
            .or_insert(0) += 1;
        *cov.records_by_label_style
            .entry(r.label_style.clone())
            .or_insert(0) += 1;
        *cov.records_by_ui_state
            .entry(r.ui_state.clone())
            .or_insert(0) += 1;
        *cov.records_by_case_class
            .entry(r.case_class.clone())
            .or_insert(0) += 1;
    }
    cov
}

// ---------------------------------------------------------------------------
// Top-level build
// ---------------------------------------------------------------------------

/// Deterministically build the full fixture package for `params`.
pub fn build(params: &SceneParams) -> FixturePackage {
    let mut rng = SplitMix64::new(params.seed);

    let records = build_records(&mut rng, params);
    let links = build_links(&mut rng, &records);
    let outbox = build_outbox(&mut rng, params);
    let partitions = build_partitions(params);
    let corruption = build_corruption(&mut rng, params);
    let imports = build_imports(&mut rng, params);
    let source_cancels = build_source_cancels(&mut rng, params);

    let data_files = vec![
        ("records.json".to_string(), to_json_bytes(&records)),
        ("links.json".to_string(), to_json_bytes(&links)),
        ("outbox.json".to_string(), to_json_bytes(&outbox)),
        ("partitions.json".to_string(), to_json_bytes(&partitions)),
        ("corruption.json".to_string(), to_json_bytes(&corruption)),
        ("imports.json".to_string(), to_json_bytes(&imports)),
        ("sources.json".to_string(), to_json_bytes(&source_cancels)),
    ];

    let (files, package_sha256) = package_files_and_hash(&data_files);
    let counts = compute_counts(&records, &links);
    let expected = compute_expected(&records);
    let coverage = compute_coverage(&records, params);

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
        scene_coverage: Some(coverage),
        release_oracle: None,
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

// ---------------------------------------------------------------------------
// Shared test contract (invoked by small_v2 and medium_v2)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::collections::BTreeSet;

    fn read_file<T: for<'de> Deserialize<'de>>(pkg: &FixturePackage, name: &str) -> T {
        let (_, bytes) = pkg
            .data_files
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{name} present"));
        serde_json::from_slice(bytes).unwrap_or_else(|e| panic!("{name} deserializes: {e}"))
    }

    /// Assert every planted-case contract required by task 0.2.2 for a package.
    pub fn assert_scene_contract(params: &SceneParams) {
        let pkg = build(params);

        // -- seed / id metadata ------------------------------------------------
        assert_eq!(pkg.manifest.fixture_id, params.fixture_id);
        assert_eq!(pkg.manifest.generator.seed, params.seed);
        assert_eq!(
            pkg.manifest.generator.seed_hex,
            format!("0x{:08X}", params.seed)
        );

        // -- size contract -----------------------------------------------------
        let records: Vec<SceneRecord> = read_file(&pkg, "records.json");
        assert_eq!(
            records.len(),
            params.total_records,
            "records.json size contract"
        );
        assert_eq!(pkg.manifest.counts.total_records, params.total_records);

        // -- two-run byte & hash determinism ----------------------------------
        let a = build(params);
        let b = build(params);
        assert_eq!(a.all_files(), b.all_files(), "byte-identical across runs");
        assert_eq!(a.manifest.package_sha256, b.manifest.package_sha256);
        assert!(!a.manifest.package_sha256.is_empty());

        // -- seven destinations present ---------------------------------------
        let dests: BTreeSet<&str> = records
            .iter()
            .filter(|r| r.valid)
            .map(|r| r.destination.as_str())
            .collect();
        for d in Destination::ALL {
            assert!(dests.contains(d.code()), "missing destination {}", d.code());
        }
        assert_eq!(dests.len(), 7, "exactly the seven destinations");

        // -- long/RTL/CJK (+ascii) labels present -----------------------------
        let styles: BTreeSet<&str> = records
            .iter()
            .filter(|r| r.valid)
            .map(|r| r.label_style.as_str())
            .collect();
        for s in LabelStyle::ALL {
            assert!(
                styles.contains(s.code()),
                "missing label style {}",
                s.code()
            );
        }
        // A concrete long label really exceeds 256 chars; RTL/CJK carry non-ASCII.
        assert!(
            records
                .iter()
                .any(|r| r.label_style == "long" && r.label.chars().count() > 256),
            "no long label > 256 chars"
        );
        assert!(
            records
                .iter()
                .any(|r| r.label_style == "rtl" && !r.label.is_ascii()),
            "no non-ASCII RTL label"
        );
        assert!(
            records
                .iter()
                .any(|r| r.label_style == "cjk" && !r.label.is_ascii()),
            "no non-ASCII CJK label"
        );

        // -- UI states + traces/corrections present ---------------------------
        let ui: BTreeSet<&str> = records
            .iter()
            .filter(|r| r.valid)
            .map(|r| r.ui_state.as_str())
            .collect();
        for s in UiState::ALL {
            assert!(ui.contains(s.code()), "missing ui state {}", s.code());
        }
        let classes: BTreeSet<&str> = records
            .iter()
            .filter(|r| r.valid)
            .map(|r| r.case_class.as_str())
            .collect();
        for c in [CaseClass::State, CaseClass::Trace, CaseClass::Correction] {
            assert!(
                classes.contains(c.code()),
                "missing case class {}",
                c.code()
            );
        }

        // -- outbox / model / corruption / import / source-cancel present -----
        let outbox: Vec<OutboxItem> = read_file(&pkg, "outbox.json");
        assert_eq!(outbox.len(), params.outbox_items);
        assert!(!outbox.is_empty(), "outbox backlog present");
        assert!(
            outbox
                .iter()
                .any(|o| o.status == "failed" && o.attempts >= 3),
            "outbox backlog must include failed/retried items"
        );
        assert!(
            outbox.iter().any(|o| o.model_partition.is_some()),
            "outbox must reference a model partition"
        );

        let partitions: Vec<ModelPartition> = read_file(&pkg, "partitions.json");
        assert_eq!(partitions.len(), params.model_partitions);
        assert!(!partitions.is_empty(), "model partitions present");
        assert!(partitions
            .iter()
            .all(|p| p.dim == 384 && p.dtype == "f32le"));

        let corruption: Vec<CorruptionSentinel> = read_file(&pkg, "corruption.json");
        assert_eq!(corruption.len(), params.corruption_sentinels);
        assert!(!corruption.is_empty(), "corruption sentinels present");
        assert!(
            corruption.iter().any(|c| c.expected_hash != c.actual_hash),
            "corruption sentinels must diverge from expected hash"
        );

        let imports: Vec<ImportCandidate> = read_file(&pkg, "imports.json");
        assert_eq!(imports.len(), params.import_candidates);
        assert!(!imports.is_empty(), "import candidates present");
        assert!(
            imports.iter().any(|c| c.has_unknown_optional_fields),
            "import cases must include unknown optional fields"
        );

        let sources: Vec<SourceCancellation> = read_file(&pkg, "sources.json");
        assert_eq!(sources.len(), params.source_cancellations);
        assert!(!sources.is_empty(), "source-cancel cases present");
        assert!(
            sources.iter().any(|s| s.ingest_status == "cancelled"),
            "must include a cancelled ingestion"
        );
        // Invariant: cancellation never commits a partial semantic record.
        assert!(sources.iter().all(|s| !s.partial_semantic_record));

        // -- manifest / metadata validity -------------------------------------
        let m = &pkg.manifest;
        assert_eq!(m.schema_version, FIXTURE_MANIFEST_SCHEMA);
        assert_eq!(m.generator.version, GENERATOR_VERSION);
        assert_eq!(m.generator.algorithm, "splitmix64");
        assert_eq!(m.schema_versions.authority_schema, 2);
        assert!(!m.contains_private_data);
        assert_eq!(m.files.len(), pkg.data_files.len());
        for f in &m.files {
            assert_eq!(f.sha256.len(), 64, "sha256 hex length");
            assert!(f.size > 0);
            assert_eq!(f.media_type, "application/json");
        }
        // File checksums match the actual data bytes.
        for (name, bytes) in &pkg.data_files {
            let entry = m.files.iter().find(|f| &f.path == name).expect("entry");
            assert_eq!(entry.sha256, sha256_hex(bytes), "checksum for {name}");
            assert_eq!(entry.size, bytes.len(), "size for {name}");
        }

        // -- scene coverage matches records -----------------------------------
        let cov = m.scene_coverage.as_ref().expect("scene coverage present");
        assert_eq!(cov.records_by_destination.len(), 7);
        assert_eq!(cov.outbox_items, params.outbox_items);
        assert_eq!(cov.model_partitions, params.model_partitions);
        assert_eq!(cov.corruption_sentinels, params.corruption_sentinels);
        assert_eq!(cov.import_candidates, params.import_candidates);
        assert_eq!(cov.source_cancellations, params.source_cancellations);

        // -- membership hash is independent & stable --------------------------
        let mut ids: Vec<String> = records
            .iter()
            .filter(|r| r.valid)
            .map(|r| r.id.clone())
            .collect();
        ids.sort();
        assert_eq!(
            m.expected.membership_hash,
            sha256_hex(ids.join("\n").as_bytes())
        );
        assert_eq!(m.expected.valid_record_ids, ids);
        // ID uniqueness across the valid set.
        assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), ids.len());

        // -- manifest round-trips through JSON --------------------------------
        let bytes = pkg.manifest_bytes();
        let parsed: FixtureManifest = serde_json::from_slice(&bytes).expect("manifest parses");
        assert_eq!(parsed, *m);

        // -- no private-data markers ------------------------------------------
        assert!(!m.contains_private_data);
    }
}
