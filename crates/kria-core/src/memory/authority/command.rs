//! The command envelope — the validated, self-describing unit the authority
//! admits (task **F1.3.1**, design §5.1 command state machine, §4.1
//! `events_v2`/`idempotency_results`).
//!
//! Everything the authority needs to *decide* on a durable command travels in
//! one [`CommandEnvelope`]: the authenticated [`CallerContext`], the semantic
//! operation ([`CommandKind`] + canonical `payload`), the caller-chosen
//! [`IdempotencyKey`] paired with a deterministic [`CommandHash`] over the
//! semantic content, the [`GraphRevision`] the caller issued against, the
//! provenance [`SourceContext`] (invocation id + source kind/id/trust) used to
//! correlate start/completion events, the admission [`MemoryMode`], a bounded
//! execution [`Deadline`], and — for destructive/corrective commands — the
//! [`PreviewToken`] that confirms a previously previewed impact against a base
//! revision.
//!
//! ## Scope of this module (F1.3.1 only)
//!
//! This defines the envelope **types** and the **canonical command-hash**
//! computation, with the validation of required fields. It does **not** open a
//! transaction, append events, reserve a revision, write audit/idempotency, or
//! publish the outbox — those are F1.3.2–F1.3.7. The transaction path
//! ([`crate::memory::authority`]) consumes an already-validated envelope.
//!
//! ## Canonical command hash (design §4.1 `idempotency_results.command_hash`)
//!
//! `idempotency_results` is keyed by `(caller_partition, idempotency_key)` and
//! stores a `command_hash`; a replay that reuses a key with a *different* hash
//! is a conflict rather than a replay (MGR-005 AC3). The hash must therefore be
//! a **deterministic, order-independent** digest over the command's *semantic*
//! content and must exclude per-attempt / execution-budget fields
//! (invocation id, deadline, preview token, the idempotency key itself). See
//! [`CommandHash::compute`].

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::ids::blake3_hex;
use crate::memory::model::{CallerContext, GraphRevision, IdempotencyKey, InvocationId};
use crate::memory::types::MemoryMode;

pub use crate::memory::authority::CommandKind;

/// Build a canonical-encoding validation error (`StorageError::Encoding`).
fn encoding_err(msg: impl Into<String>) -> crate::memory::error::MemoryError {
    StorageError::Encoding(msg.into()).into()
}

/// Version tag mixed into the canonical command-hash preimage. Bump only if the
/// hashed field set changes, so old idempotency rows never silently collide
/// with a new hashing scheme.
const COMMAND_HASH_SCHEMA: u32 = 1;

// ─────────────────────────────────────────────────────────────────────────
// SourceKind — provenance origin (events_v2.source_kind / sources.source_kind)
// ─────────────────────────────────────────────────────────────────────────

/// The kind of source a command originates from (design §4.1 `sources`
/// `source_kind CHECK(native/mcp/openclaw/sidecar/import/library/conversation)`;
/// `events_v2.source_kind`). A closed set so `source_kind` can never be a raw
/// unchecked string at the authority boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    /// In-process native tool / core subsystem.
    Native,
    /// A Model Context Protocol server.
    Mcp,
    /// A sandboxed OpenClaw skill.
    OpenClaw,
    /// A local sidecar process.
    Sidecar,
    /// An interchange / bulk import.
    Import,
    /// A library / document corpus ingestion.
    Library,
    /// A conversation turn.
    Conversation,
}

impl SourceKind {
    /// The canonical text form stored in `source_kind` columns.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Native => "native",
            SourceKind::Mcp => "mcp",
            SourceKind::OpenClaw => "openclaw",
            SourceKind::Sidecar => "sidecar",
            SourceKind::Import => "import",
            SourceKind::Library => "library",
            SourceKind::Conversation => "conversation",
        }
    }

    /// Whether a command from this source represents an *active invocation* that
    /// gets a start/completion Event pair (design §5.1 command state machine,
    /// §19.4 "start/completion event pair for invocations"), as opposed to a
    /// passive ingestion / turn that records a single observation/completion
    /// Event and needs no separate start marker.
    ///
    /// Tool-like sources — [`SourceKind::Native`], [`SourceKind::Mcp`],
    /// [`SourceKind::OpenClaw`], [`SourceKind::Sidecar`] — are invocations: they
    /// execute an action whose beginning and end are both auditable. Ingestion /
    /// turn sources ([`SourceKind::Import`], [`SourceKind::Library`],
    /// [`SourceKind::Conversation`]) are not — they produce their event(s)
    /// without a distinct "started" phase.
    pub fn is_invocation(self) -> bool {
        matches!(
            self,
            SourceKind::Native | SourceKind::Mcp | SourceKind::OpenClaw | SourceKind::Sidecar
        )
    }
}

impl std::str::FromStr for SourceKind {
    type Err = crate::memory::error::MemoryError;

    /// Parse the canonical `source_kind` text; rejects anything outside the
    /// schema's `CHECK` set.
    fn from_str(s: &str) -> MemoryResult<Self> {
        Ok(match s {
            "native" => SourceKind::Native,
            "mcp" => SourceKind::Mcp,
            "openclaw" => SourceKind::OpenClaw,
            "sidecar" => SourceKind::Sidecar,
            "import" => SourceKind::Import,
            "library" => SourceKind::Library,
            "conversation" => SourceKind::Conversation,
            other => return Err(encoding_err(format!("unknown source_kind {other:?}"))),
        })
    }
}

impl std::fmt::Display for SourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for SourceKind {
    /// Emit the canonical `source_kind` text ([`SourceKind::as_str`]) so the
    /// JSON wire form matches the schema `CHECK` set and the custom
    /// [`Deserialize`]/[`std::str::FromStr`] (a derived `snake_case` rename would
    /// emit `"open_claw"` for [`SourceKind::OpenClaw`], which
    /// deserialization/SQL would then reject — SQL↔Rust↔API must agree).
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SourceKind {
    /// Parse the canonical `source_kind` text (rejecting anything outside the
    /// schema `CHECK` set), consistent with [`SourceKind`]'s [`std::str::FromStr`]
    /// and derived [`Serialize`].
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// ─────────────────────────────────────────────────────────────────────────
// SourceTrust — coarse provenance trust tier
// ─────────────────────────────────────────────────────────────────────────

/// The coarse trust tier a command's source carries at admission time (design
/// §7.4 "source-specific … trust", `sources.trust_class`). This is the
/// *envelope-level* provenance tier the command travels with; the full
/// Effective-Policy source-trust lattice (associative meet, capability
/// intersection) is defined and enforced in **F1.4** (`policy/source_trust`).
///
/// Ordered from most to least trusted so admission can compare tiers:
/// `System > Trusted > Limited > Untrusted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTrust {
    /// The local core / native subsystem — implicitly trusted on the laptop.
    System,
    /// An authenticated, capability-scoped source (e.g. a vetted MCP server).
    Trusted,
    /// A capability-limited source (e.g. a limited-trust OpenClaw skill).
    Limited,
    /// External / unvetted content (e.g. imported material, self-reflection);
    /// treated as untrusted until independently verified (design §7.3).
    Untrusted,
}

impl SourceTrust {
    /// The canonical text form (stable for audit/logging and `trust_class`).
    pub fn as_str(self) -> &'static str {
        match self {
            SourceTrust::System => "system",
            SourceTrust::Trusted => "trusted",
            SourceTrust::Limited => "limited",
            SourceTrust::Untrusted => "untrusted",
        }
    }
}

impl std::str::FromStr for SourceTrust {
    type Err = crate::memory::error::MemoryError;

    /// Parse the canonical trust text.
    fn from_str(s: &str) -> MemoryResult<Self> {
        Ok(match s {
            "system" => SourceTrust::System,
            "trusted" => SourceTrust::Trusted,
            "limited" => SourceTrust::Limited,
            "untrusted" => SourceTrust::Untrusted,
            other => return Err(encoding_err(format!("unknown source trust {other:?}"))),
        })
    }
}

impl std::fmt::Display for SourceTrust {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Maximum length of a [`SourceContext`] `source_id`, in bytes. Bounded so a
/// hostile/buggy caller cannot smuggle unbounded text through provenance.
pub const SOURCE_ID_MAX_LEN: usize = 512;

// ─────────────────────────────────────────────────────────────────────────
// SourceContext — invocation + provenance correlation
// ─────────────────────────────────────────────────────────────────────────

/// The invocation/provenance context a command carries so the authority can
/// correlate its `start`/`completion` events and attribute provenance
/// (design §4.1 `events_v2.invocation_id/source_kind/source_id`; §7.4 tool
/// observations). Every field is a validated value object or a bounded checked
/// string — never a raw unchecked identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceContext {
    /// The invocation this command belongs to (start/completion correlation).
    invocation_id: InvocationId,
    /// The kind of source that issued the command.
    source_kind: SourceKind,
    /// The source's stable identity (`events_v2.source_id`, NOT NULL).
    source_id: String,
    /// The coarse trust tier the source carries at admission.
    trust: SourceTrust,
}

impl SourceContext {
    /// Validate and construct a source context. `source_id` must be non-empty,
    /// within [`SOURCE_ID_MAX_LEN`] bytes, and free of control characters.
    pub fn new(
        invocation_id: InvocationId,
        source_kind: SourceKind,
        source_id: impl Into<String>,
        trust: SourceTrust,
    ) -> MemoryResult<Self> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() {
            return Err(encoding_err("source_id must not be empty"));
        }
        if source_id.len() > SOURCE_ID_MAX_LEN {
            return Err(encoding_err(format!(
                "source_id too long: {} bytes (max {SOURCE_ID_MAX_LEN})",
                source_id.len()
            )));
        }
        if let Some(bad) = source_id.chars().find(|c| c.is_control()) {
            return Err(encoding_err(format!(
                "source_id contains control character {bad:?}"
            )));
        }
        Ok(Self {
            invocation_id,
            source_kind,
            source_id,
            trust,
        })
    }

    /// The invocation id (start/completion event correlation).
    pub fn invocation_id(&self) -> &InvocationId {
        &self.invocation_id
    }

    /// The source kind.
    pub fn source_kind(&self) -> SourceKind {
        self.source_kind
    }

    /// The source's stable identity.
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// The coarse trust tier.
    pub fn trust(&self) -> SourceTrust {
        self.trust
    }
}

// ─────────────────────────────────────────────────────────────────────────
// PreviewToken — confirmation handle for destructive/corrective commands
// ─────────────────────────────────────────────────────────────────────────

/// Maximum length of a [`PreviewToken`], in bytes.
pub const PREVIEW_TOKEN_MAX_LEN: usize = 1024;

/// An opaque token issued by a preview and presented back on the confirming
/// command (design §5.1 `Previewed --> Validate: confirm with base revision`;
/// §8.1 `command.preview/commit`, `GraphActionV2 {previewToken}`). It binds a
/// confirmed impact to the base revision it was computed at; the transaction
/// (F1.3.2+) rejects a stale token via `RevisionConflict`. Here it is a bounded,
/// non-empty, control-char-free opaque string — the authority never lets a
/// caller invent internal structure.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct PreviewToken(String);

impl PreviewToken {
    /// Validate and wrap a preview token.
    pub fn new(s: impl Into<String>) -> MemoryResult<Self> {
        let s = s.into();
        if s.is_empty() {
            return Err(encoding_err("preview token must not be empty"));
        }
        if s.len() > PREVIEW_TOKEN_MAX_LEN {
            return Err(encoding_err(format!(
                "preview token too long: {} bytes (max {PREVIEW_TOKEN_MAX_LEN})",
                s.len()
            )));
        }
        if let Some(bad) = s.chars().find(|c| c.is_control()) {
            return Err(encoding_err(format!(
                "preview token contains control character {bad:?}"
            )));
        }
        Ok(Self(s))
    }

    /// The token string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PreviewToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ─────────────────────────────────────────────────────────────────────────
// CommandKind preview classification
// ─────────────────────────────────────────────────────────────────────────

/// Extension: which command kinds are destructive/corrective and therefore must
/// be previewed then confirmed (design §5.1 `Validate --> Previewed`). Appends
/// ([`CommandKind::Observe`]) never preview; correction/forget/restore/hard
/// delete/declassify always do.
pub trait CommandKindExt {
    /// Whether a durable command of this kind must carry a [`PreviewToken`]
    /// confirming a prior preview.
    fn requires_preview(self) -> bool;
}

impl CommandKindExt for CommandKind {
    fn requires_preview(self) -> bool {
        match self {
            CommandKind::Observe => false,
            CommandKind::Correct
            | CommandKind::Forget
            | CommandKind::Restore
            | CommandKind::HardDelete
            | CommandKind::Declassify => true,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Deadline — bounded execution budget
// ─────────────────────────────────────────────────────────────────────────

/// A bounded execution deadline for a durable command (design §8.1 request
/// deadlines; A6 boundedness — "deadlines are capped"). Stored as a millisecond
/// budget so the value is deterministic and independent of wall-clock; the
/// absolute instant is derived from a start time via [`Deadline::deadline_from`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Deadline {
    budget_ms: u64,
}

impl Deadline {
    /// Hard ceiling for any single durable command budget (A6: limit errors
    /// never switch to unbounded behavior).
    pub const MAX_MS: u64 = 120_000;

    /// Default durable-write budget (design §8.1 "writes 2s").
    pub const DEFAULT_WRITE_MS: u64 = 2_000;

    /// Construct a deadline from a millisecond budget. Rejects a zero budget and
    /// any budget above [`Deadline::MAX_MS`].
    pub fn from_millis(budget_ms: u64) -> MemoryResult<Self> {
        if budget_ms == 0 {
            return Err(encoding_err("command deadline budget must be > 0"));
        }
        if budget_ms > Self::MAX_MS {
            return Err(encoding_err(format!(
                "command deadline budget {budget_ms}ms exceeds max {}ms",
                Self::MAX_MS
            )));
        }
        Ok(Self { budget_ms })
    }

    /// The default durable-write deadline (2s).
    pub fn default_write() -> Self {
        Self {
            budget_ms: Self::DEFAULT_WRITE_MS,
        }
    }

    /// The budget in milliseconds.
    pub fn budget_ms(&self) -> u64 {
        self.budget_ms
    }

    /// The budget as a [`Duration`].
    pub fn budget(&self) -> Duration {
        Duration::from_millis(self.budget_ms)
    }

    /// The absolute deadline instant given a start time.
    pub fn deadline_from(
        &self,
        start: crate::memory::model::UtcTimestamp,
    ) -> crate::memory::model::UtcTimestamp {
        let end = start.as_datetime() + chrono::Duration::milliseconds(self.budget_ms as i64);
        crate::memory::model::UtcTimestamp::from_datetime(end)
    }
}

// ─────────────────────────────────────────────────────────────────────────
// CommandHash — canonical, deterministic digest of semantic content
// ─────────────────────────────────────────────────────────────────────────

/// A deterministic digest over a command's *semantic* content, stored as
/// `idempotency_results.command_hash` and used to distinguish a legitimate
/// replay (same key, same hash) from a conflict (same key, different hash)
/// (MGR-005 AC3, design §4.1). Hex-encoded BLAKE3 (design §14 hashing).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct CommandHash(String);

impl CommandHash {
    /// Compute the canonical command hash over the semantic content:
    /// hash schema tag, caller partition, mode, command kind, source
    /// kind/id/trust, and the canonicalized payload. Deliberately excludes
    /// per-attempt / execution-budget fields (invocation id, idempotency key,
    /// base revision, deadline, preview token) so a retry of the *same intent*
    /// hashes identically.
    ///
    /// Determinism: the payload is canonicalized with recursively sorted object
    /// keys ([`canonical_json`]) before hashing, so field ordering in the input
    /// JSON never changes the hash.
    fn compute(
        caller: &CallerContext,
        mode: &MemoryMode,
        kind: CommandKind,
        source: &SourceContext,
        payload: &Value,
    ) -> Self {
        let mut preimage = Map::new();
        preimage.insert("v".into(), Value::from(COMMAND_HASH_SCHEMA));
        preimage.insert(
            "caller_partition".into(),
            Value::from(caller.partition_key()),
        );
        preimage.insert("mode".into(), Value::from(mode.as_str()));
        preimage.insert("kind".into(), Value::from(kind_str(kind)));
        preimage.insert(
            "source_kind".into(),
            Value::from(source.source_kind().as_str()),
        );
        preimage.insert("source_id".into(), Value::from(source.source_id()));
        preimage.insert("source_trust".into(), Value::from(source.trust().as_str()));
        preimage.insert("payload".into(), canonical_json(payload));

        let canonical = Value::Object(preimage);
        // `canonical` already has sorted keys at the top level (BTreeMap) and a
        // canonicalized payload subtree; serialize deterministically and hash.
        let bytes =
            serde_json::to_vec(&canonical).expect("canonical command preimage always serializes");
        CommandHash(blake3_hex(&bytes))
    }

    /// The hex digest string (as stored in `idempotency_results.command_hash`).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CommandHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The canonical text of a [`CommandKind`] (stable, snake_case) for hashing and
/// audit. Delegates to [`CommandKind::as_str`] so the canonical form lives in
/// one place.
fn kind_str(kind: CommandKind) -> &'static str {
    kind.as_str()
}

/// Recursively canonicalize a JSON value so equivalent content hashes
/// identically regardless of object key ordering: object keys are sorted and
/// their values canonicalized; arrays preserve order (order is semantic);
/// scalars pass through unchanged.
fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            // BTreeMap-backed `Map` iterates in sorted key order; rebuild so any
            // insertion-order (preserve_order) backing is normalized too.
            let mut sorted: Vec<(&String, &Value)> = map.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(b.0));
            let mut out = Map::new();
            for (k, v) in sorted {
                out.insert(k.clone(), canonical_json(v));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// CommandEnvelope — the admitted unit
// ─────────────────────────────────────────────────────────────────────────

/// The validated, self-describing command the authority admits (design §5.1).
///
/// Constructed at the composition boundary from an authenticated
/// [`CallerContext`] plus the caller's semantic request; the
/// [`CommandHash`] is computed on construction. It is `Serialize` (for audit /
/// logging) but intentionally **not** `Deserialize`: a caller cannot rehydrate
/// a trusted envelope — in particular its [`CallerContext`] — from untrusted
/// input. The transaction path (F1.3.2+) consumes an already-constructed,
/// already-validated envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandEnvelope {
    caller: CallerContext,
    kind: CommandKind,
    idempotency_key: IdempotencyKey,
    command_hash: CommandHash,
    base_revision: GraphRevision,
    source: SourceContext,
    mode: MemoryMode,
    deadline: Deadline,
    payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview_token: Option<PreviewToken>,
}

impl CommandEnvelope {
    /// Validate and construct a command envelope, computing its canonical
    /// [`CommandHash`] from the semantic content.
    ///
    /// Required-field rules enforced here:
    /// * A destructive/corrective kind ([`CommandKindExt::requires_preview`])
    ///   MUST carry a [`PreviewToken`] (it confirms a prior preview against a
    ///   base revision — design §5.1).
    /// * An append ([`CommandKind::Observe`]) MUST NOT carry a preview token
    ///   (appends are never previewed; a token would be meaningless).
    ///
    /// All other fields are validated value objects, so they cannot be absent
    /// or malformed by construction.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        caller: CallerContext,
        kind: CommandKind,
        idempotency_key: IdempotencyKey,
        base_revision: GraphRevision,
        source: SourceContext,
        mode: MemoryMode,
        deadline: Deadline,
        payload: Value,
        preview_token: Option<PreviewToken>,
    ) -> MemoryResult<Self> {
        match (kind.requires_preview(), preview_token.is_some()) {
            (true, false) => {
                return Err(encoding_err(format!(
                    "command kind {:?} requires a preview token to confirm against its base revision",
                    kind
                )));
            }
            (false, true) => {
                return Err(encoding_err(format!(
                    "command kind {:?} is not previewable and must not carry a preview token",
                    kind
                )));
            }
            _ => {}
        }

        let command_hash = CommandHash::compute(&caller, &mode, kind, &source, &payload);

        Ok(Self {
            caller,
            kind,
            idempotency_key,
            command_hash,
            base_revision,
            source,
            mode,
            deadline,
            payload,
            preview_token,
        })
    }

    /// The authenticated caller.
    pub fn caller(&self) -> &CallerContext {
        &self.caller
    }

    /// The governed operation kind.
    pub fn kind(&self) -> CommandKind {
        self.kind
    }

    /// The caller-chosen idempotency key (paired with the caller partition in
    /// `idempotency_results`).
    pub fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    /// The canonical command hash over the semantic content.
    pub fn command_hash(&self) -> &CommandHash {
        &self.command_hash
    }

    /// The revision the caller issued against (optimistic concurrency).
    pub fn base_revision(&self) -> GraphRevision {
        self.base_revision
    }

    /// The invocation / provenance context.
    pub fn source(&self) -> &SourceContext {
        &self.source
    }

    /// The admission memory mode.
    pub fn mode(&self) -> &MemoryMode {
        &self.mode
    }

    /// The bounded execution deadline.
    pub fn deadline(&self) -> Deadline {
        self.deadline
    }

    /// The command-kind-specific payload (canonicalized only for hashing; the
    /// stored value preserves the caller's structure).
    pub fn payload(&self) -> &Value {
        &self.payload
    }

    /// The preview token confirming a prior preview, when this kind requires it.
    pub fn preview_token(&self) -> Option<&PreviewToken> {
        self.preview_token.as_ref()
    }

    /// Whether this command must be previewed then confirmed.
    pub fn requires_preview(&self) -> bool {
        self.kind.requires_preview()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::model::PolicyPartition;
    use proptest::prelude::*;
    use std::str::FromStr;

    fn caller() -> CallerContext {
        let partition = PolicyPartition::new("user", "chat", 0).unwrap();
        CallerContext::local_desktop("local-desktop", partition).unwrap()
    }

    fn source() -> SourceContext {
        SourceContext::new(
            InvocationId::new_v7(),
            SourceKind::Native,
            "core:cognition",
            SourceTrust::System,
        )
        .unwrap()
    }

    fn observe_envelope(payload: Value) -> CommandEnvelope {
        CommandEnvelope::new(
            caller(),
            CommandKind::Observe,
            IdempotencyKey::new("cmd-1").unwrap(),
            GraphRevision::base(),
            source(),
            MemoryMode::Permanent,
            Deadline::default_write(),
            payload,
            None,
        )
        .unwrap()
    }

    // ── Value-object validation ─────────────────────────────────────────
    #[test]
    fn source_kind_and_trust_round_trip() {
        for k in [
            SourceKind::Native,
            SourceKind::Mcp,
            SourceKind::OpenClaw,
            SourceKind::Sidecar,
            SourceKind::Import,
            SourceKind::Library,
            SourceKind::Conversation,
        ] {
            assert_eq!(SourceKind::from_str(k.as_str()).unwrap(), k);
        }
        assert!(SourceKind::from_str("bogus").is_err());

        for t in [
            SourceTrust::System,
            SourceTrust::Trusted,
            SourceTrust::Limited,
            SourceTrust::Untrusted,
        ] {
            assert_eq!(SourceTrust::from_str(t.as_str()).unwrap(), t);
        }
        assert!(SourceTrust::from_str("bogus").is_err());
        // Trust ordering: System is most trusted, Untrusted least.
        assert!(SourceTrust::System < SourceTrust::Untrusted);
    }

    #[test]
    fn source_context_rejects_bad_source_id() {
        let inv = InvocationId::new_v7();
        assert!(
            SourceContext::new(inv.clone(), SourceKind::Native, "", SourceTrust::System).is_err()
        );
        assert!(
            SourceContext::new(inv.clone(), SourceKind::Native, "  ", SourceTrust::System).is_err()
        );
        assert!(SourceContext::new(
            inv.clone(),
            SourceKind::Native,
            "bad\nid",
            SourceTrust::System
        )
        .is_err());
        let oversized = "x".repeat(SOURCE_ID_MAX_LEN + 1);
        assert!(
            SourceContext::new(inv, SourceKind::Native, oversized, SourceTrust::System).is_err()
        );
    }

    #[test]
    fn preview_token_validation() {
        assert!(PreviewToken::new("tok-abc").is_ok());
        assert!(PreviewToken::new("").is_err());
        assert!(PreviewToken::new("bad\ttoken").is_err());
        assert!(PreviewToken::new("x".repeat(PREVIEW_TOKEN_MAX_LEN + 1)).is_err());
    }

    #[test]
    fn deadline_is_bounded() {
        assert_eq!(Deadline::default_write().budget_ms(), 2_000);
        assert!(Deadline::from_millis(0).is_err());
        assert!(Deadline::from_millis(Deadline::MAX_MS).is_ok());
        assert!(Deadline::from_millis(Deadline::MAX_MS + 1).is_err());
        // Absolute deadline derives from a start time.
        let start =
            crate::memory::model::UtcTimestamp::from_rfc3339_utc("2026-01-01T00:00:00Z").unwrap();
        let d = Deadline::from_millis(2_000).unwrap();
        let end = d.deadline_from(start);
        assert!(end > start);
    }

    // ── Required-field validation ───────────────────────────────────────
    #[test]
    fn observe_rejects_a_preview_token() {
        let err = CommandEnvelope::new(
            caller(),
            CommandKind::Observe,
            IdempotencyKey::new("k").unwrap(),
            GraphRevision::base(),
            source(),
            MemoryMode::Permanent,
            Deadline::default_write(),
            serde_json::json!({"content": "hi"}),
            Some(PreviewToken::new("tok").unwrap()),
        );
        assert!(err.is_err(), "append must not carry a preview token");
    }

    #[test]
    fn destructive_kinds_require_a_preview_token() {
        for kind in [
            CommandKind::Correct,
            CommandKind::Forget,
            CommandKind::Restore,
            CommandKind::HardDelete,
        ] {
            assert!(kind.requires_preview(), "{kind:?} must require preview");
            // Missing token → rejected.
            let missing = CommandEnvelope::new(
                caller(),
                kind,
                IdempotencyKey::new("k").unwrap(),
                GraphRevision::new(7),
                source(),
                MemoryMode::Permanent,
                Deadline::default_write(),
                serde_json::json!({"target": "rec-1"}),
                None,
            );
            assert!(
                missing.is_err(),
                "{kind:?} without a token must be rejected"
            );

            // With a token → accepted.
            let ok = CommandEnvelope::new(
                caller(),
                kind,
                IdempotencyKey::new("k").unwrap(),
                GraphRevision::new(7),
                source(),
                MemoryMode::Permanent,
                Deadline::default_write(),
                serde_json::json!({"target": "rec-1"}),
                Some(PreviewToken::new("tok").unwrap()),
            );
            assert!(ok.is_ok(), "{kind:?} with a token must be accepted");
        }
    }

    #[test]
    fn observe_does_not_require_preview() {
        assert!(!CommandKind::Observe.requires_preview());
        let env = observe_envelope(serde_json::json!({"content": "hi"}));
        assert!(!env.requires_preview());
        assert!(env.preview_token().is_none());
    }

    // ── Canonical command hash: determinism & stability ─────────────────
    #[test]
    fn command_hash_is_deterministic_for_equal_content() {
        let a = observe_envelope(serde_json::json!({"content": "hello", "tags": ["x", "y"]}));
        let b = observe_envelope(serde_json::json!({"content": "hello", "tags": ["x", "y"]}));
        assert_eq!(
            a.command_hash(),
            b.command_hash(),
            "equal semantic content must hash identically"
        );
        // Hash is a non-empty hex digest.
        assert!(!a.command_hash().as_str().is_empty());
        assert!(a
            .command_hash()
            .as_str()
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn command_hash_is_stable_under_object_key_reordering() {
        // Same object, keys authored in different order → identical hash.
        let a = observe_envelope(serde_json::json!({"a": 1, "b": 2, "nested": {"p": 1, "q": 2}}));
        let b = observe_envelope(serde_json::json!({"nested": {"q": 2, "p": 1}, "b": 2, "a": 1}));
        assert_eq!(
            a.command_hash(),
            b.command_hash(),
            "object key order must not change the hash"
        );
    }

    #[test]
    fn command_hash_excludes_per_attempt_and_budget_fields() {
        let payload = serde_json::json!({"content": "same"});
        // Baseline.
        let base = observe_envelope(payload.clone());

        // Different invocation id (per-attempt) → same hash.
        let diff_inv = CommandEnvelope::new(
            caller(),
            CommandKind::Observe,
            IdempotencyKey::new("cmd-1").unwrap(),
            GraphRevision::base(),
            SourceContext::new(
                InvocationId::new_v7(), // different invocation
                SourceKind::Native,
                "core:cognition",
                SourceTrust::System,
            )
            .unwrap(),
            MemoryMode::Permanent,
            Deadline::default_write(),
            payload.clone(),
            None,
        )
        .unwrap();
        assert_eq!(base.command_hash(), diff_inv.command_hash());

        // Different idempotency key + base revision + deadline → same hash.
        let diff_meta = CommandEnvelope::new(
            caller(),
            CommandKind::Observe,
            IdempotencyKey::new("cmd-2").unwrap(),
            GraphRevision::new(99),
            source(),
            MemoryMode::Permanent,
            Deadline::from_millis(5_000).unwrap(),
            payload,
            None,
        )
        .unwrap();
        assert_eq!(
            base.command_hash(),
            diff_meta.command_hash(),
            "idempotency key / base revision / deadline are not semantic content"
        );
    }

    #[test]
    fn command_hash_changes_with_semantic_content() {
        let base = observe_envelope(serde_json::json!({"content": "a"}));

        // Different payload content.
        let diff_payload = observe_envelope(serde_json::json!({"content": "b"}));
        assert_ne!(base.command_hash(), diff_payload.command_hash());

        // Different mode.
        let diff_mode = CommandEnvelope::new(
            caller(),
            CommandKind::Observe,
            IdempotencyKey::new("cmd-1").unwrap(),
            GraphRevision::base(),
            source(),
            MemoryMode::Temporary, // different mode
            Deadline::default_write(),
            serde_json::json!({"content": "a"}),
            None,
        )
        .unwrap();
        assert_ne!(base.command_hash(), diff_mode.command_hash());

        // Different source kind/id/trust.
        let diff_source = CommandEnvelope::new(
            caller(),
            CommandKind::Observe,
            IdempotencyKey::new("cmd-1").unwrap(),
            GraphRevision::base(),
            SourceContext::new(
                InvocationId::new_v7(),
                SourceKind::Mcp, // different kind
                "mcp:server-1",
                SourceTrust::Trusted,
            )
            .unwrap(),
            MemoryMode::Permanent,
            Deadline::default_write(),
            serde_json::json!({"content": "a"}),
            None,
        )
        .unwrap();
        assert_ne!(base.command_hash(), diff_source.command_hash());
    }

    #[test]
    fn envelope_serializes_for_audit() {
        let env = observe_envelope(serde_json::json!({"content": "hi"}));
        let json = serde_json::to_value(&env).unwrap();
        assert_eq!(json["kind"], "observe");
        assert_eq!(json["mode"], "permanent");
        assert_eq!(json["source"]["source_kind"], "native");
        assert_eq!(json["source"]["trust"], "system");
        assert_eq!(json["command_hash"], env.command_hash().as_str());
        // Optional preview token is omitted when absent.
        assert!(json.get("preview_token").is_none());
    }

    // ── Property: canonical hash is order-independent for arbitrary maps ──
    proptest! {
        /// For any set of string keys and integer values, hashing an object
        /// built in the given order equals hashing the reverse-order object:
        /// the canonical command hash is stable under object key reordering.
        #[test]
        fn prop_command_hash_stable_under_key_reordering(
            pairs in proptest::collection::vec(
                ("[a-z]{1,8}", 0i64..1000),
                1..12,
            )
        ) {
            // Deduplicate keys (a JSON object cannot have duplicate keys).
            let mut seen = std::collections::BTreeMap::new();
            for (k, v) in &pairs {
                seen.insert(k.clone(), *v);
            }

            let forward = {
                let mut m = Map::new();
                for (k, v) in seen.iter() {
                    m.insert(k.clone(), Value::from(*v));
                }
                Value::Object(m)
            };
            let backward = {
                let mut m = Map::new();
                for (k, v) in seen.iter().rev() {
                    m.insert(k.clone(), Value::from(*v));
                }
                Value::Object(m)
            };

            let ha = observe_envelope(forward);
            let hb = observe_envelope(backward);
            prop_assert_eq!(ha.command_hash(), hb.command_hash());
        }
    }
}
