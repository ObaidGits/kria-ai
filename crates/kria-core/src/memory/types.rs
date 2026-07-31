//! Core domain types for the cognitive memory system (memory-upgrade design §17).
//!
//! Persisted classification enums serialize as **strings with an `Other(String)`
//! fallback** (design §40 / R25): an older binary reading newer data never panics
//! and preserves unknown values on rewrite. The [`string_enum!`] macro generates
//! that behavior uniformly.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ids::{Hlc, Timestamp};

/// Generate a forward-compatible, string-serialized enum with an `Other(String)`
/// catch-all so unknown values round-trip instead of failing (design §40).
macro_rules! string_enum {
    (
        $(#[$outer:meta])*
        $vis:vis enum $name:ident { $($variant:ident => $s:literal),+ $(,)? }
    ) => {
        $(#[$outer])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        $vis enum $name {
            $($variant,)+
            /// Forward-compat: an unrecognized value read from storage or an API.
            Other(String),
        }

        impl $name {
            /// The canonical wire string for this value.
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $s,)+
                    Self::Other(s) => s.as_str(),
                }
            }

            /// All known (non-`Other`) variants — handy for enumeration/UI.
            pub fn known() -> &'static [&'static str] {
                &[$($s,)+]
            }
        }

        impl std::str::FromStr for $name {
            type Err = std::convert::Infallible;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(match s {
                    $($s => Self::$variant,)+
                    other => Self::Other(other.to_string()),
                })
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
                ser.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
                let s = String::deserialize(de)?;
                // FromStr is infallible (unknown → Other).
                Ok(s.parse().unwrap())
            }
        }
    };
}

string_enum! {
    /// The memory taxonomy (design §3 / architecture §3).
    pub enum MemoryType {
        Working => "working",
        ShortTerm => "short_term",
        Episodic => "episodic",
        Semantic => "semantic",
        Procedural => "procedural",
        Goal => "goal",
        Reflection => "reflection",
        Failure => "failure",
        ReasoningTrace => "reasoning_trace",
        WorldModel => "world_model",
        UserProfile => "user_profile",
        Capability => "capability",
        Workspace => "workspace",
        DesktopContext => "desktop_context",
        Library => "library",
    }
}

string_enum! {
    /// Memory lifecycle state (design §13.1 FSM).
    pub enum MemoryState {
        Active => "active",
        Promoted => "promoted",
        Compressed => "compressed",
        Archived => "archived",
        Superseded => "superseded",
        Forgotten => "forgotten",
        Deleted => "deleted",
    }
}

string_enum! {
    /// Re-verification class (governs re-verification, not deletion — design §22.4).
    pub enum StalenessClass {
        Immutable => "immutable",
        Permanent => "permanent",
        Slow => "slow",
        VolatileVerifiable => "volatile_verifiable",
        VolatileUnverifiable => "volatile_unverifiable",
    }
}

string_enum! {
    /// Content sensitivity (design §47.3 classifier).
    pub enum Sensitivity {
        Public => "public",
        Private => "private",
        Secret => "secret",
    }
}

string_enum! {
    /// Memory mode, enforced at the write gate (design §23) and the redesign's
    /// canonical mode gate ([`crate::memory::modes`], task F1.4.4).
    ///
    /// MGR-035 / the glossary define **five canonical `Memory_Mode` classes** —
    /// `Permanent`, `Temporary`, `Session_Only`, `Read_Only`, `Disabled`. Every
    /// variant below maps onto exactly one of those classes via
    /// [`MemoryMode::class`](crate::memory::modes::ModeClass); the extra
    /// variants are finer-grained product modes that share a canonical class's
    /// admission/read/purge semantics (e.g. `Developer`/`Research` are
    /// `Permanent`-class, `Incognito`/`Guest` are `Read_Only`-class). An unknown
    /// `Other` value has **no** class and fails closed (no durable fallback).
    pub enum MemoryMode {
        Permanent => "permanent",
        Temporary => "temporary",
        SessionOnly => "session_only",
        Incognito => "incognito",
        Workspace => "workspace",
        LibraryOnly => "library_only",
        ReadOnly => "read_only",
        Disabled => "disabled",
        Guest => "guest",
        Developer => "developer",
        Benchmark => "benchmark",
        Safe => "safe",
        Research => "research",
    }
}

string_enum! {
    /// Knowledge partition for isolation + selective sharing (design §12 / L7).
    pub enum Scope {
        Global => "global",
        Company => "company",
        Client => "client",
        Workspace => "workspace",
        Session => "session",
        Personal => "personal",
    }
}

string_enum! {
    /// Content modality (text now; others reserved for future — design §3).
    pub enum Modality {
        Text => "text",
        Image => "image",
        Audio => "audio",
        Video => "video",
    }
}

string_enum! {
    /// Availability of an optional dependency (embedder/LLM/index). Drives the
    /// L8 degradation ladder.
    pub enum Availability {
        Up => "up",
        Degraded => "degraded",
        Down => "down",
    }
}

/// Provenance of the originating content/actor for a write (design §46.1 maps
/// the tool layer's `TriggerProvenance` onto this). Serialized as a tagged
/// string like `tool:file_ops`, `mcp:github:search`, `openclaw:pdf-skill`.
///
/// **Superseded by** the structured [`crate::memory::model::Provenance`] +
/// [`crate::memory::model::SourceRef`] (canonical v2 provenance). Retained as
/// the live write-path provenance tag until the F1.5 write cutover; see the
/// ledger in [`crate::memory::model::legacy_mapping`] (task F2.1.6).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// Directly stated by the user (highest reliability).
    User,
    /// A native tool outcome (`tool:{name}`).
    Tool(String),
    /// An MCP server tool outcome (`mcp:{server}:{tool}`).
    Mcp { server: String, tool: String },
    /// An OpenClaw skill outcome (`openclaw:{skill}`).
    OpenClaw(String),
    /// A Python sidecar module (`sidecar:{module}`).
    Sidecar(String),
    /// KRIA's own reflection/consolidation (untrusted, L11).
    SelfReflection,
    /// Extracted from a library item (`library:{item}:chunk:{idx}`).
    Library { item: Uuid, chunk: u32 },
    /// External fetched content (web/doc) — untrusted data (injection wall).
    ExternalContent(String),
    /// A data import bundle (`import`).
    Import,
    /// Any other/opaque source.
    Other(String),
}

impl Source {
    /// Reliability weight for importance/confidence scoring (design §22.1).
    pub fn authority(&self) -> f32 {
        match self {
            Source::User => 1.0,
            Source::Tool(_) | Source::Mcp { .. } | Source::Sidecar(_) => 0.8,
            Source::Library { .. } | Source::ExternalContent(_) => 0.6,
            Source::OpenClaw(_) => 0.6,
            Source::SelfReflection => 0.5,
            Source::Import => 0.5,
            Source::Other(_) => 0.4,
        }
    }

    /// Whether content from this source must pass the injection wall as
    /// untrusted data (design §46.1 / D-11).
    pub fn is_untrusted_content(&self) -> bool {
        matches!(
            self,
            Source::ExternalContent(_)
                | Source::Library { .. }
                | Source::OpenClaw(_)
                | Source::Import
        )
    }

    /// The `source:` provenance tag stored on events/memories (design §46.1).
    pub fn tag(&self) -> String {
        match self {
            Source::User => "user".to_string(),
            Source::Tool(n) => format!("tool:{n}"),
            Source::Mcp { server, tool } => format!("mcp:{server}:{tool}"),
            Source::OpenClaw(s) => format!("openclaw:{s}"),
            Source::Sidecar(m) => format!("sidecar:{m}"),
            Source::SelfReflection => "self_reflection".to_string(),
            Source::Library { item, chunk } => format!("library:{item}:chunk:{chunk}"),
            Source::ExternalContent(k) => format!("external:{k}"),
            Source::Import => "import".to_string(),
            Source::Other(s) => s.clone(),
        }
    }
}

impl Serialize for Source {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.tag())
    }
}

/// A distinct embedding model version; vectors from different versions are
/// never compared (architecture §9 / C4). Backs the version-partitioned tables.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelVersion(pub String);

impl ModelVersion {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModelVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Success/failure utility signal for a memory (design §22.3, Memory Worth).
/// A soft re-rank + archival hint — never a hard-delete trigger (D-8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryWorth {
    pub success: u32,
    pub failure: u32,
    pub samples: u32,
}

impl MemoryWorth {
    /// Minimum observations before Memory Worth may influence ranking/archival
    /// (design D-8).
    pub const MIN_SAMPLES: u32 = 20;

    /// Whether this signal has enough evidence to be trusted.
    pub fn is_significant(&self) -> bool {
        self.samples >= Self::MIN_SAMPLES
    }

    /// Net worth in [-1, 1]; 0 when not yet significant.
    pub fn score(&self) -> f32 {
        if !self.is_significant() {
            return 0.0;
        }
        let total = (self.success + self.failure).max(1) as f32;
        (self.success as f32 - self.failure as f32) / total
    }
}

/// A predicate that lets a memory be re-verified against a live source
/// (filesystem path, tool, git) — design §22.4.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum VerifyPredicate {
    Path(String),
    Git(String),
    Tool(String),
}

/// User emphasis signals extracted from input, feeding the importance score
/// (design §22.1).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EmphasisSignals {
    pub explicit_remember: bool,
    pub repetition: u32,
    pub marker_terms: Vec<String>,
}

/// An immutable, append-only event — the source of truth (design §11/§14, L1).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: Uuid,
    pub hlc: Hlc,
    pub ts_utc: Timestamp,
    /// Originating timezone offset in minutes (design §14 / N10).
    pub tz_offset_min: i16,
    pub event_type: EventType,
    #[serde(with = "source_serde")]
    pub source: Source,
    pub session_id: Option<Uuid>,
    pub parent_event_id: Option<Uuid>,
    pub shred_key_id: Option<String>,
    pub payload: serde_json::Value,
    pub encrypted: bool,
    /// BLAKE3 of the (pre-encryption) payload.
    pub checksum: String,
}

/// Helper so `Source` deserializes from its tag string on events.
mod source_serde {
    use super::Source;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(src: &Source, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&src.tag())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Source, D::Error> {
        let tag = String::deserialize(d)?;
        Ok(parse_source_tag(&tag))
    }

    /// Parse a `source:` tag back into a [`Source`] (inverse of `Source::tag`).
    pub fn parse_source_tag(tag: &str) -> Source {
        match tag {
            "user" => Source::User,
            "self_reflection" => Source::SelfReflection,
            "import" => Source::Import,
            other => {
                if let Some(rest) = other.strip_prefix("tool:") {
                    Source::Tool(rest.to_string())
                } else if let Some(rest) = other.strip_prefix("mcp:") {
                    match rest.split_once(':') {
                        Some((server, tool)) => Source::Mcp {
                            server: server.to_string(),
                            tool: tool.to_string(),
                        },
                        None => Source::Other(other.to_string()),
                    }
                } else if let Some(rest) = other.strip_prefix("openclaw:") {
                    Source::OpenClaw(rest.to_string())
                } else if let Some(rest) = other.strip_prefix("sidecar:") {
                    Source::Sidecar(rest.to_string())
                } else if let Some(rest) = other.strip_prefix("external:") {
                    Source::ExternalContent(rest.to_string())
                } else {
                    Source::Other(other.to_string())
                }
            }
        }
    }
}

pub use source_serde::parse_source_tag;

string_enum! {
    /// Event-type taxonomy (design §11). Extensible via `Other`.
    pub enum EventType {
        Observation => "observation",
        DesktopContext => "desktop_context",
        WorkspaceState => "workspace_state",
        FileEvent => "file_event",
        UserMessage => "user_message",
        AssistantMessage => "assistant_message",
        ToolInvocation => "tool_invocation",
        ToolOutcome => "tool_outcome",
        MemoryCreated => "memory_created",
        MemorySuperseded => "memory_superseded",
        MemoryMerged => "memory_merged",
        MemorySplit => "memory_split",
        MemoryPromoted => "memory_promoted",
        MemoryDemoted => "memory_demoted",
        MemoryArchived => "memory_archived",
        MemoryForgotten => "memory_forgotten",
        MemoryDeleted => "memory_deleted",
        MemoryRestored => "memory_restored",
        ReflectionProduced => "reflection_produced",
        ConsolidationRun => "consolidation_run",
        EpisodeClosed => "episode_closed",
        EntityMerged => "entity_merged",
        EntitySplit => "entity_split",
        Feedback => "feedback",
        ModeSwitched => "mode_switched",
        ContradictionFlagged => "contradiction_flagged",
        KnowledgeGapRecorded => "knowledge_gap_recorded",
        LibraryIngested => "library_ingested",
        LibraryVersioned => "library_versioned",
        LibraryDeleted => "library_deleted",
        MigrationApplied => "migration_applied",
        ReconcileRun => "reconcile_run",
    }
}

/// A derived, durable, mutable knowledge unit (design §12/§17, L4).
///
/// **Superseded by** [`crate::memory::model::Record`]
/// (`RecordKind::Memory`) + [`crate::memory::model::Provenance`] (canonical v2
/// cognitive record). Retained as the live persistence/retrieval row until the
/// F1.5 write cutover + F3 retrieval-on-v2; see the ledger in
/// [`crate::memory::model::legacy_mapping`] (task F2.1.6).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Memory {
    pub id: Uuid,
    pub content: String,
    pub memory_type: MemoryType,
    /// 0 raw → 1 episode → 2 skill → 3 rule (terminal). Design §20.
    pub compression_level: u8,
    pub source_event_id: Uuid,
    pub namespace: String,
    pub owner_id: String,
    pub device_id: String,
    pub scope: Scope,
    pub confidence: f32,
    pub importance: f32,
    pub access_count: u64,
    pub decay_score: f32,
    pub staleness_class: StalenessClass,
    pub sensitivity: Sensitivity,
    pub state: MemoryState,
    pub created_at: Timestamp,
    pub last_accessed: Option<Timestamp>,
    pub valid_from: Timestamp,
    pub valid_until: Option<Timestamp>,
    pub embedding_id: Option<Uuid>,
    pub embedding_model_version: Option<ModelVersion>,
    pub estimated_tokens: u32,
    pub content_hash: String,
    pub shred_key_id: Option<String>,
    pub verify_against: Option<VerifyPredicate>,
    pub superseded_by: Option<Uuid>,
    pub episode_id: Option<Uuid>,
    pub goal_context_id: Option<Uuid>,
    pub worth: MemoryWorth,
    pub modality: Modality,
    pub preference_pair_id: Option<String>,
    pub training_eligible: bool,
}

/// What a subsystem submits to the Write Policy Engine — never a direct write
/// (design §17, L3).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WriteCandidate {
    pub content: String,
    pub proposed_type: Option<MemoryType>,
    #[serde(with = "source_serde")]
    pub source: Source,
    pub session_id: Uuid,
    pub namespace_hint: Option<String>,
    pub scope_hint: Option<Scope>,
    pub sensitivity_hint: Option<Sensitivity>,
    #[serde(default)]
    pub emphasis: EmphasisSignals,
    pub verify_against: Option<VerifyPredicate>,
    #[serde(default)]
    pub derived_from: Vec<Uuid>,
}

impl WriteCandidate {
    /// Minimal constructor for a user-stated memory.
    pub fn user(session_id: Uuid, content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            proposed_type: None,
            source: Source::User,
            session_id,
            namespace_hint: None,
            scope_hint: None,
            sensitivity_hint: None,
            emphasis: EmphasisSignals::default(),
            verify_against: None,
            derived_from: Vec::new(),
        }
    }

    /// A system/global write NOT tied to any conversation session (M2). Uses a
    /// FRESH per-write id so session-scoped consolidation, `Temporary` purge, and
    /// session analytics never collapse unrelated system writes (tools, library
    /// chunks, cold-start imports, explicit "remember this") into one giant
    /// pseudo-session — the defect the old fixed sentinel session-UUIDs caused.
    /// Scoped `Global`. Callers set `source` afterwards for provenance.
    pub fn global(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            proposed_type: None,
            source: Source::User,
            session_id: crate::memory::ids::new_id(),
            namespace_hint: None,
            scope_hint: Some(Scope::Global),
            sensitivity_hint: None,
            emphasis: EmphasisSignals::default(),
            verify_against: None,
            derived_from: Vec::new(),
        }
    }
}

/// The outcome the Write Policy Engine returns to the caller (design §17).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum WriteDecision {
    /// Derived memory committed.
    Stored { memory_id: Uuid },
    /// Merged into an existing duplicate.
    Deduped { into: Uuid },
    /// Buffered for a later idle flush.
    Batched,
    /// Raw event durable; enrichment queued (the common fast-path result).
    Queued { event_id: Uuid },
    /// Rejected by policy (a normal outcome, not an error).
    Rejected { reason: RejectReason },
    /// Held pending user confirmation (secret/high-impact).
    NeedsConfirmation { token: String },
}

/// Why the Write Policy Engine rejected a candidate (design §17/§18).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", content = "detail", rename_all = "snake_case")]
pub enum RejectReason {
    Mode(MemoryMode),
    QualityFilter,
    SecurityScan(String),
    NamespaceViolation,
    FalsePromotionGuard,
    Contradiction(Uuid),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_enum_roundtrips_and_forward_compat() {
        // Known value round-trips.
        let json = serde_json::to_string(&MemoryType::Semantic).unwrap();
        assert_eq!(json, "\"semantic\"");
        let back: MemoryType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, MemoryType::Semantic);

        // Unknown value is preserved, not an error (design §40 / R25).
        let unknown: MemoryType = serde_json::from_str("\"quantum_memory\"").unwrap();
        assert_eq!(unknown, MemoryType::Other("quantum_memory".to_string()));
        assert_eq!(
            serde_json::to_string(&unknown).unwrap(),
            "\"quantum_memory\""
        );
    }

    #[test]
    fn global_writes_are_fresh_and_global_scoped() {
        // M2: global/system writes must NOT share a session (which would collapse
        // unrelated writes into one pseudo-session for consolidation/purge) and
        // must be Global-scoped.
        let a = WriteCandidate::global("fact one");
        let b = WriteCandidate::global("fact two");
        assert_ne!(
            a.session_id, b.session_id,
            "global writes must get distinct fresh sessions"
        );
        assert!(matches!(a.scope_hint, Some(Scope::Global)));
        assert!(matches!(b.scope_hint, Some(Scope::Global)));
    }

    #[test]
    fn source_tag_roundtrips() {
        let cases = [
            Source::User,
            Source::Tool("file_ops".into()),
            Source::Mcp {
                server: "github".into(),
                tool: "search".into(),
            },
            Source::OpenClaw("pdf".into()),
            Source::SelfReflection,
        ];
        for s in cases {
            assert_eq!(parse_source_tag(&s.tag()), s);
        }
    }

    #[test]
    fn memory_worth_gates_on_min_samples() {
        let low = MemoryWorth {
            success: 5,
            failure: 0,
            samples: 5,
        };
        assert!(!low.is_significant());
        assert_eq!(low.score(), 0.0);

        let ok = MemoryWorth {
            success: 18,
            failure: 2,
            samples: 20,
        };
        assert!(ok.is_significant());
        assert!(ok.score() > 0.7);
    }
}

// ── Storage-port supporting types (design §16) ─────────────────────────────

/// A graph entity (person/project/tool/concept/…). Design §12.
///
/// **Superseded by** [`crate::memory::model::Entity`] (`entities_v2`) +
/// [`crate::memory::model::Alias`]/[`crate::memory::model::Mention`] (canonical
/// v2 graph identity). Retained as the live `GraphStore` read/write model until
/// F2.2 (relation registry canonical) + F1.5; see the ledger in
/// [`crate::memory::model::legacy_mapping`] (task F2.1.6).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: Uuid,
    pub canonical_id: Uuid,
    pub entity_type: String,
    pub display_name: String,
    pub created_at: Timestamp,
}

// `Relationship` (legacy free-text edge) and `GraphHit` (legacy traversal result)
// were deleted in task F2.2.7. The canonical replacements are:
//   - `relationships_v2` table + registry-governed write path (F2.2.3–2.2.5)
//   - F3.3 traversal result over `crate::memory::model::Entity` (v2)
// See `crate::memory::model::legacy_mapping` for the cutover ledger.

/// Which derived index an outbox entry targets (design §14/§16).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IndexTarget {
    LanceDb,
    Tantivy,
    Fts,
}

impl IndexTarget {
    pub fn as_str(&self) -> &'static str {
        match self {
            IndexTarget::LanceDb => "lancedb",
            IndexTarget::Tantivy => "tantivy",
            IndexTarget::Fts => "fts",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "lancedb" => Some(IndexTarget::LanceDb),
            "tantivy" => Some(IndexTarget::Tantivy),
            "fts" => Some(IndexTarget::Fts),
            _ => None,
        }
    }
}

/// Outbox operation kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutboxOp {
    Upsert,
    Delete,
}

impl OutboxOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            OutboxOp::Upsert => "upsert",
            OutboxOp::Delete => "delete",
        }
    }
}

/// Outbox entry lifecycle (design §13.3 FSM).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutboxStatus {
    Pending,
    Done,
    DeadLetter,
}

impl OutboxStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            OutboxStatus::Pending => "pending",
            OutboxStatus::Done => "done",
            OutboxStatus::DeadLetter => "deadletter",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(OutboxStatus::Pending),
            "done" => Some(OutboxStatus::Done),
            "deadletter" => Some(OutboxStatus::DeadLetter),
            _ => None,
        }
    }
}

/// A transactional-outbox entry: an index update queued inside the authority
/// transaction and relayed idempotently (design §14/D-5).
#[derive(Clone, Debug, PartialEq)]
pub struct OutboxEntry {
    pub id: i64,
    pub memory_id: Uuid,
    pub index_target: IndexTarget,
    pub op: OutboxOp,
    pub content_hash: String,
    pub attempts: u32,
    pub status: OutboxStatus,
    pub created_at: Timestamp,
    /// When set, the relay must not apply this entry before this UTC instant
    /// (exponential backoff gate, task 1.8.4). `None` = eligible immediately.
    pub next_attempt_at: Option<Timestamp>,
    /// The last recorded failure reason (dead-letter observability, task 1.8.4).
    pub error_code: Option<String>,
}

impl OutboxEntry {
    /// A fresh pending upsert (id assigned by the DB on insert).
    pub fn upsert(memory_id: Uuid, target: IndexTarget, content_hash: impl Into<String>) -> Self {
        Self {
            id: 0,
            memory_id,
            index_target: target,
            op: OutboxOp::Upsert,
            content_hash: content_hash.into(),
            attempts: 0,
            status: OutboxStatus::Pending,
            created_at: chrono::Utc::now(),
            next_attempt_at: None,
            error_code: None,
        }
    }

    /// A fresh pending delete.
    pub fn delete(memory_id: Uuid, target: IndexTarget) -> Self {
        Self {
            id: 0,
            memory_id,
            index_target: target,
            op: OutboxOp::Delete,
            content_hash: String::new(),
            attempts: 0,
            status: OutboxStatus::Pending,
            created_at: chrono::Utc::now(),
            next_attempt_at: None,
            error_code: None,
        }
    }
}

/// A Write Policy decision recorded to the audit log (design §28/§45.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditDecision {
    Stored,
    Rejected,
    Deduped,
    Batched,
}

impl AuditDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuditDecision::Stored => "stored",
            AuditDecision::Rejected => "rejected",
            AuditDecision::Deduped => "deduped",
            AuditDecision::Batched => "batched",
        }
    }
}

/// One row in the memory-audit log.
#[derive(Clone, Debug)]
pub struct AuditRecord {
    pub id: Uuid,
    pub ts: Timestamp,
    pub decision: AuditDecision,
    pub reason: String,
    pub candidate_hash: Option<String>,
    pub namespace: Option<String>,
    pub mode: Option<MemoryMode>,
}

/// Payload columns stored alongside a vector for pre-filtering (design §15).
#[derive(Clone, Debug, PartialEq)]
pub struct VectorPayload {
    pub namespace: String,
    pub scope: Scope,
    pub sensitivity: Sensitivity,
    pub memory_type: MemoryType,
    pub content_hash: String,
    pub created_at: Timestamp,
}

/// A vector-search hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VectorHit {
    pub id: Uuid,
    pub score: f32,
}

/// A full-text-search hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SearchHit {
    pub id: Uuid,
    pub score: f32,
}

/// Mandatory scope/namespace/sensitivity filter applied at retrieval (L7/D-20).
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeFilter {
    /// Allowed namespaces; empty = no namespace restriction.
    pub namespaces: Vec<String>,
    /// Allowed scopes; empty = no scope restriction (global always allowed).
    pub scopes: Vec<Scope>,
    /// Whether `secret`-sensitivity memories may be returned.
    pub include_secret: bool,
}

impl Default for ScopeFilter {
    fn default() -> Self {
        Self {
            namespaces: Vec::new(),
            scopes: Vec::new(),
            include_secret: false,
        }
    }
}

impl ScopeFilter {
    /// Whether a memory with the given attributes passes this filter (L7).
    pub fn allows(&self, namespace: &str, scope: &Scope, sensitivity: &Sensitivity) -> bool {
        if !self.include_secret && *sensitivity == Sensitivity::Secret {
            return false;
        }
        if !self.namespaces.is_empty() && !self.namespaces.iter().any(|n| n == namespace) {
            return false;
        }
        if !self.scopes.is_empty()
            && *scope != Scope::Global
            && !self.scopes.iter().any(|s| s == scope)
        {
            return false;
        }
        true
    }
}
