//! Validated domain value objects for the v2 memory authority boundary.
//!
//! Task F1.2.2: consolidate the typed value objects the v2 schema/authority
//! boundary needs so that "validated IDs/times/policy hashes cannot be raw
//! unchecked strings at boundaries." Every type here is a newtype with a smart
//! constructor that validates on construction, so an invalid value is
//! unrepresentable once inside the domain.
//!
//! These reuse the canonical validators already anchored at the write boundary
//! ([`crate::db::encoding`]) and the id generator
//! ([`crate::ids`]) — they do not re-implement UUID/timestamp rules.
//!
//! Nothing wires these into stores yet (that is F1.2.3+); this task only defines
//! and exports them as the public model surface.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::db::encoding::{assert_rfc3339_utc, canonical_uuid};
use crate::error::{MemoryResult, StorageError};

// ── F2 cognitive-record model (design §4.2/§4.3, task F2.1.1) ───────────────
//
// The typed cognitive records and their supporting semantic/observation types.
// These are the Rust counterparts of the v2 authority tables added by migration
// 0017. They reuse the validated value objects defined in this module
// (RecordId/EventId/UtcTimestamp/ValidInterval/PolicyPartition/…) so no field
// is a raw unchecked string at the domain boundary.
pub mod active_predicate;
pub mod consent_gate;
pub mod consolidation;
pub mod content_fence;
pub mod contradiction;
pub mod correction;
pub mod dedup_versioning;
pub mod entity;
pub mod entity_proposal;
pub mod episode;
pub mod feedback;
pub mod forward_compat;
pub mod goal;
pub mod identifier;
pub mod ingestion_chunk;
pub mod ingestion_control;
pub mod interchange;
pub mod interchange_export;
pub mod interchange_fixtures;
pub mod interchange_import;
pub mod legacy_mapping;
pub mod mention_provenance;
pub mod merge_preview;
pub mod observation;
pub mod proposal_action;
pub mod provenance;
pub mod record;
pub mod relation_registry;
pub mod relationship_identity;
pub mod row_mapping;
pub mod source;
pub mod source_state;
pub mod split_reversal;
pub mod supersession;
pub mod temporal_query;
pub mod truth;

pub use consent_gate::{
    ConsentDecision, ConsentGate, ConsentGateError, ConsentOutcome, ConsentRequest,
    DiscoveryCandidate,
};
pub use consolidation::{ConsolidationLevel, ConsolidationRun};
pub use content_fence::{
    ContentFence, ContentFenceDecision, FenceScanResult, InjectionPattern, PolicyPropagationResult,
    SecretSensitivityClass,
};
pub use contradiction::{
    ConflictSide, ContradictionEvaluator, ContradictionExplanation, ContradictionResolution,
    EvidenceWeight, PrecedenceFactor, PrecedenceFactorResult, UnresolvedReason,
};
pub use correction::{
    CorrectionCommitRequest, CorrectionError, CorrectionKind, CorrectionPreview,
    CorrectionPreviewRequest, CorrectionResult, CorrectionValidator,
};
pub use dedup_versioning::{
    DuplicateDecision, DuplicateEvaluator, IdempotencyKeyBuilder, SourceEventKey,
    VersionedItemState,
};
pub use entity::{Alias, Entity, Evidence, EvidencePolarity, Mention};
pub use entity_proposal::{
    EntityProposalBuilder, EntityResolutionProposal, ProposalError, ProposalMatchMethod,
    ProposalRationale, ProposalStatus,
};
pub use episode::Episode;
pub use feedback::Feedback;
pub use forward_compat::{ForwardCompatible, UnknownFields};
pub use goal::{Goal, GoalProgress, GoalStatus};
pub use identifier::{
    IdentifierNormalizer, IdentifierStrength, IdentifierType, NormalizationError,
    NormalizedIdentifier,
};
pub use ingestion_control::{
    CancelPoint, DependencyDeletionAction, FaultInjectionPoint, IngestionFaultResult,
    SourceDeletionDependency, SourceDeletionPreview, SourceDeletionPreviewBuilder,
};
pub use merge_preview::{
    MergeCommitRequest, MergePreview, MergePreviewBuilder, MergePreviewError, MergePreviewSummary,
    PolicyMeet,
};
pub use observation::{RetrievalTrace, RetrievalTraceItem, ToolObservation};
pub use proposal_action::{
    CanonicalEndpointCorrection, ProposalAction, ProposalActionBuilder, ProposalActionError,
    ProposalActionKind, ProposalAfterState, ProposalBeforeState,
};
pub use provenance::{
    Actor, HasProvenance, Locator, Method, ModelIdentity, ParentRef, Provenance, ProvenanceTime,
    SourceRef,
};
pub use record::{Record, RecordKind};
pub use relation_registry::{
    DirectionClass, EndpointKind, EvidencePolicy, RelationDefinition, RelationName,
    RelationRegistry, ValidityPolicy,
};
pub use relationship_identity::{RelationEndpoint, RelationshipIdentity};
pub use source::Source as SourceRecord;
pub use source_state::{
    ConsentState, SourceCursor, SourceKind as SourceKindV2, SourceLifecycleState,
    SourceStateTransitionError, SourceStateValidator, SourceTrustClass,
};
pub use split_reversal::{
    LinkEndpointKind, SplitReconstructionBuilder, SplitReconstructionError,
    SplitReconstructionItem, SplitReconstructionPlan, UnresolvableItem, UnresolvableReason,
};
pub use truth::TruthState;

/// Build a canonical-encoding validation error (`StorageError::Encoding`).
pub(crate) fn encoding_err(msg: impl Into<String>) -> crate::error::MemoryError {
    StorageError::Encoding(msg.into()).into()
}

/// Maximum length of an [`IdempotencyKey`], in bytes. Bounded so a hostile or
/// buggy caller cannot use the idempotency table as unbounded storage.
pub const IDEMPOTENCY_KEY_MAX_LEN: usize = 512;

// ── UUID-shaped identifiers ────────────────────────────────────────────────
//
// `RecordId`, `EventId`, and `InvocationId` are distinct newtypes over the
// canonical lower-case UUID form so they cannot be transposed at call sites.
// A macro keeps the three definitions identical without duplication.

macro_rules! uuid_newtype {
    ($(#[$meta:meta])* $name:ident, $what:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            /// Validate `s` as a canonical UUID and wrap it. Accepts mixed-case
            /// hex (normalised to lower-case); rejects any other shape.
            pub fn new(s: impl AsRef<str>) -> MemoryResult<Self> {
                Ok(Self(canonical_uuid(s.as_ref())?))
            }

            /// Generate a fresh time-ordered id (UUID v7) — always canonical.
            pub fn new_v7() -> Self {
                Self(crate::ids::new_id().to_string())
            }

            /// The canonical lower-case UUID string.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consume the newtype, returning the owned canonical string.
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl TryFrom<String> for $name {
            type Error = crate::error::MemoryError;
            fn try_from(s: String) -> MemoryResult<Self> {
                Self::new(s)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = crate::error::MemoryError;
            fn try_from(s: &str) -> MemoryResult<Self> {
                Self::new(s)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
                ser.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
                let raw = String::deserialize(de)?;
                Self::new(raw).map_err(serde::de::Error::custom)
            }
        }
    };
}

uuid_newtype!(
    /// Identity of a cognitive record (`records.id`) — canonical UUID.
    RecordId,
    "record id"
);
uuid_newtype!(
    /// Identity of an authority event (`events.id`) — canonical UUID.
    EventId,
    "event id"
);
uuid_newtype!(
    /// Identity of a tool/command invocation — canonical UUID.
    InvocationId,
    "invocation id"
);
uuid_newtype!(
    /// Identity of an audit record (`audit_records.id`) — canonical UUID.
    AuditId,
    "audit id"
);
uuid_newtype!(
    /// Identity of an audited declassification provenance record
    /// (`declassifications.id`) — canonical UUID (MGR-004 AC3).
    DeclassificationId,
    "declassification id"
);
uuid_newtype!(
    /// Identity of a graph entity (`entities_v2.id`) — canonical UUID.
    EntityId,
    "entity id"
);
uuid_newtype!(
    /// Identity of an entity alias (`aliases.id`) — canonical UUID.
    AliasId,
    "alias id"
);
uuid_newtype!(
    /// Identity of a mention (`mentions.id`) — canonical UUID.
    MentionId,
    "mention id"
);
uuid_newtype!(
    /// Identity of an evidence artifact (`evidence_v2.id`) — canonical UUID.
    EvidenceId,
    "evidence id"
);
uuid_newtype!(
    /// Identity of a semantic relationship row (`relationships_v2.id`) —
    /// canonical UUID (task F2.2.3/F2.2.4).
    RelationshipId,
    "relationship id"
);
uuid_newtype!(
    /// Identity of an episode (`episodes_v2.id`) — canonical UUID.
    EpisodeId,
    "episode id"
);
uuid_newtype!(
    /// Identity of a goal (`goals_v2.id`) — canonical UUID.
    GoalId,
    "goal id"
);
uuid_newtype!(
    /// Identity of a goal-progress observation (`goal_progress.id`) — canonical UUID.
    GoalProgressId,
    "goal progress id"
);
uuid_newtype!(
    /// Identity of a consolidation run (`consolidation_runs.id`) — canonical UUID.
    ConsolidationRunId,
    "consolidation run id"
);
uuid_newtype!(
    /// Identity of a source (`sources.id`) — canonical UUID.
    SourceId,
    "source id"
);
uuid_newtype!(
    /// Identity of a tool observation (`tool_observations.id`) — canonical UUID.
    ToolObservationId,
    "tool observation id"
);
uuid_newtype!(
    /// Identity of a retrieval trace (`retrieval_traces.id`) — canonical UUID.
    RetrievalTraceId,
    "retrieval trace id"
);
uuid_newtype!(
    /// Identity of a feedback signal (`feedback.id`) — canonical UUID.
    FeedbackId,
    "feedback id"
);

// ── IdempotencyKey ─────────────────────────────────────────────────────────

/// A bounded, opaque idempotency key supplied by a caller (paired with a
/// [`PolicyPartition`] in `idempotency_results`). Not a UUID: it is any stable
/// caller-chosen token, but it must be non-empty, within
/// [`IDEMPOTENCY_KEY_MAX_LEN`] bytes, and free of interior control characters so
/// it is safe to store and log verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Validate and wrap an idempotency key.
    pub fn new(s: impl Into<String>) -> MemoryResult<Self> {
        let s = s.into();
        if s.is_empty() {
            return Err(encoding_err("idempotency key must not be empty"));
        }
        if s.len() > IDEMPOTENCY_KEY_MAX_LEN {
            return Err(encoding_err(format!(
                "idempotency key too long: {} bytes (max {IDEMPOTENCY_KEY_MAX_LEN})",
                s.len()
            )));
        }
        if let Some(bad) = s.chars().find(|c| c.is_control()) {
            return Err(encoding_err(format!(
                "idempotency key contains control character {bad:?}"
            )));
        }
        Ok(Self(s))
    }

    /// The key string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the newtype, returning the owned key string.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for IdempotencyKey {
    type Error = crate::error::MemoryError;
    fn try_from(s: String) -> MemoryResult<Self> {
        Self::new(s)
    }
}

impl Serialize for IdempotencyKey {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for IdempotencyKey {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(de)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

// ── PolicyPartition ────────────────────────────────────────────────────────

/// Maximum sensitivity level (design §4.1: `sensitivity INTEGER CHECK 0..3`).
pub const SENSITIVITY_MAX: u8 = 3;

/// The structured policy partition used across the v2 schema. It captures the
/// `namespace` / `scope` / `sensitivity` triple that governs where a row lives
/// and who may read it (design §4.1 policy columns), plus an optional
/// `owner_id`. Numeric sensitivity ordering makes `effective = max(...)`.
///
/// [`PolicyPartition::partition_key`] produces the stable canonical string
/// stored as the `policy_partition` / `caller_partition` column.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct PolicyPartition {
    namespace: String,
    scope: String,
    sensitivity: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner_id: Option<String>,
}

impl PolicyPartition {
    /// Construct a partition without an explicit owner. Validates `namespace`
    /// and `scope` are non-empty and `sensitivity` is in `0..=3`.
    pub fn new(
        namespace: impl Into<String>,
        scope: impl Into<String>,
        sensitivity: u8,
    ) -> MemoryResult<Self> {
        Self::with_owner(namespace, scope, sensitivity, None)
    }

    /// Construct a partition with an optional owner id.
    pub fn with_owner(
        namespace: impl Into<String>,
        scope: impl Into<String>,
        sensitivity: u8,
        owner_id: Option<String>,
    ) -> MemoryResult<Self> {
        let namespace = namespace.into();
        let scope = scope.into();
        if namespace.trim().is_empty() {
            return Err(encoding_err("policy partition namespace must not be empty"));
        }
        if scope.trim().is_empty() {
            return Err(encoding_err("policy partition scope must not be empty"));
        }
        if sensitivity > SENSITIVITY_MAX {
            return Err(encoding_err(format!(
                "policy sensitivity {sensitivity} out of range 0..={SENSITIVITY_MAX}"
            )));
        }
        if let Some(owner) = &owner_id {
            if owner.trim().is_empty() {
                return Err(encoding_err(
                    "policy partition owner_id must not be empty when present",
                ));
            }
        }
        Ok(Self {
            namespace,
            scope,
            sensitivity,
            owner_id,
        })
    }

    /// The partition namespace.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The partition scope.
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// The sensitivity level (`0..=3`).
    pub fn sensitivity(&self) -> u8 {
        self.sensitivity
    }

    /// The optional owner id.
    pub fn owner_id(&self) -> Option<&str> {
        self.owner_id.as_deref()
    }

    /// The stable canonical string stored as `policy_partition` /
    /// `caller_partition`: `"{namespace}/{scope}/{sensitivity}"`. Owner is not
    /// part of the partition key (it is stored in its own column).
    pub fn partition_key(&self) -> String {
        format!("{}/{}/{}", self.namespace, self.scope, self.sensitivity)
    }
}

impl fmt::Display for PolicyPartition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.partition_key())
    }
}

/// Serde surrogate for validating on deserialize.
#[derive(Deserialize)]
struct PolicyPartitionRaw {
    namespace: String,
    scope: String,
    sensitivity: u8,
    #[serde(default)]
    owner_id: Option<String>,
}

impl<'de> Deserialize<'de> for PolicyPartition {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = PolicyPartitionRaw::deserialize(de)?;
        Self::with_owner(raw.namespace, raw.scope, raw.sensitivity, raw.owner_id)
            .map_err(serde::de::Error::custom)
    }
}

// ── GraphRevision ──────────────────────────────────────────────────────────

/// A monotonic authority revision number (`authority_meta.graph_revision`,
/// `graph_revisions.revision`). `u64` gives the schema's `>= 0` invariant for
/// free. The schema invariant `base_revision = revision - 1` is expressed by
/// [`GraphRevision::base`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GraphRevision(u64);

impl GraphRevision {
    /// The base revision `0` (before any committed change).
    pub const fn base() -> Self {
        GraphRevision(0)
    }

    /// Wrap a raw revision number.
    pub const fn new(revision: u64) -> Self {
        GraphRevision(revision)
    }

    /// The raw revision number.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The next revision (`revision + 1`). Saturating; the space is effectively
    /// unbounded for a single-laptop authority.
    pub const fn next(self) -> Self {
        GraphRevision(self.0.saturating_add(1))
    }

    /// The base revision for this revision, per the schema invariant
    /// `base_revision = revision - 1`. Revision `0` has no base (`None`).
    pub const fn base_revision(self) -> Option<GraphRevision> {
        match self.0 {
            0 => None,
            n => Some(GraphRevision(n - 1)),
        }
    }
}

impl fmt::Display for GraphRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── UtcTimestamp ───────────────────────────────────────────────────────────

/// A UTC instant that is guaranteed to serialize to canonical RFC 3339 UTC
/// text. This is the boundary type ensuring stored timestamps are canonical
/// UTC (design §14; validators in [`crate::db::encoding`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UtcTimestamp(DateTime<Utc>);

impl UtcTimestamp {
    /// The current wall-clock instant in UTC.
    pub fn now() -> Self {
        UtcTimestamp(Utc::now())
    }

    /// Wrap an already-`DateTime<Utc>` value (always canonical UTC).
    pub fn from_datetime(dt: DateTime<Utc>) -> Self {
        UtcTimestamp(dt)
    }

    /// Parse and validate an RFC 3339 timestamp that must be in UTC (offset
    /// zero). Rejects non-UTC offsets and malformed input.
    pub fn from_rfc3339_utc(s: &str) -> MemoryResult<Self> {
        assert_rfc3339_utc(s)?;
        let parsed = DateTime::parse_from_rfc3339(s)
            .map_err(|e| encoding_err(format!("timestamp {s:?} is not valid RFC 3339: {e}")))?;
        Ok(UtcTimestamp(parsed.with_timezone(&Utc)))
    }

    /// The underlying `DateTime<Utc>`.
    pub fn as_datetime(&self) -> DateTime<Utc> {
        self.0
    }

    /// Canonical RFC 3339 UTC text (`…+00:00`), suitable for storage.
    pub fn to_rfc3339(&self) -> String {
        self.0.to_rfc3339()
    }
}

impl fmt::Display for UtcTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

impl Serialize for UtcTimestamp {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.to_rfc3339())
    }
}

impl<'de> Deserialize<'de> for UtcTimestamp {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(de)?;
        Self::from_rfc3339_utc(&raw).map_err(serde::de::Error::custom)
    }
}

// ── ValidInterval ──────────────────────────────────────────────────────────

/// A bitemporal-style valid interval (`valid_from` / `valid_until`) with
/// half-open `[from, until)` semantics. Either bound may be open (`None`). The
/// constructor rejects inverted intervals (`valid_from > valid_until`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ValidInterval {
    #[serde(skip_serializing_if = "Option::is_none")]
    valid_from: Option<UtcTimestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    valid_until: Option<UtcTimestamp>,
}

impl ValidInterval {
    /// Construct an interval, rejecting `valid_from > valid_until` when both are
    /// present.
    pub fn new(
        valid_from: Option<UtcTimestamp>,
        valid_until: Option<UtcTimestamp>,
    ) -> MemoryResult<Self> {
        if let (Some(from), Some(until)) = (valid_from, valid_until) {
            if from > until {
                return Err(encoding_err(format!(
                    "inverted valid interval: from {from} > until {until}"
                )));
            }
        }
        Ok(Self {
            valid_from,
            valid_until,
        })
    }

    /// A fully-open interval (valid for all time).
    pub fn open() -> Self {
        Self {
            valid_from: None,
            valid_until: None,
        }
    }

    /// The lower bound (inclusive), if any.
    pub fn valid_from(&self) -> Option<UtcTimestamp> {
        self.valid_from
    }

    /// The upper bound (exclusive), if any.
    pub fn valid_until(&self) -> Option<UtcTimestamp> {
        self.valid_until
    }

    /// Whether the interval is fully open on both ends.
    pub fn is_open(&self) -> bool {
        self.valid_from.is_none() && self.valid_until.is_none()
    }

    /// Whether `ts` falls within the half-open interval `[from, until)`. Open
    /// bounds are treated as `-∞` / `+∞`.
    pub fn contains(&self, ts: UtcTimestamp) -> bool {
        let after_start = self.valid_from.map(|from| ts >= from).unwrap_or(true);
        let before_end = self.valid_until.map(|until| ts < until).unwrap_or(true);
        after_start && before_end
    }
}

/// Serde surrogate for validating on deserialize.
#[derive(Deserialize)]
struct ValidIntervalRaw {
    #[serde(default)]
    valid_from: Option<UtcTimestamp>,
    #[serde(default)]
    valid_until: Option<UtcTimestamp>,
}

impl<'de> Deserialize<'de> for ValidInterval {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = ValidIntervalRaw::deserialize(de)?;
        Self::new(raw.valid_from, raw.valid_until).map_err(serde::de::Error::custom)
    }
}

// ── Schema / version value objects ─────────────────────────────────────────

/// A schema/version number (`records.schema_version`, migration
/// `schema_versions.version`, `mem_vectors` etc.). A plain monotonic `u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SchemaVersion(u32);

impl SchemaVersion {
    /// Wrap a raw schema version number.
    pub const fn new(version: u32) -> Self {
        SchemaVersion(version)
    }

    /// The raw version number.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A registry/relation version number (`relation_registry (relation_name,
/// version)`, per-record content version). A plain monotonic `u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Version(u32);

impl Version {
    /// The first version (`1`).
    pub const fn first() -> Self {
        Version(1)
    }

    /// Wrap a raw version number.
    pub const fn new(version: u32) -> Self {
        Version(version)
    }

    /// The raw version number.
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The next version (`version + 1`, saturating).
    pub const fn next(self) -> Self {
        Version(self.0.saturating_add(1))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── CallerContext ──────────────────────────────────────────────────────────

/// The adapter boundary that authenticated a caller (design §19.8; architecture
/// "Tauri/Axum: authenticate, create `CallerContext`"). Desktop callers are
/// in-process and locally trusted; server callers are authenticated over a
/// transport and can never gain scope beyond their signed grants. The origin is
/// the *distinct* per-adapter fact this task preserves — both adapters wire to
/// the one composition root, but each constructs its own authenticated caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallerOrigin {
    /// The in-process desktop (Tauri) adapter — local, implicitly trusted.
    LocalDesktop,
    /// A caller authenticated by the server (Axum) adapter over a transport.
    AuthenticatedRemote,
}

impl CallerOrigin {
    /// The canonical text form (stable for audit/logging).
    pub fn as_str(self) -> &'static str {
        match self {
            CallerOrigin::LocalDesktop => "local_desktop",
            CallerOrigin::AuthenticatedRemote => "authenticated_remote",
        }
    }

    /// Whether this is the in-process, locally-trusted desktop origin.
    pub fn is_local(self) -> bool {
        matches!(self, CallerOrigin::LocalDesktop)
    }

    /// Whether this is a transport-authenticated remote origin.
    pub fn is_remote(self) -> bool {
        matches!(self, CallerOrigin::AuthenticatedRemote)
    }
}

impl fmt::Display for CallerOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An authenticated caller identity constructed at an adapter boundary.
///
/// It binds the authenticated `actor_id` / `device_id` to the
/// [`PolicyPartition`] the caller operates within, tagged with the
/// [`CallerOrigin`] that admitted it. This is the typed seam the composition
/// root exposes so adapters never pass raw unchecked identity strings across the
/// boundary (parent-task invariant). Core policy consumes this context and MAY
/// narrow the partition but MUST NOT expand it (design §19.8); the context
/// itself carries no capability — it is an identity assertion the adapter has
/// already authenticated.
///
/// Deliberately **not** `Deserialize`: a caller cannot deserialize itself into a
/// trusted context from untrusted input. Adapters construct it explicitly via
/// [`CallerContext::local_desktop`] / [`CallerContext::authenticated_remote`]
/// after authenticating their transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallerContext {
    origin: CallerOrigin,
    actor_id: String,
    device_id: String,
    partition: PolicyPartition,
}

impl CallerContext {
    /// Construct the in-process desktop caller (local, trusted). The desktop is
    /// a single-user laptop, so the authenticated actor *is* the local device;
    /// `device_id` identifies that laptop.
    pub fn local_desktop(
        device_id: impl Into<String>,
        partition: PolicyPartition,
    ) -> MemoryResult<Self> {
        let device_id = device_id.into();
        Self::new(
            CallerOrigin::LocalDesktop,
            device_id.clone(),
            device_id,
            partition,
        )
    }

    /// Construct an authenticated remote caller admitted by the server adapter.
    /// `actor_id` is the authenticated principal (from the transport's verified
    /// identity); `device_id` identifies the originating device.
    pub fn authenticated_remote(
        actor_id: impl Into<String>,
        device_id: impl Into<String>,
        partition: PolicyPartition,
    ) -> MemoryResult<Self> {
        Self::new(
            CallerOrigin::AuthenticatedRemote,
            actor_id,
            device_id,
            partition,
        )
    }

    /// Shared validated constructor: `actor_id` and `device_id` must be
    /// non-empty (an unauthenticated / anonymous caller is unrepresentable).
    fn new(
        origin: CallerOrigin,
        actor_id: impl Into<String>,
        device_id: impl Into<String>,
        partition: PolicyPartition,
    ) -> MemoryResult<Self> {
        let actor_id = actor_id.into();
        let device_id = device_id.into();
        if actor_id.trim().is_empty() {
            return Err(encoding_err("caller actor_id must not be empty"));
        }
        if device_id.trim().is_empty() {
            return Err(encoding_err("caller device_id must not be empty"));
        }
        Ok(Self {
            origin,
            actor_id,
            device_id,
            partition,
        })
    }

    /// The adapter boundary that authenticated this caller.
    pub fn origin(&self) -> CallerOrigin {
        self.origin
    }

    /// The authenticated principal id.
    pub fn actor_id(&self) -> &str {
        &self.actor_id
    }

    /// The originating device id.
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// The policy partition the caller operates within.
    pub fn partition(&self) -> &PolicyPartition {
        &self.partition
    }

    /// Whether this caller was admitted by the local desktop adapter.
    pub fn is_local(&self) -> bool {
        self.origin.is_local()
    }

    /// Whether this caller was authenticated by the remote server adapter.
    pub fn is_remote(&self) -> bool {
        self.origin.is_remote()
    }

    /// The stable canonical partition string (`namespace/scope/sensitivity`).
    pub fn partition_key(&self) -> String {
        self.partition.partition_key()
    }
}

impl fmt::Display for CallerContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}@{}",
            self.origin.as_str(),
            self.actor_id,
            self.partition.partition_key()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_UUID: &str = "018f4e2a-1c3b-7d4e-8f90-abcdef012345";

    // ── UUID identifiers ────────────────────────────────────────────────
    #[test]
    fn uuid_ids_accept_valid_and_roundtrip_serde() {
        let rid = RecordId::new(VALID_UUID).unwrap();
        assert_eq!(rid.as_str(), VALID_UUID);
        let json = serde_json::to_string(&rid).unwrap();
        assert_eq!(json, format!("\"{VALID_UUID}\""));
        let back: RecordId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rid);

        // Generated ids are always valid.
        let e = EventId::new_v7();
        assert!(EventId::new(e.as_str()).is_ok());
        let i = InvocationId::new_v7();
        assert!(InvocationId::new(i.as_str()).is_ok());
    }

    #[test]
    fn uuid_ids_normalize_upper_case() {
        let upper = "018F4E2A-1C3B-7D4E-8F90-ABCDEF012345";
        assert_eq!(RecordId::new(upper).unwrap().as_str(), VALID_UUID);
    }

    #[test]
    fn uuid_ids_reject_non_uuid() {
        assert!(RecordId::new("not-a-uuid").is_err());
        assert!(EventId::new("").is_err());
        assert!(InvocationId::new("018f4e2a1c3b7d4e8f90abcdef012345").is_err());
        // Rejected on deserialize too.
        assert!(serde_json::from_str::<RecordId>("\"garbage\"").is_err());
    }

    // ── IdempotencyKey ──────────────────────────────────────────────────
    #[test]
    fn idempotency_key_accepts_bounded_nonempty() {
        let k = IdempotencyKey::new("cmd-abc-123").unwrap();
        assert_eq!(k.as_str(), "cmd-abc-123");
        let json = serde_json::to_string(&k).unwrap();
        let back: IdempotencyKey = serde_json::from_str(&json).unwrap();
        assert_eq!(back, k);
    }

    #[test]
    fn idempotency_key_rejects_empty_oversized_and_control() {
        assert!(IdempotencyKey::new("").is_err());
        let oversized = "x".repeat(IDEMPOTENCY_KEY_MAX_LEN + 1);
        assert!(IdempotencyKey::new(oversized).is_err());
        assert!(IdempotencyKey::new("bad\nkey").is_err());
        assert!(IdempotencyKey::new("tab\tkey").is_err());
        // Exactly at the limit is accepted.
        let exact = "y".repeat(IDEMPOTENCY_KEY_MAX_LEN);
        assert!(IdempotencyKey::new(exact).is_ok());
    }

    // ── PolicyPartition ─────────────────────────────────────────────────
    #[test]
    fn policy_partition_accepts_valid_and_keys() {
        let p = PolicyPartition::new("user", "chat", 2).unwrap();
        assert_eq!(p.partition_key(), "user/chat/2");
        assert_eq!(p.to_string(), "user/chat/2");
        assert_eq!(p.namespace(), "user");
        assert_eq!(p.scope(), "chat");
        assert_eq!(p.sensitivity(), 2);
        assert_eq!(p.owner_id(), None);

        let owned = PolicyPartition::with_owner("user", "chat", 0, Some("owner-1".into())).unwrap();
        assert_eq!(owned.owner_id(), Some("owner-1"));

        // Round-trip serde with validation.
        let json = serde_json::to_string(&p).unwrap();
        let back: PolicyPartition = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn policy_partition_rejects_bad_sensitivity_and_empty_parts() {
        assert!(PolicyPartition::new("user", "chat", 4).is_err());
        assert!(PolicyPartition::new("", "chat", 0).is_err());
        assert!(PolicyPartition::new("user", "  ", 0).is_err());
        assert!(PolicyPartition::with_owner("user", "chat", 0, Some("".into())).is_err());
        // sensitivity=3 is the max valid value.
        assert!(PolicyPartition::new("user", "chat", 3).is_ok());
        // Rejected on deserialize.
        assert!(serde_json::from_str::<PolicyPartition>(
            "{\"namespace\":\"u\",\"scope\":\"s\",\"sensitivity\":4}"
        )
        .is_err());
    }

    // ── GraphRevision ───────────────────────────────────────────────────
    #[test]
    fn graph_revision_base_semantics() {
        assert_eq!(GraphRevision::new(0).base_revision(), None);
        assert_eq!(
            GraphRevision::new(5).base_revision(),
            Some(GraphRevision::new(4))
        );
        assert_eq!(GraphRevision::base().get(), 0);
        assert_eq!(GraphRevision::new(5).next(), GraphRevision::new(6));
        // serde round-trip.
        let r = GraphRevision::new(42);
        let back: GraphRevision =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }

    // ── UtcTimestamp ────────────────────────────────────────────────────
    #[test]
    fn utc_timestamp_accepts_utc_and_roundtrips() {
        let ts = UtcTimestamp::from_rfc3339_utc("2026-01-01T00:00:00Z").unwrap();
        // Canonical output is +00:00 form.
        assert!(ts.to_rfc3339().ends_with("+00:00"));
        let json = serde_json::to_string(&ts).unwrap();
        let back: UtcTimestamp = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ts);
        // now() is constructible and after the epoch example.
        assert!(UtcTimestamp::now() >= ts);
    }

    #[test]
    fn utc_timestamp_rejects_non_utc_and_garbage() {
        assert!(UtcTimestamp::from_rfc3339_utc("2026-01-01T00:00:00+05:30").is_err());
        assert!(UtcTimestamp::from_rfc3339_utc("2026-01-01 00:00:00").is_err());
        assert!(UtcTimestamp::from_rfc3339_utc("garbage").is_err());
        assert!(UtcTimestamp::from_rfc3339_utc("").is_err());
        // Rejected on deserialize.
        assert!(serde_json::from_str::<UtcTimestamp>("\"garbage\"").is_err());
    }

    // ── ValidInterval ───────────────────────────────────────────────────
    #[test]
    fn valid_interval_accepts_ordered_and_open() {
        let from = UtcTimestamp::from_rfc3339_utc("2026-01-01T00:00:00Z").unwrap();
        let until = UtcTimestamp::from_rfc3339_utc("2026-06-01T00:00:00Z").unwrap();
        let iv = ValidInterval::new(Some(from), Some(until)).unwrap();
        assert!(!iv.is_open());

        let mid = UtcTimestamp::from_rfc3339_utc("2026-03-01T00:00:00Z").unwrap();
        assert!(iv.contains(mid));
        assert!(iv.contains(from)); // inclusive lower bound
        assert!(!iv.contains(until)); // exclusive upper bound

        let open = ValidInterval::open();
        assert!(open.is_open());
        assert!(open.contains(mid));

        // serde round-trip.
        let back: ValidInterval =
            serde_json::from_str(&serde_json::to_string(&iv).unwrap()).unwrap();
        assert_eq!(back, iv);
    }

    #[test]
    fn valid_interval_rejects_inverted() {
        let from = UtcTimestamp::from_rfc3339_utc("2026-06-01T00:00:00Z").unwrap();
        let until = UtcTimestamp::from_rfc3339_utc("2026-01-01T00:00:00Z").unwrap();
        assert!(ValidInterval::new(Some(from), Some(until)).is_err());
        // Deserialize also validates.
        let json = format!(
            "{{\"valid_from\":\"{}\",\"valid_until\":\"{}\"}}",
            from.to_rfc3339(),
            until.to_rfc3339()
        );
        assert!(serde_json::from_str::<ValidInterval>(&json).is_err());
    }

    // ── Schema / version ────────────────────────────────────────────────
    #[test]
    fn schema_and_version_numbers() {
        assert_eq!(SchemaVersion::new(2).get(), 2);
        assert_eq!(SchemaVersion::new(2).to_string(), "2");
        assert_eq!(Version::first().get(), 1);
        assert_eq!(Version::first().next(), Version::new(2));
        let sv = SchemaVersion::new(7);
        let back: SchemaVersion =
            serde_json::from_str(&serde_json::to_string(&sv).unwrap()).unwrap();
        assert_eq!(back, sv);
    }

    // ── CallerContext ───────────────────────────────────────────────────
    #[test]
    fn caller_context_local_desktop_is_trusted_local() {
        let partition = PolicyPartition::new("user", "chat", 0).unwrap();
        let caller = CallerContext::local_desktop("local-desktop", partition.clone()).unwrap();

        assert_eq!(caller.origin(), CallerOrigin::LocalDesktop);
        assert!(caller.is_local());
        assert!(!caller.is_remote());
        // Local caller: the authenticated actor IS the device.
        assert_eq!(caller.actor_id(), "local-desktop");
        assert_eq!(caller.device_id(), "local-desktop");
        assert_eq!(caller.partition(), &partition);
        assert_eq!(caller.partition_key(), "user/chat/0");
        assert_eq!(caller.origin().as_str(), "local_desktop");
    }

    #[test]
    fn caller_context_authenticated_remote_is_remote() {
        let partition = PolicyPartition::new("user", "remote", 1).unwrap();
        let caller =
            CallerContext::authenticated_remote("phone-abc", "device-xyz", partition.clone())
                .unwrap();

        assert_eq!(caller.origin(), CallerOrigin::AuthenticatedRemote);
        assert!(caller.is_remote());
        assert!(!caller.is_local());
        assert_eq!(caller.actor_id(), "phone-abc");
        assert_eq!(caller.device_id(), "device-xyz");
        assert_eq!(caller.partition(), &partition);
        assert_eq!(caller.origin().as_str(), "authenticated_remote");
    }

    #[test]
    fn caller_context_distinct_adapters_construct_distinct_origins() {
        // The same single-user partition, admitted at two different adapter
        // boundaries, yields two DISTINCT callers (different origin) — this is
        // the "distinct authenticated caller construction at adapter boundaries"
        // the composition root preserves.
        let partition = PolicyPartition::new("user", "chat", 0).unwrap();
        let desktop = CallerContext::local_desktop("local-desktop", partition.clone()).unwrap();
        let remote =
            CallerContext::authenticated_remote("local-desktop", "local-desktop", partition)
                .unwrap();
        assert_ne!(desktop, remote);
        assert_ne!(desktop.origin(), remote.origin());
    }

    #[test]
    fn caller_context_rejects_empty_identity() {
        let partition = PolicyPartition::new("user", "chat", 0).unwrap();
        assert!(CallerContext::local_desktop("", partition.clone()).is_err());
        assert!(CallerContext::local_desktop("   ", partition.clone()).is_err());
        assert!(CallerContext::authenticated_remote("", "device", partition.clone()).is_err());
        assert!(CallerContext::authenticated_remote("actor", "", partition).is_err());
    }

    #[test]
    fn caller_context_display_and_serialize() {
        let partition = PolicyPartition::new("user", "chat", 2).unwrap();
        let caller = CallerContext::local_desktop("local-desktop", partition).unwrap();
        assert_eq!(
            caller.to_string(),
            "local_desktop:local-desktop@user/chat/2"
        );
        // Serializable for audit/logging (identity assertion), origin as snake_case.
        let json = serde_json::to_value(&caller).unwrap();
        assert_eq!(json["origin"], "local_desktop");
        assert_eq!(json["actor_id"], "local-desktop");
    }
}
