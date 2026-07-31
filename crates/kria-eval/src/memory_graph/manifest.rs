//! Evidence Artifact `manifest.json` schema and runtime validation for the
//! Memory Graph Production Redesign spec (task F0.4 / 0.4.1).
//!
//! `validation.md` §3 ("Evidence Artifact and Manifest Schema") specifies the
//! exact contents every evidence `manifest.json` must carry and the conditions
//! under which a manifest fails validation. This module renders that contract as
//! strongly-typed Rust/serde structures ([`EvidenceManifest`] and its field
//! types) and provides deterministic, structured *runtime schema validation*
//! ([`EvidenceManifest::validate`]).
//!
//! ## Scope boundary (0.4.1 only)
//!
//! This task defines the **complete** field shape and validates a manifest
//! *value* for schema-level well-formedness. It deliberately does **not**:
//!
//! * collect/stream SHA-256, sizes, or media types, or reject
//!   missing/mutable/duplicate/tampered artifacts on disk — that is **0.4.2**;
//!   here we only check that an artifact reference is *well-formed* (a
//!   repository-relative path that does not escape the repo, or an immutable
//!   URI, plus a syntactically valid checksum field);
//! * wrap or execute commands — that is **0.4.3**;
//! * enforce reviewer independence, sign-off completeness, or non-waivable P0
//!   rules — that is **0.4.4**; here we only define the reviewer/waiver fields
//!   and check that a claimed `Pass` carries at least one review record;
//! * resolve the predecessor/gate promotion chain — that is **0.4.5**; here we
//!   only check that each supplied predecessor hash is syntactically valid.
//!
//! Everything the later tasks need to *consume* is defined here; the runtime
//! checks stay at the schema level.
//!
//! ## Field mapping to `validation.md` §3
//!
//! Every field the spec enumerates has a home:
//!
//! | validation.md phrase | type / field |
//! |---|---|
//! | `schemaVersion` | [`EvidenceManifest::schema_version`] |
//! | `runId`, `gate`, `status`, UTC start/end, actor | scalar fields |
//! | commit, branch, dirty-state digest | [`GitProvenance`] |
//! | exact command/working directory/exit code | [`CommandInvocation`] |
//! | requirement/decision/suite IDs | `requirement_ids` / `decision_ids` / `suite_ids` |
//! | fixture IDs/seeds/generator hashes | [`FixtureRef`] |
//! | authority schema/ontology/model/RRF/scene versions | [`VersionSet`] |
//! | lockfile & binary hashes, OS/kernel/WebKitGTK/runtime/build profile | [`BuildEnvironment`] |
//! | reference-hardware ID (CPU/RAM/GPU/storage/display/DPI) | [`ReferenceHardware`] |
//! | power/thermal/network state, warm/cold protocol | [`EnvironmentState`] |
//! | locale/theme/input/AT | [`Accessibility`] |
//! | artifact `{path,mediaType,sha256,size}` | [`ArtifactReference`] |
//! | assertion totals | [`AssertionTotals`] |
//! | counterexamples | [`Counterexample`] |
//! | metric samples/intervals | [`MetricSeries`] |
//! | reviewer records | [`ReviewRecord`] |
//! | waivers | [`Waiver`] |
//! | predecessor-manifest hashes | `predecessor_hashes` |

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::fixtures::hex_lower;

/// Stable schema identifier embedded in every serialized manifest. Bump the
/// version suffix on any breaking change to the field shape below.
pub const MANIFEST_SCHEMA_VERSION: &str = "memory-graph-evidence-manifest/v1";

/// The five allowed evidence statuses (`validation.md` §1 rule 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RunStatus {
    /// Declared but not yet executed/verified.
    Planned,
    /// Verified pass with linked, commit-specific artifacts.
    Pass,
    /// Verified failure.
    Fail,
    /// Could not run to completion (dependency/environment blocker).
    Blocked,
    /// Not applicable to this gate.
    NotApplicable,
}

/// The seven backend-first gates (`F0`..`F6`). Predecessor/promotion logic is
/// task 0.4.5; this enum only fixes the allowed gate vocabulary so a manifest
/// with a bogus `gate` fails schema validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Gate {
    /// Evidence reset gate.
    F0,
    /// Authority/security/lifecycle/recovery gate.
    F1,
    /// Semantics/truth/entities/sources gate.
    F2,
    /// Retrieval/cognition/API gate.
    F3,
    /// Human Digital Twin / list-first gate.
    F4,
    /// Production release gate.
    F5,
    /// Optional true-3D gate.
    F6,
}

/// Git provenance: commit, branch, and the dirty-state digest.
///
/// `validation.md` §3: a manifest fails validation when "the tree is dirty
/// without a recorded digest". We model that precisely: [`GitProvenance::dirty`]
/// records whether the working tree was dirty, and [`GitProvenance::dirty_digest`]
/// must be present and well-formed whenever it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitProvenance {
    /// Full commit hash (40-hex SHA-1 or 64-hex SHA-256).
    pub commit: String,
    /// Branch name the run executed on.
    pub branch: String,
    /// Whether the working tree had uncommitted changes at run time.
    pub dirty: bool,
    /// Digest of the dirty working tree; required (and validated) iff `dirty`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dirty_digest: Option<String>,
}

/// One exactly-captured command invocation: argv, working directory, exit code.
///
/// Parent-0.4 invariant: "Commands capture cwd/argv/exit code." The command
/// *wrappers* that produce these records are task 0.4.3; this type only fixes
/// the shape the manifest stores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandInvocation {
    /// Catalog command ID (e.g. `CMD-MG-EVAL`); empty when ad hoc.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command_id: String,
    /// Exact argv, element 0 is the program.
    pub argv: Vec<String>,
    /// Working directory the command ran in (repository-relative or absolute).
    pub working_directory: String,
    /// Process exit code.
    pub exit_code: i32,
}

/// A deterministic fixture reference: ID, seed, and generator commit hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureRef {
    /// Fixture ID (e.g. `mg-unit-v2`).
    pub fixture_id: String,
    /// Seed as recorded in the fixture contract (e.g. `0x4D475201`).
    pub seed: String,
    /// Generator commit/version hash that produced the fixture package.
    pub generator_hash: String,
}

/// Authority version set: schema/ontology/model/RRF/scene versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionSet {
    /// Authority (SQLite) schema version.
    pub authority_schema: String,
    /// Ontology/registry version.
    pub ontology: String,
    /// Embedding/LLM model version.
    pub model: String,
    /// Reciprocal-Rank-Fusion profile version.
    pub rrf: String,
    /// Visual scene schema version.
    pub scene: String,
}

/// Build environment: OS/kernel/WebKitGTK/runtime/build profile plus the
/// lockfile and binary hashes that pin the toolchain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildEnvironment {
    /// Operating system name/version (required).
    pub os: Option<String>,
    /// Kernel version (required).
    pub kernel: Option<String>,
    /// WebKitGTK version (optional; only meaningful for GUI runs).
    #[serde(default)]
    pub webkit_gtk: Option<String>,
    /// Language runtime/toolchain version (required).
    pub runtime: Option<String>,
    /// Build profile, e.g. `release`/`debug` (required).
    pub build_profile: Option<String>,
    /// Lockfile digests keyed by lockfile name (e.g. `Cargo.lock`).
    #[serde(default)]
    pub lockfile_hashes: BTreeMap<String, String>,
    /// Produced-binary digests keyed by binary name.
    #[serde(default)]
    pub binary_hashes: BTreeMap<String, String>,
}

/// Reference-hardware identity: `CPU/RAM/GPU/storage/display/DPI`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceHardware {
    /// Stable reference-hardware identifier (required).
    pub hardware_id: Option<String>,
    /// CPU description (required).
    pub cpu: Option<String>,
    /// RAM description (required).
    pub ram: Option<String>,
    /// GPU description (optional; required only for GPU-bound suites).
    #[serde(default)]
    pub gpu: Option<String>,
    /// Storage description (optional).
    #[serde(default)]
    pub storage: Option<String>,
    /// Display description (optional).
    #[serde(default)]
    pub display: Option<String>,
    /// DPI/scale (optional).
    #[serde(default)]
    pub dpi: Option<String>,
}

/// Warm vs cold measurement protocol (`validation.md` §3: "warm/cold protocol").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum MeasurementProtocol {
    /// Warm-cache iterations.
    Warm,
    /// Cold-start iteration.
    Cold,
    /// A run combining warm and cold phases.
    WarmAndCold,
}

/// Runtime environment state: power/thermal/network plus the warm/cold protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentState {
    /// Power/AC state (required).
    pub power_state: Option<String>,
    /// Thermal state (optional).
    #[serde(default)]
    pub thermal_state: Option<String>,
    /// Network state (required).
    pub network_state: Option<String>,
    /// Warm/cold measurement protocol.
    pub protocol: MeasurementProtocol,
}

/// Accessibility environment: `locale/theme/input/AT` (assistive technology).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accessibility {
    /// Locale (required).
    pub locale: Option<String>,
    /// Theme, e.g. light/dark/forced-colors (optional).
    #[serde(default)]
    pub theme: Option<String>,
    /// Input modality (optional).
    #[serde(default)]
    pub input: Option<String>,
    /// Assistive technology in use, e.g. Orca (optional).
    #[serde(default)]
    pub assistive_tech: Option<String>,
}

/// A single artifact reference: `{path, mediaType, sha256, size}`.
///
/// Actual streaming SHA-256 collection, size measurement, and
/// missing/mutable/duplicate/tampered rejection are task 0.4.2. Here we only
/// validate that the reference is *well-formed*: the path is repository-relative
/// and does not escape the repo, or is an immutable URI, and the checksum is a
/// syntactically valid lowercase 64-hex SHA-256.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactReference {
    /// Repository-relative path or immutable URI.
    pub path: String,
    /// IANA media type (e.g. `application/json`).
    pub media_type: String,
    /// Lowercase 64-hex SHA-256 of the artifact bytes.
    pub sha256: String,
    /// Artifact size in bytes.
    pub size: u64,
}

/// Assertion totals for the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssertionTotals {
    /// Total assertions executed.
    pub total: u64,
    /// Assertions that passed.
    pub passed: u64,
    /// Assertions that failed.
    pub failed: u64,
}

/// A persisted property-test counterexample (seed + minimized case).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Counterexample {
    /// Suite the counterexample belongs to.
    pub suite_id: String,
    /// Exact seed that reproduces it.
    pub seed: String,
    /// Minimized counterexample payload.
    pub minimized: String,
}

/// A named metric's samples plus its reported interval (e.g. p95 / 95% CI).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricSeries {
    /// Metric name (e.g. `core_retrieval_p95_ms`).
    pub metric: String,
    /// Raw samples.
    pub samples: Vec<f64>,
    /// Optional `[low, high]` interval bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<[f64; 2]>,
}

/// A reviewer sign-off record.
///
/// Reviewer *independence* and non-waivable sign-off enforcement are task 0.4.4.
/// This type defines the fields (`validation.md` §6: reviewer identity/role, UTC
/// timestamp, manifest hash, reviewed artifact hashes, verdict, signature
/// method) and basic presence is checked in [`EvidenceManifest::validate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRecord {
    /// Reviewer role (e.g. `Security`, `Accessibility`).
    pub role: String,
    /// Reviewer identity.
    pub reviewer_id: String,
    /// UTC timestamp of the review.
    pub timestamp: String,
    /// Hash of the manifest the reviewer signed.
    pub manifest_hash: String,
    /// Hashes of the artifacts the reviewer inspected.
    #[serde(default)]
    pub reviewed_artifact_hashes: Vec<String>,
    /// Review verdict text.
    pub verdict: String,
    /// Whether the reviewer is independent of the implementation author.
    /// (Enforcement is 0.4.4; the field is defined here.)
    #[serde(default)]
    pub independent: bool,
    /// Signature method used.
    pub signature_method: String,
}

/// A waiver record. Non-waivable-P0 enforcement is task 0.4.4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiver {
    /// Waiver identifier.
    pub waiver_id: String,
    /// Scope the waiver applies to.
    pub scope: String,
    /// Justification text.
    pub justification: String,
    /// Optional expiry timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry: Option<String>,
}

/// The complete Evidence Artifact manifest (`manifest.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceManifest {
    /// Stable schema identifier (see [`MANIFEST_SCHEMA_VERSION`]).
    pub schema_version: String,
    /// Run identifier (unique per evidence run root).
    pub run_id: String,
    /// The gate this evidence targets.
    pub gate: Gate,
    /// The evidence status.
    pub status: RunStatus,
    /// UTC run start (RFC 3339).
    pub started_at: String,
    /// UTC run end (RFC 3339).
    pub ended_at: String,
    /// The actor that produced the run.
    pub actor: String,
    /// Git provenance (commit/branch/dirty digest).
    pub git: GitProvenance,
    /// Every command invocation captured for the run.
    pub commands: Vec<CommandInvocation>,
    /// Requirement IDs covered (`MGR-NNN`).
    #[serde(default)]
    pub requirement_ids: Vec<String>,
    /// Decision IDs covered (`MGD-NNN`).
    #[serde(default)]
    pub decision_ids: Vec<String>,
    /// Suite IDs executed (`V-...`).
    #[serde(default)]
    pub suite_ids: Vec<String>,
    /// Fixture references (ID/seed/generator hash).
    #[serde(default)]
    pub fixtures: Vec<FixtureRef>,
    /// Authority schema/ontology/model/RRF/scene versions.
    pub versions: VersionSet,
    /// OS/kernel/WebKitGTK/runtime/build profile plus lockfile/binary hashes.
    pub build_environment: BuildEnvironment,
    /// Reference-hardware identity.
    pub reference_hardware: ReferenceHardware,
    /// Power/thermal/network state and warm/cold protocol.
    pub environment_state: EnvironmentState,
    /// Locale/theme/input/AT.
    pub accessibility: Accessibility,
    /// Artifact references.
    #[serde(default)]
    pub artifacts: Vec<ArtifactReference>,
    /// Assertion totals.
    pub assertions: AssertionTotals,
    /// Persisted counterexamples.
    #[serde(default)]
    pub counterexamples: Vec<Counterexample>,
    /// Metric sample series.
    #[serde(default)]
    pub metrics: Vec<MetricSeries>,
    /// Reviewer records.
    #[serde(default)]
    pub reviews: Vec<ReviewRecord>,
    /// Waiver records.
    #[serde(default)]
    pub waivers: Vec<Waiver>,
    /// Predecessor-manifest hashes (the F0→F6 chain).
    #[serde(default)]
    pub predecessor_hashes: Vec<String>,
}

/// Machine-readable class for a manifest schema-validation defect.
///
/// Each variant maps to a `validation.md` §3 failure clause or a well-formedness
/// rule. [`ManifestDiagnosticKind::code`] gives a stable string for reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestDiagnosticKind {
    /// `schemaVersion` is absent or does not equal [`MANIFEST_SCHEMA_VERSION`].
    BadSchemaVersion,
    /// A required scalar field is empty/blank.
    MissingField,
    /// A required timestamp is not valid RFC 3339.
    BadTimestamp,
    /// The commit digest is absent or not a valid 40/64-hex hash.
    BadCommitDigest,
    /// The working tree is dirty but no (valid) dirty digest was recorded.
    DirtyWithoutDigest,
    /// A governed ID does not match its expected shape.
    MalformedId,
    /// A fixture seed is not a valid hex seed literal.
    MalformedFixtureSeed,
    /// A required environment field is null.
    NullRequiredEnvironment,
    /// An artifact path escapes the repository root (`..` traversal).
    ArtifactPathEscape,
    /// An artifact path is absolute and not an immutable URI.
    ArtifactPathAbsolute,
    /// An artifact checksum field is not a valid lowercase 64-hex SHA-256.
    MalformedArtifactChecksum,
    /// A predecessor-manifest hash is not a valid hash.
    MalformedPredecessorHash,
    /// A claimed `Pass` carries no reviewer records.
    MissingReviews,
    // ---- 0.4.2 on-disk artifact verification kinds ----
    /// A declared on-disk artifact does not exist under the evidence root
    /// (a manifest cannot self-certify a file that is not present).
    ArtifactMissing,
    /// The on-disk artifact size differs from the declared `size`.
    ArtifactSizeMismatch,
    /// The streamed SHA-256 of the on-disk artifact differs from the declared
    /// `sha256` (tamper/corruption).
    ArtifactChecksumMismatch,
    /// The on-disk artifact's media type (by extension) differs from the
    /// declared `mediaType`.
    ArtifactMediaTypeMismatch,
    /// Two artifact references share the same path or the same checksum.
    DuplicateArtifact,
    /// The artifact resolves to a mutable location: a symlink (its target can
    /// be repointed) or a world-writable file, or it does not live under the
    /// immutable evidence run root.
    MutableArtifact,
    /// The artifact could not be read from disk (I/O error other than absence).
    ArtifactReadError,
    /// An artifact declared as an immutable URI is not well-formed.
    MalformedArtifactUri,
    // ---- 0.4.4 reviewer + waiver governance enforcement kinds ----
    /// A claimed `Pass` is missing a reviewer role its gate mandates
    /// (`validation.md` §6 gate→sign-off table); a Pass must carry every
    /// mandatory reviewer role for its gate.
    MissingRequiredReviewer,
    /// A sign-off that requires independence (Security, Accessibility, Visual
    /// Truth, Retrieval-quality, crypto, or license per `validation.md` §6) was
    /// made by a non-independent reviewer (`independent == false` or the
    /// reviewer is the run actor / implementation author).
    NonIndependentReviewer,
    /// A review references a `reviewedArtifactHashes` checksum that is not part
    /// of the manifest's artifact set (a reviewer cannot sign artifacts absent
    /// from the manifest).
    ReviewHashNotInManifest,
    /// A review's `manifestHash` is absent or not a valid lowercase 64-hex
    /// SHA-256.
    MalformedReviewManifestHash,
    /// A review's `signatureMethod` is absent or not an allowed signature
    /// method.
    BadSignatureMethod,
    /// A review's `timestamp` is not a valid RFC 3339 instant expressed in UTC
    /// (`Z` / `+00:00`).
    NonUtcTimestamp,
    /// A waiver's scope matches a non-waivable class — P0 acceptance criteria,
    /// security, privacy/policy leak, integrity/authority corruption, false
    /// erasure, accessibility, license, or an earlier gate (`validation.md` §6:
    /// "a waiver cannot override ...").
    NonWaivableCondition,
    // ---- 0.4.5 predecessor / gate-promotion kinds ----
    /// The immediate predecessor gate (`Fn-1`) has no manifest in the supplied
    /// chain, so gate `Fn` cannot be promoted (`F0` has no predecessor; every
    /// later gate requires its `Fn-1` predecessor). An empty
    /// `predecessorHashes` for a non-`F0` gate also surfaces here.
    PredecessorMissing,
    /// A predecessor gate is present in the chain but has no valid, signed
    /// `Pass` manifest (e.g. it is `Planned`/`NotApplicable`, schema-invalid, or
    /// its governance sign-off is incomplete), so it cannot license promotion.
    PredecessorNotPassed,
    /// A valid signed `Pass` predecessor exists but none of this manifest's
    /// recorded `predecessorHashes` matches its computed manifest hash (the
    /// chain link is unverifiable / points at a different manifest).
    PredecessorHashMismatch,
    /// The `F0→Fn-1` chain has a hole: an intermediate required gate is entirely
    /// absent from the supplied predecessor chain, so the chain is not contiguous.
    GateChainGap,
    /// A predecessor gate carries a `Fail`/`Blocked` manifest (an unresolved
    /// earlier P0/failure); no later-gate polish may mask it, so a later gate
    /// cannot be promoted to `Pass` over it.
    EarlierGateP0Unresolved,
    /// Promotion was attempted from a checklist/checkbox-style claim: the
    /// manifest carries no executed command and/or no checksummed artifact, so
    /// there is no machine evidence to promote (`validation.md` §7: "no Pass
    /// points only to a checklist"). A checked `tasks.md` box is never evidence.
    ChecklistOnlyPromotion,
}

impl ManifestDiagnosticKind {
    /// Stable machine code for reports/annotations.
    pub fn code(self) -> &'static str {
        match self {
            ManifestDiagnosticKind::BadSchemaVersion => "bad_schema_version",
            ManifestDiagnosticKind::MissingField => "missing_field",
            ManifestDiagnosticKind::BadTimestamp => "bad_timestamp",
            ManifestDiagnosticKind::BadCommitDigest => "bad_commit_digest",
            ManifestDiagnosticKind::DirtyWithoutDigest => "dirty_without_digest",
            ManifestDiagnosticKind::MalformedId => "malformed_id",
            ManifestDiagnosticKind::MalformedFixtureSeed => "malformed_fixture_seed",
            ManifestDiagnosticKind::NullRequiredEnvironment => "null_required_environment",
            ManifestDiagnosticKind::ArtifactPathEscape => "artifact_path_escape",
            ManifestDiagnosticKind::ArtifactPathAbsolute => "artifact_path_absolute",
            ManifestDiagnosticKind::MalformedArtifactChecksum => "malformed_artifact_checksum",
            ManifestDiagnosticKind::MalformedPredecessorHash => "malformed_predecessor_hash",
            ManifestDiagnosticKind::MissingReviews => "missing_reviews",
            ManifestDiagnosticKind::ArtifactMissing => "artifact_missing",
            ManifestDiagnosticKind::ArtifactSizeMismatch => "artifact_size_mismatch",
            ManifestDiagnosticKind::ArtifactChecksumMismatch => "artifact_checksum_mismatch",
            ManifestDiagnosticKind::ArtifactMediaTypeMismatch => "artifact_media_type_mismatch",
            ManifestDiagnosticKind::DuplicateArtifact => "duplicate_artifact",
            ManifestDiagnosticKind::MutableArtifact => "mutable_artifact",
            ManifestDiagnosticKind::ArtifactReadError => "artifact_read_error",
            ManifestDiagnosticKind::MalformedArtifactUri => "malformed_artifact_uri",
            ManifestDiagnosticKind::MissingRequiredReviewer => "missing_required_reviewer",
            ManifestDiagnosticKind::NonIndependentReviewer => "non_independent_reviewer",
            ManifestDiagnosticKind::ReviewHashNotInManifest => "review_hash_not_in_manifest",
            ManifestDiagnosticKind::MalformedReviewManifestHash => "malformed_review_manifest_hash",
            ManifestDiagnosticKind::BadSignatureMethod => "bad_signature_method",
            ManifestDiagnosticKind::NonUtcTimestamp => "non_utc_timestamp",
            ManifestDiagnosticKind::NonWaivableCondition => "non_waivable_condition",
            ManifestDiagnosticKind::PredecessorMissing => "predecessor_missing",
            ManifestDiagnosticKind::PredecessorNotPassed => "predecessor_not_passed",
            ManifestDiagnosticKind::PredecessorHashMismatch => "predecessor_hash_mismatch",
            ManifestDiagnosticKind::GateChainGap => "gate_chain_gap",
            ManifestDiagnosticKind::EarlierGateP0Unresolved => "earlier_gate_p0_unresolved",
            ManifestDiagnosticKind::ChecklistOnlyPromotion => "checklist_only_promotion",
        }
    }
}

/// One structured, deterministic manifest-validation diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestDiagnostic {
    /// The defect class.
    pub kind: ManifestDiagnosticKind,
    /// Dotted field path the defect applies to (e.g. `git.commit`).
    pub field: String,
    /// Deterministic human-readable message.
    pub reason: String,
}

impl ManifestDiagnostic {
    /// Construct a diagnostic. Public within the crate so sibling evidence
    /// modules (e.g. the 0.4.5 gate-promotion evaluator) can emit the same
    /// structured, deterministic diagnostics.
    pub fn new(
        kind: ManifestDiagnosticKind,
        field: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        ManifestDiagnostic {
            kind,
            field: field.into(),
            reason: reason.into(),
        }
    }

    /// Deterministic ordering key: `(code, field, reason)`.
    fn sort_key(&self) -> (&'static str, &str, &str) {
        (self.kind.code(), self.field.as_str(), self.reason.as_str())
    }
}

/// The outcome of validating one manifest value against the schema contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestValidation {
    /// Whether the manifest is schema-valid (no diagnostics).
    pub ok: bool,
    /// All diagnostics, sorted by `(code, field, reason)`.
    pub diagnostics: Vec<ManifestDiagnostic>,
}

impl ManifestValidation {
    /// Whether a diagnostic of the given kind is present.
    pub fn has_kind(&self, kind: ManifestDiagnosticKind) -> bool {
        self.diagnostics.iter().any(|d| d.kind == kind)
    }
}

/// Whether `s` is entirely lowercase ASCII hex of exactly `len` characters.
fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Whether `s` is a valid git commit hash (40-hex SHA-1 or 64-hex SHA-256,
/// case-insensitive on the hex digits).
fn is_commit_hash(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether `s` looks like a hex seed literal (`0x` prefix + hex, or bare hex).
fn is_hex_seed(s: &str) -> bool {
    let body = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    !body.is_empty() && body.chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether `id` matches `<PREFIX>-<3 digits>` (used for `MGR-`/`MGD-`).
fn is_numeric_id(id: &str, prefix: &str) -> bool {
    match id.strip_prefix(prefix) {
        Some(rest) => rest.len() == 3 && rest.chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

/// Whether `path` contains a `://` scheme separator (treated as an immutable
/// URI for artifact-location purposes; deep URI validation is out of scope).
fn is_uri(path: &str) -> bool {
    if let Some(idx) = path.find("://") {
        let scheme = &path[..idx];
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '.' || c == '-')
    } else {
        false
    }
}

/// Classify a repository-relative artifact path for escape/absoluteness.
/// Returns `Some(kind)` when the path is defective.
fn classify_relative_path(path: &str) -> Option<ManifestDiagnosticKind> {
    // Absolute POSIX path or Windows drive path is not repository-relative.
    if path.starts_with('/') || path.starts_with('\\') {
        return Some(ManifestDiagnosticKind::ArtifactPathAbsolute);
    }
    if path.len() >= 2 && path.as_bytes()[1] == b':' && path.as_bytes()[0].is_ascii_alphabetic() {
        return Some(ManifestDiagnosticKind::ArtifactPathAbsolute);
    }
    // Walk components; a `..` that pops past the root escapes the repository.
    let mut depth: i32 = 0;
    for comp in path.split(['/', '\\']) {
        match comp {
            "" | "." => continue,
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return Some(ManifestDiagnosticKind::ArtifactPathEscape);
                }
            }
            _ => depth += 1,
        }
    }
    None
}

impl EvidenceManifest {
    /// The schema version this build validates against.
    pub const SCHEMA_VERSION: &'static str = MANIFEST_SCHEMA_VERSION;

    /// Parse a manifest from a JSON string.
    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    /// Serialize to stable, pretty-printed JSON.
    pub fn to_json_pretty(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    /// Serialize to compact JSON.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    /// Runtime-validate this manifest value against the schema contract.
    ///
    /// Returns a deterministic [`ManifestValidation`]: diagnostics are sorted by
    /// `(code, field, reason)` so the result is byte-stable and order-independent
    /// of the field-scan order.
    pub fn validate(&self) -> ManifestValidation {
        let mut d: Vec<ManifestDiagnostic> = Vec::new();

        // schemaVersion — must equal the pinned constant.
        if self.schema_version != MANIFEST_SCHEMA_VERSION {
            d.push(ManifestDiagnostic::new(
                ManifestDiagnosticKind::BadSchemaVersion,
                "schemaVersion",
                format!(
                    "expected '{MANIFEST_SCHEMA_VERSION}', found '{}'",
                    self.schema_version
                ),
            ));
        }

        // Required scalar fields must be non-blank.
        require_nonblank(&mut d, "runId", &self.run_id);
        require_nonblank(&mut d, "actor", &self.actor);

        // UTC start/end must be valid RFC 3339 timestamps.
        require_timestamp(&mut d, "startedAt", &self.started_at);
        require_timestamp(&mut d, "endedAt", &self.ended_at);

        // Git provenance.
        if self.git.commit.trim().is_empty() {
            d.push(ManifestDiagnostic::new(
                ManifestDiagnosticKind::BadCommitDigest,
                "git.commit",
                "commit digest is absent",
            ));
        } else if !is_commit_hash(self.git.commit.trim()) {
            d.push(ManifestDiagnostic::new(
                ManifestDiagnosticKind::BadCommitDigest,
                "git.commit",
                "commit digest is not a 40- or 64-hex hash",
            ));
        }
        require_nonblank(&mut d, "git.branch", &self.git.branch);
        if self.git.dirty {
            match self.git.dirty_digest.as_deref() {
                None => d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::DirtyWithoutDigest,
                    "git.dirtyDigest",
                    "working tree is dirty but no dirty-state digest was recorded",
                )),
                Some(digest) if digest.trim().is_empty() || !is_commit_hash(digest.trim()) => d
                    .push(ManifestDiagnostic::new(
                        ManifestDiagnosticKind::DirtyWithoutDigest,
                        "git.dirtyDigest",
                        "dirty tree recorded a blank or malformed dirty-state digest",
                    )),
                Some(_) => {}
            }
        }

        // Commands: argv non-empty, working directory present.
        for (i, cmd) in self.commands.iter().enumerate() {
            if cmd.argv.is_empty() || cmd.argv.iter().all(|a| a.trim().is_empty()) {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::MissingField,
                    format!("commands[{i}].argv"),
                    "command records an empty argv",
                ));
            }
            require_nonblank(
                &mut d,
                &format!("commands[{i}].workingDirectory"),
                &cmd.working_directory,
            );
        }

        // Governed ID shapes.
        for (i, id) in self.requirement_ids.iter().enumerate() {
            if !is_numeric_id(id, "MGR-") {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::MalformedId,
                    format!("requirementIds[{i}]"),
                    format!("'{id}' is not a valid MGR-NNN requirement ID"),
                ));
            }
        }
        for (i, id) in self.decision_ids.iter().enumerate() {
            if !is_numeric_id(id, "MGD-") {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::MalformedId,
                    format!("decisionIds[{i}]"),
                    format!("'{id}' is not a valid MGD-NNN decision ID"),
                ));
            }
        }
        for (i, id) in self.suite_ids.iter().enumerate() {
            if !id.starts_with("V-") || id.len() < 4 {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::MalformedId,
                    format!("suiteIds[{i}]"),
                    format!("'{id}' is not a valid V-* suite ID"),
                ));
            }
        }

        // Fixtures: ID shape + hex seed + generator hash present.
        for (i, fx) in self.fixtures.iter().enumerate() {
            if !(fx.fixture_id.starts_with("mg-") && fx.fixture_id.ends_with("-v2")) {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::MalformedId,
                    format!("fixtures[{i}].fixtureId"),
                    format!("'{}' is not a valid mg-*-v2 fixture ID", fx.fixture_id),
                ));
            }
            if !is_hex_seed(&fx.seed) {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::MalformedFixtureSeed,
                    format!("fixtures[{i}].seed"),
                    format!("'{}' is not a valid hex seed literal", fx.seed),
                ));
            }
            require_nonblank(
                &mut d,
                &format!("fixtures[{i}].generatorHash"),
                &fx.generator_hash,
            );
        }

        // Authority versions must all be present.
        require_nonblank(
            &mut d,
            "versions.authoritySchema",
            &self.versions.authority_schema,
        );
        require_nonblank(&mut d, "versions.ontology", &self.versions.ontology);
        require_nonblank(&mut d, "versions.model", &self.versions.model);
        require_nonblank(&mut d, "versions.rrf", &self.versions.rrf);
        require_nonblank(&mut d, "versions.scene", &self.versions.scene);

        // Required environment fields must not be null.
        let env = &self.build_environment;
        require_env(&mut d, "buildEnvironment.os", &env.os);
        require_env(&mut d, "buildEnvironment.kernel", &env.kernel);
        require_env(&mut d, "buildEnvironment.runtime", &env.runtime);
        require_env(&mut d, "buildEnvironment.buildProfile", &env.build_profile);

        let hw = &self.reference_hardware;
        require_env(&mut d, "referenceHardware.hardwareId", &hw.hardware_id);
        require_env(&mut d, "referenceHardware.cpu", &hw.cpu);
        require_env(&mut d, "referenceHardware.ram", &hw.ram);

        let state = &self.environment_state;
        require_env(&mut d, "environmentState.powerState", &state.power_state);
        require_env(
            &mut d,
            "environmentState.networkState",
            &state.network_state,
        );

        require_env(&mut d, "accessibility.locale", &self.accessibility.locale);

        // Artifact references: well-formed location + checksum.
        for (i, art) in self.artifacts.iter().enumerate() {
            let field = format!("artifacts[{i}].path");
            if art.path.trim().is_empty() {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::MissingField,
                    field.clone(),
                    "artifact path is empty",
                ));
            } else if !is_uri(&art.path) {
                if let Some(kind) = classify_relative_path(&art.path) {
                    let reason = match kind {
                        ManifestDiagnosticKind::ArtifactPathEscape => {
                            "artifact path escapes the repository root"
                        }
                        _ => "artifact path is absolute and not an immutable URI",
                    };
                    d.push(ManifestDiagnostic::new(kind, field.clone(), reason));
                }
            }
            require_nonblank(
                &mut d,
                &format!("artifacts[{i}].mediaType"),
                &art.media_type,
            );
            if !is_lower_hex(&art.sha256, 64) {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::MalformedArtifactChecksum,
                    format!("artifacts[{i}].sha256"),
                    "sha256 is not a lowercase 64-hex digest",
                ));
            }
        }

        // Predecessor-manifest hashes must each be valid hashes.
        for (i, h) in self.predecessor_hashes.iter().enumerate() {
            if !is_lower_hex(h, 64) {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::MalformedPredecessorHash,
                    format!("predecessorHashes[{i}]"),
                    "predecessor hash is not a lowercase 64-hex SHA-256",
                ));
            }
        }

        // A claimed Pass must carry at least one reviewer record. (Independence
        // and role-completeness enforcement is task 0.4.4.)
        if self.status == RunStatus::Pass && self.reviews.is_empty() {
            d.push(ManifestDiagnostic::new(
                ManifestDiagnosticKind::MissingReviews,
                "reviews",
                "a Pass manifest must carry at least one reviewer record",
            ));
        }
        // Reviewer records that do exist must carry their identifying fields.
        for (i, rev) in self.reviews.iter().enumerate() {
            require_nonblank(&mut d, &format!("reviews[{i}].role"), &rev.role);
            require_nonblank(
                &mut d,
                &format!("reviews[{i}].reviewerId"),
                &rev.reviewer_id,
            );
            require_nonblank(
                &mut d,
                &format!("reviews[{i}].signatureMethod"),
                &rev.signature_method,
            );
        }

        // Waivers that exist must carry an ID and justification.
        for (i, w) in self.waivers.iter().enumerate() {
            require_nonblank(&mut d, &format!("waivers[{i}].waiverId"), &w.waiver_id);
            require_nonblank(
                &mut d,
                &format!("waivers[{i}].justification"),
                &w.justification,
            );
        }

        d.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        ManifestValidation {
            ok: d.is_empty(),
            diagnostics: d,
        }
    }

    /// On-disk artifact verification (task 0.4.2).
    ///
    /// Complements the pure schema [`EvidenceManifest::validate`] with the I/O
    /// layer `validation.md` §3 requires: for every [`ArtifactReference`] it
    /// streams the on-disk bytes to compute a SHA-256 (in bounded chunks, never
    /// loading a whole file into memory), measures the on-disk size, and checks
    /// the media type by extension against the declared values, rejecting:
    ///
    /// * **missing** files — a manifest cannot self-certify a file absent on
    ///   disk ([`ManifestDiagnosticKind::ArtifactMissing`]);
    /// * **escaping** references — a repository-relative path that traverses
    ///   above `root`, lexically or via symlink resolution
    ///   ([`ManifestDiagnosticKind::ArtifactPathEscape`] /
    ///   [`ManifestDiagnosticKind::ArtifactPathAbsolute`]);
    /// * **mutable** artifacts — symlinks (a target can be repointed),
    ///   world-writable files, or files resolving outside the immutable
    ///   evidence run root ([`ManifestDiagnosticKind::MutableArtifact`]);
    /// * **duplicate** references — the same path or the same checksum listed
    ///   more than once ([`ManifestDiagnosticKind::DuplicateArtifact`]);
    /// * **checksum/size-invalid** artifacts — a computed SHA-256 or on-disk
    ///   size that differs from the declared value
    ///   ([`ManifestDiagnosticKind::ArtifactChecksumMismatch`] /
    ///   [`ManifestDiagnosticKind::ArtifactSizeMismatch`]).
    ///
    /// References that are **immutable URIs** (contain `://`) are verified for
    /// well-formedness only ([`ManifestDiagnosticKind::MalformedArtifactUri`])
    /// and skip all filesystem checks — their bytes are verified out of band.
    ///
    /// `root` is the evidence run root the repository-relative paths resolve
    /// against. The returned [`ManifestValidation`] carries deterministic
    /// diagnostics sorted by `(code, field, reason)`.
    pub fn verify_artifacts(&self, root: impl AsRef<Path>) -> ManifestValidation {
        let root = root.as_ref();
        let mut d: Vec<ManifestDiagnostic> = Vec::new();

        // Canonicalize the root once for on-disk escape detection; fall back to
        // the supplied path when it cannot be canonicalized (e.g. missing).
        let root_canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());

        // Duplicate tracking (path + checksum), per the validation.md dedup rule.
        let mut seen_paths: BTreeSet<String> = BTreeSet::new();
        let mut seen_checksums: BTreeSet<String> = BTreeSet::new();

        for (i, art) in self.artifacts.iter().enumerate() {
            let path_field = format!("artifacts[{i}].path");
            let raw = art.path.trim();

            // Duplicate detection runs regardless of on-disk vs URI location.
            if !raw.is_empty() && !seen_paths.insert(raw.to_string()) {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::DuplicateArtifact,
                    path_field.clone(),
                    format!("artifact path '{raw}' is listed more than once"),
                ));
            }
            let checksum = art.sha256.trim().to_ascii_lowercase();
            if !checksum.is_empty() && !seen_checksums.insert(checksum.clone()) {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::DuplicateArtifact,
                    format!("artifacts[{i}].sha256"),
                    format!("artifact checksum '{checksum}' is listed more than once"),
                ));
            }

            if raw.is_empty() {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::ArtifactMissing,
                    path_field,
                    "artifact path is empty; nothing to verify on disk",
                ));
                continue;
            }

            // Immutable URIs: verify well-formedness only, skip filesystem I/O.
            if raw.contains("://") {
                if !is_uri(raw) {
                    d.push(ManifestDiagnostic::new(
                        ManifestDiagnosticKind::MalformedArtifactUri,
                        path_field,
                        format!("'{raw}' is not a well-formed immutable URI"),
                    ));
                }
                continue;
            }

            // Repository-relative path: reject lexical escape/absoluteness first.
            if let Some(kind) = classify_relative_path(raw) {
                let reason = match kind {
                    ManifestDiagnosticKind::ArtifactPathEscape => {
                        "artifact path escapes the evidence root"
                    }
                    _ => "artifact path is absolute and not an immutable URI",
                };
                d.push(ManifestDiagnostic::new(kind, path_field, reason));
                continue;
            }

            let full = root.join(raw);

            // Existence + link classification via a non-following stat.
            let link_meta = match std::fs::symlink_metadata(&full) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    d.push(ManifestDiagnostic::new(
                        ManifestDiagnosticKind::ArtifactMissing,
                        path_field,
                        format!("declared artifact '{raw}' does not exist under the evidence root"),
                    ));
                    continue;
                }
                Err(e) => {
                    d.push(ManifestDiagnostic::new(
                        ManifestDiagnosticKind::ArtifactReadError,
                        path_field,
                        format!("cannot stat artifact '{raw}': {e}"),
                    ));
                    continue;
                }
            };

            // Symlinks are mutable: the target can be repointed after recording.
            if link_meta.file_type().is_symlink() {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::MutableArtifact,
                    path_field,
                    format!(
                        "artifact '{raw}' is a symlink; evidence must be an immutable regular file"
                    ),
                ));
                continue;
            }

            // Resolve symlink-free real path and confirm it stays under root.
            match std::fs::canonicalize(&full) {
                Ok(real) if !real.starts_with(&root_canon) => {
                    d.push(ManifestDiagnostic::new(
                        ManifestDiagnosticKind::ArtifactPathEscape,
                        path_field,
                        format!("artifact '{raw}' resolves outside the evidence root"),
                    ));
                    continue;
                }
                Ok(_) => {}
                Err(e) => {
                    d.push(ManifestDiagnostic::new(
                        ManifestDiagnosticKind::ArtifactReadError,
                        path_field,
                        format!("cannot resolve artifact '{raw}': {e}"),
                    ));
                    continue;
                }
            }

            // World-writable regular files are mutable evidence.
            if is_world_writable(&link_meta) {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::MutableArtifact,
                    path_field.clone(),
                    format!("artifact '{raw}' is world-writable; evidence must be immutable"),
                ));
                // Continue verifying checksum/size/media so all defects surface.
            }

            // Media type by extension (only when the extension is recognized).
            if let Some(accepted) = expected_media_types(raw) {
                let declared = art
                    .media_type
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                if !accepted.iter().any(|t| *t == declared) {
                    d.push(ManifestDiagnostic::new(
                        ManifestDiagnosticKind::ArtifactMediaTypeMismatch,
                        format!("artifacts[{i}].mediaType"),
                        format!(
                            "declared media type '{}' does not match expected {:?} for this extension",
                            art.media_type, accepted
                        ),
                    ));
                }
            }

            // Streaming SHA-256 + size in bounded chunks.
            match stream_sha256_and_size(&full) {
                Ok((digest, size)) => {
                    if size != art.size {
                        d.push(ManifestDiagnostic::new(
                            ManifestDiagnosticKind::ArtifactSizeMismatch,
                            format!("artifacts[{i}].size"),
                            format!("declared size {} but on-disk size is {size}", art.size),
                        ));
                    }
                    if digest != checksum {
                        d.push(ManifestDiagnostic::new(
                            ManifestDiagnosticKind::ArtifactChecksumMismatch,
                            format!("artifacts[{i}].sha256"),
                            format!(
                                "declared sha256 '{}' but on-disk digest is '{digest}'",
                                art.sha256
                            ),
                        ));
                    }
                }
                Err(e) => {
                    d.push(ManifestDiagnostic::new(
                        ManifestDiagnosticKind::ArtifactReadError,
                        path_field,
                        format!("cannot read artifact '{raw}': {e}"),
                    ));
                }
            }
        }

        d.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        ManifestValidation {
            ok: d.is_empty(),
            diagnostics: d,
        }
    }

    /// Reviewer + waiver governance enforcement (task 0.4.4).
    ///
    /// Complements the schema-level [`EvidenceManifest::validate`] (which only
    /// checks that a `Pass` carries *some* review and that present records carry
    /// their identifying fields) with the substantive governance `validation.md`
    /// §6 ("Phase Gates and Required Sign-off") requires. It enforces, all as
    /// deterministic structured diagnostics sorted by `(code, field, reason)`:
    ///
    /// * **Reviewer role** — a claimed `Pass` must carry every mandatory
    ///   reviewer role its gate demands (see [`required_reviewer_roles`]); a
    ///   missing role yields [`ManifestDiagnosticKind::MissingRequiredReviewer`].
    ///   (Non-`Pass` statuses declare no completion, so role completeness is not
    ///   forced on them.)
    /// * **Independence** — a sign-off whose role requires independence
    ///   (Security, Accessibility, Visual Truth, Retrieval-quality, crypto, or
    ///   license) must be `independent == true` *and* be made by someone other
    ///   than the run [`actor`](EvidenceManifest::actor) (the implementation
    ///   author). A violation yields
    ///   [`ManifestDiagnosticKind::NonIndependentReviewer`].
    /// * **Reviewed hashes** — every `manifestHash` must be a well-formed
    ///   lowercase 64-hex SHA-256
    ///   ([`ManifestDiagnosticKind::MalformedReviewManifestHash`]), and every
    ///   `reviewedArtifactHashes` entry must reference a checksum that actually
    ///   appears in the manifest's artifact set — a reviewer cannot sign
    ///   artifacts absent from the manifest
    ///   ([`ManifestDiagnosticKind::ReviewHashNotInManifest`]).
    /// * **Signature method + UTC timestamp** — `signatureMethod` must be a
    ///   recognized method ([`ManifestDiagnosticKind::BadSignatureMethod`]) and
    ///   `timestamp` must be a valid RFC 3339 instant expressed in UTC
    ///   ([`ManifestDiagnosticKind::NonUtcTimestamp`]).
    /// * **Non-waivable conditions** — a waiver whose scope matches a
    ///   non-waivable class (P0 acceptance criteria, security, privacy/policy
    ///   leak, integrity/authority corruption, false erasure, accessibility,
    ///   license, or an earlier gate) is rejected with
    ///   [`ManifestDiagnosticKind::NonWaivableCondition`]; a `Pass` that carries
    ///   such a waiver therefore fails.
    ///
    /// This method leaves [`EvidenceManifest::validate`] and
    /// [`EvidenceManifest::verify_artifacts`] untouched; callers compose the
    /// three as needed.
    pub fn enforce_governance(&self) -> ManifestValidation {
        let mut d: Vec<ManifestDiagnostic> = Vec::new();

        // The set of artifact checksums a reviewer is allowed to reference.
        let artifact_hashes: BTreeSet<String> = self
            .artifacts
            .iter()
            .map(|a| a.sha256.trim().to_ascii_lowercase())
            .collect();

        // ---- Reviewer role completeness (Pass only) ----
        if self.status == RunStatus::Pass {
            for required in required_reviewer_roles(self.gate) {
                let covered = self
                    .reviews
                    .iter()
                    .any(|rev| role_matches(required, &rev.role));
                if !covered {
                    d.push(ManifestDiagnostic::new(
                        ManifestDiagnosticKind::MissingRequiredReviewer,
                        "reviews",
                        format!(
                            "gate {:?} Pass is missing a mandatory '{required}' reviewer sign-off",
                            self.gate
                        ),
                    ));
                }
            }
        }

        // ---- Per-review governance checks ----
        for (i, rev) in self.reviews.iter().enumerate() {
            // Independence: sign-offs on independence-required roles must be
            // independent and not authored by the run actor.
            if role_requires_independence(&rev.role) {
                let reviewer = rev.reviewer_id.trim();
                let is_author = !reviewer.is_empty() && reviewer == self.actor.trim();
                if !rev.independent || is_author {
                    let why = if is_author {
                        "the run actor / implementation author cannot be the sole approver for this role"
                    } else {
                        "reviewer is not marked independent"
                    };
                    d.push(ManifestDiagnostic::new(
                        ManifestDiagnosticKind::NonIndependentReviewer,
                        format!("reviews[{i}].independent"),
                        format!(
                            "role '{}' requires an independent sign-off: {why}",
                            rev.role
                        ),
                    ));
                }
            }

            // Manifest hash must be a valid lowercase 64-hex SHA-256.
            if !is_lower_hex(rev.manifest_hash.trim(), 64) {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::MalformedReviewManifestHash,
                    format!("reviews[{i}].manifestHash"),
                    "review manifest hash is not a lowercase 64-hex SHA-256",
                ));
            }

            // Every reviewed artifact hash must be present in the manifest.
            for (j, h) in rev.reviewed_artifact_hashes.iter().enumerate() {
                let norm = h.trim().to_ascii_lowercase();
                if !artifact_hashes.contains(&norm) {
                    d.push(ManifestDiagnostic::new(
                        ManifestDiagnosticKind::ReviewHashNotInManifest,
                        format!("reviews[{i}].reviewedArtifactHashes[{j}]"),
                        format!(
                            "reviewed artifact hash '{h}' is not present in the manifest artifact set"
                        ),
                    ));
                }
            }

            // Signature method must be recognized.
            if !is_allowed_signature_method(&rev.signature_method) {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::BadSignatureMethod,
                    format!("reviews[{i}].signatureMethod"),
                    format!(
                        "signature method '{}' is not an allowed method {:?}",
                        rev.signature_method, ALLOWED_SIGNATURE_METHODS
                    ),
                ));
            }

            // Timestamp must be a valid RFC 3339 instant in UTC.
            if !is_utc_rfc3339(&rev.timestamp) {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::NonUtcTimestamp,
                    format!("reviews[{i}].timestamp"),
                    "review timestamp is not a valid RFC 3339 UTC instant",
                ));
            }
        }

        // ---- Non-waivable waiver scopes ----
        for (i, w) in self.waivers.iter().enumerate() {
            if let Some(class) = non_waivable_class(&w.scope) {
                d.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::NonWaivableCondition,
                    format!("waivers[{i}].scope"),
                    format!(
                        "waiver '{}' targets non-waivable class '{class}'; scope '{}' cannot be waived",
                        w.waiver_id, w.scope
                    ),
                ));
            }
        }

        d.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        ManifestValidation {
            ok: d.is_empty(),
            diagnostics: d,
        }
    }
}

/// Bounded read chunk for streaming hashing (64 KiB): artifacts are hashed in
/// fixed-size chunks so arbitrarily large files never load fully into memory.
const ARTIFACT_CHUNK_BYTES: usize = 64 * 1024;

/// Stream a file in [`ARTIFACT_CHUNK_BYTES`] chunks, returning its lowercase
/// hex SHA-256 and its exact byte length without buffering the whole file.
fn stream_sha256_and_size(path: &Path) -> std::io::Result<(String, u64)> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; ARTIFACT_CHUNK_BYTES];
    let mut total: u64 = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hex_lower(&hasher.finalize()), total))
}

/// Whether `meta` describes a world-writable file (Unix `o+w`). On non-Unix
/// targets this policy check is a no-op (returns `false`).
#[cfg(unix)]
fn is_world_writable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o002 != 0
}

#[cfg(not(unix))]
fn is_world_writable(_meta: &std::fs::Metadata) -> bool {
    false
}

/// The acceptable IANA media types for a path's file extension, or `None` when
/// the extension is unknown (media type then cannot be verified from disk).
fn expected_media_types(path: &str) -> Option<&'static [&'static str]> {
    let file_name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let (_, ext) = file_name.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    let accepted: &'static [&'static str] = match ext.as_str() {
        "json" => &["application/json"],
        "jsonl" | "ndjson" => &["application/jsonl", "application/x-ndjson"],
        "xml" => &["application/xml", "text/xml"],
        "png" => &["image/png"],
        "svg" => &["image/svg+xml"],
        "md" => &["text/markdown"],
        "txt" | "log" => &["text/plain"],
        "html" | "htm" => &["text/html"],
        "csv" => &["text/csv"],
        "yaml" | "yml" => &["application/yaml", "text/yaml"],
        _ => return None,
    };
    Some(accepted)
}

/// Push a [`ManifestDiagnosticKind::MissingField`] when `value` is blank.
fn require_nonblank(d: &mut Vec<ManifestDiagnostic>, field: &str, value: &str) {
    if value.trim().is_empty() {
        d.push(ManifestDiagnostic::new(
            ManifestDiagnosticKind::MissingField,
            field,
            "required field is empty",
        ));
    }
}

/// Push a [`ManifestDiagnosticKind::NullRequiredEnvironment`] when `value` is
/// `None` or blank.
fn require_env(d: &mut Vec<ManifestDiagnostic>, field: &str, value: &Option<String>) {
    match value {
        None => d.push(ManifestDiagnostic::new(
            ManifestDiagnosticKind::NullRequiredEnvironment,
            field,
            "required environment field is null",
        )),
        Some(v) if v.trim().is_empty() => d.push(ManifestDiagnostic::new(
            ManifestDiagnosticKind::NullRequiredEnvironment,
            field,
            "required environment field is empty",
        )),
        Some(_) => {}
    }
}

/// Push a [`ManifestDiagnosticKind::BadTimestamp`] when `value` is not RFC 3339.
fn require_timestamp(d: &mut Vec<ManifestDiagnostic>, field: &str, value: &str) {
    if value.trim().is_empty() {
        d.push(ManifestDiagnostic::new(
            ManifestDiagnosticKind::MissingField,
            field,
            "required timestamp is empty",
        ));
    } else if chrono::DateTime::parse_from_rfc3339(value.trim()).is_err() {
        d.push(ManifestDiagnostic::new(
            ManifestDiagnosticKind::BadTimestamp,
            field,
            "timestamp is not a valid RFC 3339 UTC instant",
        ));
    }
}

/// Allowed reviewer signature methods (`validation.md` §6 requires a recorded
/// signature method; this fixes the accepted vocabulary). Matched
/// case-insensitively in [`is_allowed_signature_method`].
const ALLOWED_SIGNATURE_METHODS: &[&str] = &[
    "gpg", "pgp", "sigstore", "cosign", "x509", "ssh", "minisign",
];

/// The mandatory reviewer roles for a gate's `Pass`, transcribed from the
/// `validation.md` §6 gate→sign-off table. `F5` is "Release owner plus every
/// prior mandatory role", so it is the union of every `F0`..`F4` role plus the
/// Release owner.
fn required_reviewer_roles(gate: Gate) -> &'static [&'static str] {
    match gate {
        Gate::F0 => &["Spec owner", "QA/evidence owner"],
        Gate::F1 => &["Backend", "Security/Privacy", "Data Integrity"],
        Gate::F2 => &["Domain", "Security/Privacy"],
        Gate::F3 => &["Retrieval", "Cognition", "API", "Security"],
        Gate::F4 => &["Product/UX", "Accessibility", "Visual Truth", "Frontend"],
        Gate::F5 => &[
            "Release owner",
            "Spec owner",
            "QA/evidence owner",
            "Backend",
            "Security/Privacy",
            "Data Integrity",
            "Domain",
            "Retrieval",
            "Cognition",
            "API",
            "Security",
            "Product/UX",
            "Accessibility",
            "Visual Truth",
            "Frontend",
        ],
        Gate::F6 => &["Product", "Accessibility", "Performance", "Supply Chain"],
    }
}

/// The reviewer roles whose sign-off must be independent of the implementation
/// author (`validation.md` §6: "The author of an implementation may not be the
/// sole Security, Accessibility, Visual Truth, Retrieval-quality, crypto, or
/// license approver").
const INDEPENDENCE_REQUIRED_ROLES: &[&str] = &[
    "Security",
    "Accessibility",
    "Visual Truth",
    "Retrieval",
    "crypto",
    "license",
];

/// Whether a reviewer `role` satisfies a `required` role. Both sides are
/// compared case-insensitively and split on `/` so that e.g. a "Security"
/// sign-off satisfies the composite "Security/Privacy" mandate and vice versa.
fn role_matches(required: &str, role: &str) -> bool {
    let req: Vec<String> = required
        .to_ascii_lowercase()
        .split('/')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let have: Vec<String> = role
        .to_ascii_lowercase()
        .split('/')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    req.iter().any(|r| have.iter().any(|h| h == r))
}

/// Whether `role` is one whose sign-off must be independent.
fn role_requires_independence(role: &str) -> bool {
    INDEPENDENCE_REQUIRED_ROLES
        .iter()
        .any(|req| role_matches(req, role))
}

/// Whether `method` is a recognized signature method (case-insensitive).
fn is_allowed_signature_method(method: &str) -> bool {
    let m = method.trim().to_ascii_lowercase();
    !m.is_empty() && ALLOWED_SIGNATURE_METHODS.iter().any(|a| *a == m)
}

/// Whether `s` is a valid RFC 3339 instant expressed in UTC (`Z` / `+00:00`).
fn is_utc_rfc3339(s: &str) -> bool {
    match chrono::DateTime::parse_from_rfc3339(s.trim()) {
        Ok(dt) => dt.offset().local_minus_utc() == 0,
        Err(_) => false,
    }
}

/// Classify a waiver `scope` against the non-waivable classes
/// (`validation.md` §6: "A waiver cannot override P0 acceptance criteria,
/// unknown license, policy leak, false erasure, authority corruption, or an
/// earlier gate"). Returns the matched class name, or `None` when the scope is
/// waivable. Matching is a case-insensitive substring scan so paraphrased
/// scopes (e.g. "security regression", "a11y contrast") still trip the guard.
fn non_waivable_class(scope: &str) -> Option<&'static str> {
    let s = scope.to_ascii_lowercase();
    // (class-name, matching keywords) — ordered for deterministic first-match.
    const CLASSES: &[(&str, &[&str])] = &[
        ("P0", &["p0"]),
        ("security", &["security", "policy leak", "policy-leak"]),
        ("privacy", &["privacy", "false erasure", "false-erasure"]),
        (
            "integrity",
            &[
                "integrity",
                "authority corruption",
                "authority-corruption",
                "corruption",
            ],
        ),
        ("accessibility", &["accessibility", "a11y"]),
        ("license", &["license", "licence"]),
        (
            "earlier-gate",
            &["earlier gate", "earlier-gate", "prior gate"],
        ),
    ];
    for (class, keywords) in CLASSES {
        if keywords.iter().any(|k| s.contains(k)) {
            return Some(class);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fully-populated, schema-valid synthetic manifest.
    fn well_formed() -> EvidenceManifest {
        EvidenceManifest {
            schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
            run_id: "F0-2025-01-01T00-00-00Z-abc".to_string(),
            gate: Gate::F0,
            status: RunStatus::Pass,
            started_at: "2025-01-01T00:00:00Z".to_string(),
            ended_at: "2025-01-01T00:05:00Z".to_string(),
            actor: "ci-runner".to_string(),
            git: GitProvenance {
                commit: "a".repeat(40),
                branch: "main".to_string(),
                dirty: false,
                dirty_digest: None,
            },
            commands: vec![CommandInvocation {
                command_id: "CMD-MG-COVERAGE".to_string(),
                argv: vec!["mg-coverage".to_string(), "--quiet".to_string()],
                working_directory: ".".to_string(),
                exit_code: 0,
            }],
            requirement_ids: vec!["MGR-027".to_string(), "MGR-048".to_string()],
            decision_ids: vec!["MGD-016".to_string()],
            suite_ids: vec!["V-AUTH-01".to_string()],
            fixtures: vec![FixtureRef {
                fixture_id: "mg-unit-v2".to_string(),
                seed: "0x4D475201".to_string(),
                generator_hash: "deadbeef".to_string(),
            }],
            versions: VersionSet {
                authority_schema: "1".to_string(),
                ontology: "1".to_string(),
                model: "bge-small-en-v1.5".to_string(),
                rrf: "profile-v1".to_string(),
                scene: "scene-v2".to_string(),
            },
            build_environment: BuildEnvironment {
                os: Some("Ubuntu 24.04".to_string()),
                kernel: Some("6.8.0".to_string()),
                webkit_gtk: Some("2.44".to_string()),
                runtime: Some("rustc 1.83".to_string()),
                build_profile: Some("release".to_string()),
                lockfile_hashes: {
                    let mut m = BTreeMap::new();
                    m.insert("Cargo.lock".to_string(), "c".repeat(64));
                    m
                },
                binary_hashes: BTreeMap::new(),
            },
            reference_hardware: ReferenceHardware {
                hardware_id: Some("ref-laptop-01".to_string()),
                cpu: Some("Ryzen 7".to_string()),
                ram: Some("32GB".to_string()),
                gpu: None,
                storage: None,
                display: None,
                dpi: None,
            },
            environment_state: EnvironmentState {
                power_state: Some("AC".to_string()),
                thermal_state: None,
                network_state: Some("online".to_string()),
                protocol: MeasurementProtocol::WarmAndCold,
            },
            accessibility: Accessibility {
                locale: Some("en-US".to_string()),
                theme: Some("dark".to_string()),
                input: Some("keyboard".to_string()),
                assistive_tech: Some("orca".to_string()),
            },
            artifacts: vec![
                ArtifactReference {
                    path: "reports/coverage.json".to_string(),
                    media_type: "application/json".to_string(),
                    sha256: "b".repeat(64),
                    size: 1024,
                },
                ArtifactReference {
                    path: "https://artifacts.example/immutable/xyz".to_string(),
                    media_type: "application/octet-stream".to_string(),
                    sha256: "d".repeat(64),
                    size: 4096,
                },
            ],
            assertions: AssertionTotals {
                total: 48,
                passed: 48,
                failed: 0,
            },
            counterexamples: vec![],
            metrics: vec![MetricSeries {
                metric: "core_retrieval_p95_ms".to_string(),
                samples: vec![80.0, 90.0, 110.0],
                interval: Some([78.0, 112.0]),
            }],
            reviews: vec![ReviewRecord {
                role: "QA/evidence owner".to_string(),
                reviewer_id: "reviewer-1".to_string(),
                timestamp: "2025-01-01T00:06:00Z".to_string(),
                manifest_hash: "e".repeat(64),
                reviewed_artifact_hashes: vec!["b".repeat(64)],
                verdict: "approved".to_string(),
                independent: true,
                signature_method: "gpg".to_string(),
            }],
            waivers: vec![],
            predecessor_hashes: vec!["f".repeat(64)],
        }
    }

    #[test]
    fn well_formed_manifest_validates_clean() {
        let v = well_formed().validate();
        assert!(v.ok, "unexpected diagnostics: {:#?}", v.diagnostics);
        assert!(v.diagnostics.is_empty());
    }

    #[test]
    fn schema_version_constant_is_stable() {
        assert_eq!(MANIFEST_SCHEMA_VERSION, "memory-graph-evidence-manifest/v1");
        assert_eq!(EvidenceManifest::SCHEMA_VERSION, MANIFEST_SCHEMA_VERSION);
    }

    #[test]
    fn json_round_trips_with_stable_schema_version() {
        let m = well_formed();
        let first = m.to_json_pretty().expect("serializes");
        let second = m.to_json_pretty().expect("serializes");
        assert_eq!(first, second, "serialization must be byte-stable");
        assert!(first.contains("\"schemaVersion\""));
        assert!(first.contains("memory-graph-evidence-manifest/v1"));
        let parsed = EvidenceManifest::from_json(&first).expect("deserializes");
        assert_eq!(parsed, m);
        assert_eq!(parsed.to_json_pretty().expect("re-serializes"), first);
    }

    #[test]
    fn bad_schema_version_fails() {
        let mut m = well_formed();
        m.schema_version = "memory-graph-evidence-manifest/v0".to_string();
        let v = m.validate();
        assert!(!v.ok);
        assert!(v.has_kind(ManifestDiagnosticKind::BadSchemaVersion));
    }

    #[test]
    fn missing_required_field_fails() {
        let mut m = well_formed();
        m.run_id = "   ".to_string();
        let v = m.validate();
        assert!(!v.ok);
        assert!(v
            .diagnostics
            .iter()
            .any(|d| d.kind == ManifestDiagnosticKind::MissingField && d.field == "runId"));
    }

    #[test]
    fn absent_commit_digest_fails() {
        let mut m = well_formed();
        m.git.commit = String::new();
        let v = m.validate();
        assert!(v.has_kind(ManifestDiagnosticKind::BadCommitDigest));
    }

    #[test]
    fn bad_commit_digest_fails() {
        let mut m = well_formed();
        m.git.commit = "not-a-hash".to_string();
        let v = m.validate();
        assert!(v.has_kind(ManifestDiagnosticKind::BadCommitDigest));
    }

    #[test]
    fn dirty_tree_without_digest_fails() {
        let mut m = well_formed();
        m.git.dirty = true;
        m.git.dirty_digest = None;
        let v = m.validate();
        assert!(v.has_kind(ManifestDiagnosticKind::DirtyWithoutDigest));
    }

    #[test]
    fn dirty_tree_with_valid_digest_passes() {
        let mut m = well_formed();
        m.git.dirty = true;
        m.git.dirty_digest = Some("a".repeat(64));
        let v = m.validate();
        assert!(v.ok, "unexpected diagnostics: {:#?}", v.diagnostics);
    }

    #[test]
    fn null_required_environment_field_fails() {
        let mut m = well_formed();
        m.build_environment.os = None;
        let v = m.validate();
        assert!(!v.ok);
        assert!(v.diagnostics.iter().any(|d| {
            d.kind == ManifestDiagnosticKind::NullRequiredEnvironment
                && d.field == "buildEnvironment.os"
        }));
    }

    #[test]
    fn null_required_hardware_field_fails() {
        let mut m = well_formed();
        m.reference_hardware.cpu = None;
        let v = m.validate();
        assert!(v.diagnostics.iter().any(|d| {
            d.kind == ManifestDiagnosticKind::NullRequiredEnvironment
                && d.field == "referenceHardware.cpu"
        }));
    }

    #[test]
    fn malformed_predecessor_hash_fails() {
        let mut m = well_formed();
        m.predecessor_hashes = vec!["short".to_string()];
        let v = m.validate();
        assert!(v.has_kind(ManifestDiagnosticKind::MalformedPredecessorHash));
    }

    #[test]
    fn escaping_artifact_path_fails() {
        let mut m = well_formed();
        m.artifacts[0].path = "../../etc/passwd".to_string();
        let v = m.validate();
        assert!(v.has_kind(ManifestDiagnosticKind::ArtifactPathEscape));
    }

    #[test]
    fn absolute_artifact_path_fails() {
        let mut m = well_formed();
        m.artifacts[0].path = "/etc/passwd".to_string();
        let v = m.validate();
        assert!(v.has_kind(ManifestDiagnosticKind::ArtifactPathAbsolute));
    }

    #[test]
    fn nested_relative_artifact_path_passes() {
        let mut m = well_formed();
        // A path with `..` that stays within the repo is fine.
        m.artifacts[0].path = "reports/../reports/coverage.json".to_string();
        let v = m.validate();
        assert!(v.ok, "unexpected diagnostics: {:#?}", v.diagnostics);
    }

    #[test]
    fn malformed_artifact_checksum_fails() {
        let mut m = well_formed();
        m.artifacts[0].sha256 = "XYZ".to_string();
        let v = m.validate();
        assert!(v.has_kind(ManifestDiagnosticKind::MalformedArtifactChecksum));
    }

    #[test]
    fn pass_without_reviews_fails() {
        let mut m = well_formed();
        m.status = RunStatus::Pass;
        m.reviews.clear();
        let v = m.validate();
        assert!(v.has_kind(ManifestDiagnosticKind::MissingReviews));
    }

    #[test]
    fn planned_without_reviews_is_allowed() {
        let mut m = well_formed();
        m.status = RunStatus::Planned;
        m.reviews.clear();
        let v = m.validate();
        assert!(v.ok, "unexpected diagnostics: {:#?}", v.diagnostics);
    }

    #[test]
    fn malformed_ids_and_seed_fail() {
        let mut m = well_formed();
        m.requirement_ids = vec!["MGR-27".to_string()]; // wrong width
        m.decision_ids = vec!["MGD-XYZ".to_string()];
        m.suite_ids = vec!["AUTH-01".to_string()]; // missing V- prefix
        m.fixtures[0].seed = "0xZZZ".to_string();
        let v = m.validate();
        assert!(v.has_kind(ManifestDiagnosticKind::MalformedId));
        assert!(v.has_kind(ManifestDiagnosticKind::MalformedFixtureSeed));
    }

    #[test]
    fn bad_timestamp_fails() {
        let mut m = well_formed();
        m.started_at = "01/01/2025".to_string();
        let v = m.validate();
        assert!(v.has_kind(ManifestDiagnosticKind::BadTimestamp));
    }

    #[test]
    fn diagnostics_are_sorted_deterministically() {
        // Break several fields; the diagnostics vector must come back sorted.
        let mut m = well_formed();
        m.run_id = String::new();
        m.git.commit = "bad".to_string();
        m.predecessor_hashes = vec!["short".to_string()];
        m.artifacts[0].path = "/abs".to_string();
        let v = m.validate();
        let mut sorted = v.diagnostics.clone();
        sorted.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        assert_eq!(v.diagnostics, sorted);
        assert!(!v.ok);
    }

    #[test]
    fn validation_result_round_trips_through_json() {
        let mut m = well_formed();
        m.git.commit = "bad".to_string();
        let v = m.validate();
        let json = serde_json::to_string(&v).expect("serializes");
        let parsed: ManifestValidation = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(parsed, v);
    }

    // ---- 0.4.2 on-disk artifact verification tests ----

    use std::fs;
    use std::io::Write as _;

    /// Independent lowercase-hex SHA-256, not routed through the code under test.
    fn sha256_of(bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        let out = h.finalize();
        let mut s = String::with_capacity(out.len() * 2);
        for b in out {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    fn art(path: &str, media: &str, sha: &str, size: u64) -> ArtifactReference {
        ArtifactReference {
            path: path.to_string(),
            media_type: media.to_string(),
            sha256: sha.to_string(),
            size,
        }
    }

    /// A manifest built from `well_formed()` but carrying exactly `arts`.
    fn manifest_with(arts: Vec<ArtifactReference>) -> EvidenceManifest {
        let mut m = well_formed();
        m.artifacts = arts;
        m
    }

    /// Write `bytes` to `root/rel`, creating parent directories.
    fn write_artifact(root: &Path, rel: &str, bytes: &[u8]) {
        let full = root.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        let mut f = fs::File::create(&full).expect("create artifact file");
        f.write_all(bytes).expect("write artifact bytes");
    }

    #[test]
    fn verify_clean_manifest_with_matching_disk_artifacts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let body = br#"{"coverage": 100}"#;
        write_artifact(root, "reports/coverage.json", body);

        let m = manifest_with(vec![
            art(
                "reports/coverage.json",
                "application/json",
                &sha256_of(body),
                body.len() as u64,
            ),
            // Immutable URI: well-formed, filesystem checks skipped.
            art(
                "https://artifacts.example/immutable/xyz",
                "application/octet-stream",
                &"d".repeat(64),
                4096,
            ),
        ]);

        let v = m.verify_artifacts(root);
        assert!(v.ok, "unexpected diagnostics: {:#?}", v.diagnostics);
    }

    #[test]
    fn verify_rejects_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let m = manifest_with(vec![art(
            "reports/coverage.json",
            "application/json",
            &"a".repeat(64),
            10,
        )]);
        let v = m.verify_artifacts(dir.path());
        assert!(!v.ok);
        assert!(v.has_kind(ManifestDiagnosticKind::ArtifactMissing));
    }

    #[test]
    fn verify_rejects_wrong_checksum() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let body = b"actual bytes";
        write_artifact(root, "reports/coverage.json", body);
        // Declare the correct size but a bogus checksum -> tamper detected.
        let m = manifest_with(vec![art(
            "reports/coverage.json",
            "application/json",
            &"a".repeat(64),
            body.len() as u64,
        )]);
        let v = m.verify_artifacts(root);
        assert!(v.has_kind(ManifestDiagnosticKind::ArtifactChecksumMismatch));
        assert!(!v.has_kind(ManifestDiagnosticKind::ArtifactSizeMismatch));
    }

    #[test]
    fn verify_rejects_wrong_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let body = b"actual bytes";
        write_artifact(root, "reports/coverage.json", body);
        // Correct checksum, wrong declared size.
        let m = manifest_with(vec![art(
            "reports/coverage.json",
            "application/json",
            &sha256_of(body),
            body.len() as u64 + 99,
        )]);
        let v = m.verify_artifacts(root);
        assert!(v.has_kind(ManifestDiagnosticKind::ArtifactSizeMismatch));
        assert!(!v.has_kind(ManifestDiagnosticKind::ArtifactChecksumMismatch));
    }

    #[test]
    fn verify_rejects_wrong_media_type() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let body = br#"{"x":1}"#;
        write_artifact(root, "reports/coverage.json", body);
        // Correct bytes/size but the .json extension contradicts text/plain.
        let m = manifest_with(vec![art(
            "reports/coverage.json",
            "text/plain",
            &sha256_of(body),
            body.len() as u64,
        )]);
        let v = m.verify_artifacts(root);
        assert!(v.has_kind(ManifestDiagnosticKind::ArtifactMediaTypeMismatch));
        assert!(!v.has_kind(ManifestDiagnosticKind::ArtifactChecksumMismatch));
        assert!(!v.has_kind(ManifestDiagnosticKind::ArtifactSizeMismatch));
    }

    #[test]
    fn verify_rejects_duplicate_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let body = br#"{"x":1}"#;
        write_artifact(root, "reports/coverage.json", body);
        let good = art(
            "reports/coverage.json",
            "application/json",
            &sha256_of(body),
            body.len() as u64,
        );
        let m = manifest_with(vec![good.clone(), good]);
        let v = m.verify_artifacts(root);
        assert!(v.has_kind(ManifestDiagnosticKind::DuplicateArtifact));
    }

    #[test]
    fn verify_rejects_duplicate_checksum() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let body = br#"{"x":1}"#;
        // Two distinct paths with byte-identical content share a checksum.
        write_artifact(root, "reports/a.json", body);
        write_artifact(root, "reports/b.json", body);
        let sha = sha256_of(body);
        let m = manifest_with(vec![
            art(
                "reports/a.json",
                "application/json",
                &sha,
                body.len() as u64,
            ),
            art(
                "reports/b.json",
                "application/json",
                &sha,
                body.len() as u64,
            ),
        ]);
        let v = m.verify_artifacts(root);
        assert!(v.has_kind(ManifestDiagnosticKind::DuplicateArtifact));
    }

    #[test]
    fn verify_rejects_escaping_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let m = manifest_with(vec![art(
            "../outside.json",
            "application/json",
            &"a".repeat(64),
            10,
        )]);
        let v = m.verify_artifacts(dir.path());
        assert!(v.has_kind(ManifestDiagnosticKind::ArtifactPathEscape));
    }

    #[test]
    fn verify_streams_larger_than_chunk_file_clean() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        // ~200 KiB: several ARTIFACT_CHUNK_BYTES (64 KiB) chunks, proving the
        // streaming reader never depends on a single-read whole-file load.
        let mut body = Vec::with_capacity(200 * 1024);
        for i in 0..(200 * 1024u32) {
            body.push((i % 251) as u8);
        }
        assert!(body.len() > super::ARTIFACT_CHUNK_BYTES);
        write_artifact(root, "performance/samples.json", &body);
        let m = manifest_with(vec![art(
            "performance/samples.json",
            "application/json",
            &sha256_of(&body),
            body.len() as u64,
        )]);
        let v = m.verify_artifacts(root);
        assert!(v.ok, "unexpected diagnostics: {:#?}", v.diagnostics);
    }

    #[test]
    fn verify_rejects_malformed_uri() {
        let dir = tempfile::tempdir().expect("tempdir");
        let m = manifest_with(vec![art(
            "://missing-scheme/x",
            "application/octet-stream",
            &"d".repeat(64),
            10,
        )]);
        let v = m.verify_artifacts(dir.path());
        assert!(v.has_kind(ManifestDiagnosticKind::MalformedArtifactUri));
    }

    #[cfg(unix)]
    #[test]
    fn verify_rejects_symlinked_artifact_as_mutable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let body = br#"{"x":1}"#;
        // Real target lives under the root; the manifest references a symlink
        // to it, whose target can be repointed after the checksum is recorded.
        write_artifact(root, "reports/real.json", body);
        let link = root.join("reports/coverage.json");
        std::os::unix::fs::symlink(root.join("reports/real.json"), &link).expect("create symlink");
        let m = manifest_with(vec![art(
            "reports/coverage.json",
            "application/json",
            &sha256_of(body),
            body.len() as u64,
        )]);
        let v = m.verify_artifacts(root);
        assert!(v.has_kind(ManifestDiagnosticKind::MutableArtifact));
    }

    #[cfg(unix)]
    #[test]
    fn verify_rejects_world_writable_artifact_as_mutable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let body = br#"{"x":1}"#;
        write_artifact(root, "reports/coverage.json", body);
        let full = root.join("reports/coverage.json");
        let mut perms = fs::metadata(&full).expect("stat").permissions();
        perms.set_mode(0o666); // world-writable
        fs::set_permissions(&full, perms).expect("chmod");
        let m = manifest_with(vec![art(
            "reports/coverage.json",
            "application/json",
            &sha256_of(body),
            body.len() as u64,
        )]);
        let v = m.verify_artifacts(root);
        assert!(v.has_kind(ManifestDiagnosticKind::MutableArtifact));
    }

    // ---- 0.4.4 reviewer + waiver governance enforcement tests ----

    /// A valid, independent review record for `role` referencing the first
    /// well-formed artifact checksum (`b`*64) with a UTC timestamp + allowed
    /// signature method.
    fn gov_review(role: &str, reviewer_id: &str) -> ReviewRecord {
        ReviewRecord {
            role: role.to_string(),
            reviewer_id: reviewer_id.to_string(),
            timestamp: "2025-01-01T00:06:00Z".to_string(),
            manifest_hash: "e".repeat(64),
            reviewed_artifact_hashes: vec!["b".repeat(64)],
            verdict: "approved".to_string(),
            independent: true,
            signature_method: "gpg".to_string(),
        }
    }

    /// An `F1` `Pass` carrying every mandatory `F1` reviewer role, each an
    /// independent, well-signed, UTC-timestamped sign-off referencing a real
    /// manifest artifact. Actor differs from every reviewer.
    fn governance_pass() -> EvidenceManifest {
        let mut m = well_formed();
        m.gate = Gate::F1;
        m.actor = "impl-author".to_string();
        m.reviews = vec![
            gov_review("Backend", "rev-backend"),
            gov_review("Security/Privacy", "rev-security"),
            gov_review("Data Integrity", "rev-integrity"),
        ];
        m.waivers = vec![];
        m
    }

    #[test]
    fn governance_well_formed_pass_enforces_clean() {
        let v = governance_pass().enforce_governance();
        assert!(v.ok, "unexpected diagnostics: {:#?}", v.diagnostics);
    }

    #[test]
    fn governance_missing_required_reviewer_role_fails() {
        let mut m = governance_pass();
        // Drop the Data Integrity sign-off mandated for F1.
        m.reviews.retain(|r| r.role != "Data Integrity");
        let v = m.enforce_governance();
        assert!(!v.ok);
        assert!(v.has_kind(ManifestDiagnosticKind::MissingRequiredReviewer));
    }

    #[test]
    fn governance_non_independent_reviewer_fails() {
        let mut m = governance_pass();
        // Security requires independence; mark its sign-off non-independent.
        for r in &mut m.reviews {
            if r.role == "Security/Privacy" {
                r.independent = false;
            }
        }
        let v = m.enforce_governance();
        assert!(v.has_kind(ManifestDiagnosticKind::NonIndependentReviewer));
    }

    #[test]
    fn governance_reviewer_is_run_actor_fails_independence() {
        let mut m = governance_pass();
        // The implementation author (actor) cannot be the sole Security approver.
        for r in &mut m.reviews {
            if r.role == "Security/Privacy" {
                r.reviewer_id = m.actor.clone();
            }
        }
        let v = m.enforce_governance();
        assert!(v.has_kind(ManifestDiagnosticKind::NonIndependentReviewer));
    }

    #[test]
    fn governance_review_hash_not_in_manifest_fails() {
        let mut m = governance_pass();
        m.reviews[0].reviewed_artifact_hashes = vec!["a".repeat(64)];
        let v = m.enforce_governance();
        assert!(v.has_kind(ManifestDiagnosticKind::ReviewHashNotInManifest));
    }

    #[test]
    fn governance_malformed_review_manifest_hash_fails() {
        let mut m = governance_pass();
        m.reviews[0].manifest_hash = "not-a-hash".to_string();
        let v = m.enforce_governance();
        assert!(v.has_kind(ManifestDiagnosticKind::MalformedReviewManifestHash));
    }

    #[test]
    fn governance_bad_signature_method_fails() {
        let mut m = governance_pass();
        m.reviews[0].signature_method = "carrier-pigeon".to_string();
        let v = m.enforce_governance();
        assert!(v.has_kind(ManifestDiagnosticKind::BadSignatureMethod));
    }

    #[test]
    fn governance_non_utc_timestamp_fails() {
        let mut m = governance_pass();
        // Valid RFC 3339 but a non-UTC offset -> rejected.
        m.reviews[0].timestamp = "2025-01-01T00:06:00+05:30".to_string();
        let v = m.enforce_governance();
        assert!(v.has_kind(ManifestDiagnosticKind::NonUtcTimestamp));
    }

    #[test]
    fn governance_malformed_timestamp_fails() {
        let mut m = governance_pass();
        m.reviews[0].timestamp = "01/01/2025".to_string();
        let v = m.enforce_governance();
        assert!(v.has_kind(ManifestDiagnosticKind::NonUtcTimestamp));
    }

    #[test]
    fn governance_non_waivable_conditions_fail() {
        for scope in [
            "P0 acceptance criteria",
            "security regression",
            "privacy: policy leak",
            "authority corruption",
            "false erasure of records",
            "a11y contrast",
            "unknown license",
            "earlier gate F1",
        ] {
            let mut m = governance_pass();
            m.waivers = vec![Waiver {
                waiver_id: "W-1".to_string(),
                scope: scope.to_string(),
                justification: "n/a".to_string(),
                expiry: None,
            }];
            let v = m.enforce_governance();
            assert!(
                v.has_kind(ManifestDiagnosticKind::NonWaivableCondition),
                "scope '{scope}' should be non-waivable, diagnostics: {:#?}",
                v.diagnostics
            );
        }
    }

    #[test]
    fn governance_waivable_scope_passes() {
        let mut m = governance_pass();
        m.waivers = vec![Waiver {
            waiver_id: "W-2".to_string(),
            scope: "documentation typo in report footer".to_string(),
            justification: "cosmetic only".to_string(),
            expiry: None,
        }];
        let v = m.enforce_governance();
        assert!(v.ok, "unexpected diagnostics: {:#?}", v.diagnostics);
    }

    #[test]
    fn governance_non_pass_does_not_force_reviewer_roles() {
        // A Planned manifest with no reviews carries no mandatory-role burden.
        let mut m = governance_pass();
        m.status = RunStatus::Planned;
        m.reviews.clear();
        let v = m.enforce_governance();
        assert!(v.ok, "unexpected diagnostics: {:#?}", v.diagnostics);
    }

    #[test]
    fn governance_diagnostics_are_sorted_deterministically() {
        let mut m = governance_pass();
        m.reviews[0].signature_method = "bad".to_string();
        m.reviews[1].reviewed_artifact_hashes = vec!["a".repeat(64)];
        m.waivers = vec![Waiver {
            waiver_id: "W-1".to_string(),
            scope: "security".to_string(),
            justification: "n/a".to_string(),
            expiry: None,
        }];
        let v = m.enforce_governance();
        let mut sorted = v.diagnostics.clone();
        sorted.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        assert_eq!(v.diagnostics, sorted);
        assert!(!v.ok);
    }

    // ---- 0.4.5 predecessor / gate-promotion tests ----

    use crate::memory_graph::promotion::GatePromotion;

    /// A valid, signed `F0` `Pass` predecessor: carries both mandatory `F0`
    /// reviewer roles, real commands + artifacts (from [`well_formed`]), and no
    /// predecessor of its own.
    fn f0_pass_predecessor() -> EvidenceManifest {
        let mut m = well_formed();
        m.gate = Gate::F0;
        m.status = RunStatus::Pass;
        m.actor = "impl-author".to_string();
        m.reviews = vec![
            gov_review("Spec owner", "rev-spec"),
            gov_review("QA/evidence owner", "rev-qa"),
        ];
        m.waivers = vec![];
        m.predecessor_hashes = vec![];
        m
    }

    /// A valid, signed `F1` `Pass` candidate that records `pred`'s manifest hash
    /// as its predecessor.
    fn f1_candidate_for(pred: &EvidenceManifest) -> EvidenceManifest {
        let mut m = governance_pass(); // F1 Pass with all mandatory F1 reviewers
        m.predecessor_hashes = vec![pred.manifest_hash()];
        m
    }

    #[test]
    fn manifest_hash_is_stable_and_hex() {
        let m = f0_pass_predecessor();
        let h1 = m.manifest_hash();
        let h2 = m.manifest_hash();
        assert_eq!(h1, h2, "manifest hash must be byte-stable");
        assert_eq!(h1.len(), 64);
        assert!(h1
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn f0_promotes_without_any_predecessor() {
        // F0 has no predecessor; a self-clean, evidence-backed F0 Pass promotes.
        let mut m = well_formed();
        m.reviews = vec![
            gov_review("Spec owner", "rev-spec"),
            gov_review("QA/evidence owner", "rev-qa"),
        ];
        let result = m.can_promote(&[], None);
        assert!(
            result.is_promoted(),
            "expected promotion, got: {:#?}",
            result.reasons()
        );
        assert_eq!(
            result,
            GatePromotion::Promoted {
                gate: Gate::F0,
                status: RunStatus::Pass
            }
        );
    }

    #[test]
    fn f1_promotes_with_signed_f0_pass_predecessor() {
        let pred = f0_pass_predecessor();
        let candidate = f1_candidate_for(&pred);
        let result = candidate.can_promote(std::slice::from_ref(&pred), None);
        assert!(
            result.is_promoted(),
            "expected promotion, got: {:#?}",
            result.reasons()
        );
    }

    #[test]
    fn missing_predecessor_manifest_blocks_promotion() {
        let pred = f0_pass_predecessor();
        let candidate = f1_candidate_for(&pred); // records the hash...
        let result = candidate.can_promote(&[], None); // ...but no F0 supplied
        assert!(!result.is_promoted());
        assert!(result.has_kind(ManifestDiagnosticKind::PredecessorMissing));
    }

    #[test]
    fn unrecorded_predecessor_hash_blocks_promotion() {
        let pred = f0_pass_predecessor();
        let mut candidate = f1_candidate_for(&pred);
        // A valid signed F0 Pass is supplied, but its hash is not recorded.
        candidate.predecessor_hashes = vec![];
        let result = candidate.can_promote(std::slice::from_ref(&pred), None);
        assert!(!result.is_promoted());
        assert!(result.has_kind(ManifestDiagnosticKind::PredecessorHashMismatch));
    }

    #[test]
    fn failed_or_blocked_predecessor_blocks_later_pass() {
        let mut pred = f0_pass_predecessor();
        pred.status = RunStatus::Fail;
        let candidate = f1_candidate_for(&pred); // records the (Fail) manifest hash
        let result = candidate.can_promote(std::slice::from_ref(&pred), None);
        assert!(!result.is_promoted());
        // No later-gate polish may mask an earlier failure.
        assert!(result.has_kind(ManifestDiagnosticKind::EarlierGateP0Unresolved));
        // A Fail predecessor is also not a valid signed Pass.
        assert!(result.has_kind(ManifestDiagnosticKind::PredecessorNotPassed));
    }

    #[test]
    fn f6_without_signed_f5_predecessor_blocks() {
        // F6 may start "only from a signed F5 manifest".
        let mut m = well_formed();
        m.gate = Gate::F6;
        m.status = RunStatus::Pass;
        m.reviews = vec![
            gov_review("Product", "rev-product"),
            gov_review("Accessibility", "rev-a11y"),
            gov_review("Performance", "rev-perf"),
            gov_review("Supply Chain", "rev-supply"),
        ];
        m.predecessor_hashes = vec![];
        let result = m.can_promote(&[], None);
        assert!(!result.is_promoted());
        // The immediate F5 predecessor is missing...
        assert!(result.has_kind(ManifestDiagnosticKind::PredecessorMissing));
        // ...and the F0..F4 chain is entirely absent.
        assert!(result.has_kind(ManifestDiagnosticKind::GateChainGap));
    }

    #[test]
    fn checklist_or_checkbox_claim_cannot_promote_a_gate() {
        // A checked `tasks.md` box carries NO executed command and NO artifact.
        // `can_promote` takes no `tasks.md`/checkbox parameter (see its
        // signature) and derives status only from manifest evidence, so such a
        // claim is refused. This proves checkbox state is irrelevant to promotion.
        let mut checkbox_claim = f0_pass_predecessor();
        checkbox_claim.commands = vec![];
        checkbox_claim.artifacts = vec![];
        let result = checkbox_claim.can_promote(&[], None);
        assert!(!result.is_promoted());
        assert!(result.has_kind(ManifestDiagnosticKind::ChecklistOnlyPromotion));
    }

    #[test]
    fn promotion_diagnostics_are_sorted_and_deterministic() {
        let pred = f0_pass_predecessor();
        let candidate = f1_candidate_for(&pred);
        let first = candidate.can_promote(&[], None);
        let second = candidate.can_promote(&[], None);
        assert_eq!(first, second, "promotion must be deterministic");
        if let GatePromotion::Blocked { reasons, .. } = &first {
            let mut sorted = reasons.clone();
            sorted.sort_by(|a, b| {
                (a.kind.code(), a.field.as_str(), a.reason.as_str()).cmp(&(
                    b.kind.code(),
                    b.field.as_str(),
                    b.reason.as_str(),
                ))
            });
            assert_eq!(reasons, &sorted, "reasons must be sorted");
        } else {
            panic!("expected Blocked");
        }
    }
}
