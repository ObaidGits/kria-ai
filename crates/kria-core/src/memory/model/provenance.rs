//! The structured [`Provenance`] value object and its policy-safe [`Locator`]
//! (design §4.2/§4.3, glossary "Provenance"; task F2.1.2).
//!
//! Glossary "Provenance" = the *source, actor, method, time, locator,
//! model/algorithm version, and derivation links that explain a record*. This
//! module makes that a single validated value object so every applicable
//! cognitive-record type expresses its provenance the same way, instead of each
//! type re-deriving the meaning of its own flat `source_id` / `extractor` /
//! `method` / `locator_json` / `created_event_id` columns.
//!
//! What this task adds (types + validation + mapping only — the write path is
//! F1.5, the SQL↔API round-trip properties are 2.1.5, unknown-field
//! preservation is 2.1.4, and the legacy-struct reconciliation/removal ledger
//! is [`crate::memory::model::legacy_mapping`], task 2.1.6):
//!
//!   * [`Provenance`] — source + actor + method + model/algorithm identity +
//!     time + immediate parents/derivation links + policy-safe locator +
//!     creation Event.
//!   * [`Locator`] — a **typed, validated** locator that serializes to the
//!     `locator_json` column (`json_valid`-guarded in migration 0017). It is
//!     policy-safe by construction: it has *no* free-text content field, so a
//!     mention/evidence span reference cannot smuggle protected content, and
//!     every reference string is bounded and rejects control characters
//!     (raw extracted content typically carries newlines/control bytes).
//!   * [`HasProvenance`] — the shared seam mapping each existing type's flat
//!     columns onto [`Provenance`], so "express provenance consistently" holds
//!     without a parallel mechanism.
//!
//! Immediate parents / derivation links: the storage layer materializes
//! `derived_from@1` / `supports@1` / `mentions_entity@1` as Memory Links
//! (design §19.3, task F2.2) rather than as columns on the derived row. The
//! in-memory [`Provenance`] unifies those into [`Provenance::parents`]; where a
//! type *does* carry its immediate parent inline (evidence's `source_record` /
//! `source_event`), the mapping surfaces them here.

use serde::{Deserialize, Serialize};

use super::entity::Span;
use super::{encoding_err, EntityId, EpisodeId, EventId, RecordId, SourceId, UtcTimestamp};
use crate::memory::authority::command::SourceKind;
use crate::memory::error::MemoryResult;

/// Maximum length (bytes) of a provenance reference field (actor id, method
/// name/version, model id/version, endpoint kind/id). Bounded so a reference
/// can never become a content dump.
pub const PROV_FIELD_MAX_LEN: usize = 1024;

/// Maximum length (bytes) of a [`Locator`] reference string (document id,
/// conversation id, url). Slightly larger than a plain field to allow long but
/// still-structural identifiers/URLs.
pub const LOCATOR_REF_MAX_LEN: usize = 2048;

/// Validate a structural reference field: non-empty, bounded, and free of
/// control characters. The control-character rejection is the policy-safety
/// guard — raw extracted/protected content routinely contains newlines/control
/// bytes, so a structural locator/method/actor reference that carries them is
/// treated as a content leak and rejected.
pub(crate) fn validated_field(
    what: &str,
    s: impl Into<String>,
    max: usize,
) -> MemoryResult<String> {
    let s = s.into();
    if s.trim().is_empty() {
        return Err(encoding_err(format!("{what} must not be empty")));
    }
    if s.len() > max {
        return Err(encoding_err(format!(
            "{what} too long: {} bytes (max {max})",
            s.len()
        )));
    }
    if let Some(bad) = s.chars().find(|c| c.is_control()) {
        return Err(encoding_err(format!(
            "{what} contains control character {bad:?} (possible content leak)"
        )));
    }
    Ok(s)
}

// ── Actor ──────────────────────────────────────────────────────────────────

/// The actor that produced or asserted a record (`actor_id` columns). A bounded
/// non-empty reference — never raw content.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Actor(String);

impl Actor {
    /// Validate and wrap an actor id.
    pub fn new(id: impl Into<String>) -> MemoryResult<Self> {
        Ok(Self(validated_field(
            "provenance actor id",
            id,
            PROV_FIELD_MAX_LEN,
        )?))
    }

    /// The actor id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for Actor {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Actor {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(de)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

// ── Method ───────────────────────────────────────────────────────────────

/// The method that produced or asserted a record, plus its optional version
/// (mentions' `extractor`/`extractor_version`; evidence's
/// `method`/`method_version`; a classifier). Both parts are validated bounded
/// references.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Method {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
}

impl Method {
    /// Construct a method with an optional version.
    pub fn new(name: impl Into<String>, version: Option<String>) -> MemoryResult<Self> {
        let name = validated_field("provenance method name", name, PROV_FIELD_MAX_LEN)?;
        let version = match version {
            Some(v) => Some(validated_field(
                "provenance method version",
                v,
                PROV_FIELD_MAX_LEN,
            )?),
            None => None,
        };
        Ok(Self { name, version })
    }

    /// The method name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The method version, if recorded.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MethodRaw {
    name: String,
    #[serde(default)]
    version: Option<String>,
}

impl<'de> Deserialize<'de> for Method {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = MethodRaw::deserialize(de)?;
        Self::new(raw.name, raw.version).map_err(serde::de::Error::custom)
    }
}

// ── ModelIdentity ──────────────────────────────────────────────────────────

/// A model or algorithm identity + version, for extracted/derived/consolidated
/// records and retrieval traces (consolidation `algorithm`/`version`; retrieval
/// `embed_model_version`; the embedding-model manifest itself is F3.1 — this is
/// only the provenance *reference* to it). Deterministic: the same identity +
/// version denotes the same producer, which is what the retrieval trace and
/// rebuild rely on.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct ModelIdentity {
    id: String,
    version: String,
}

impl ModelIdentity {
    /// Construct a model/algorithm identity. Both `id` and `version` are
    /// required (a versionless model reference is not deterministic).
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> MemoryResult<Self> {
        Ok(Self {
            id: validated_field("provenance model id", id, PROV_FIELD_MAX_LEN)?,
            version: validated_field("provenance model version", version, PROV_FIELD_MAX_LEN)?,
        })
    }

    /// The model/algorithm identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The model/algorithm version.
    pub fn version(&self) -> &str {
        &self.version
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelIdentityRaw {
    id: String,
    version: String,
}

impl<'de> Deserialize<'de> for ModelIdentity {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = ModelIdentityRaw::deserialize(de)?;
        Self::new(raw.id, raw.version).map_err(serde::de::Error::custom)
    }
}

// ── SourceRef ────────────────────────────────────────────────────────────

/// A reference to a contributing source: the validated [`SourceId`] plus the
/// optional [`SourceKind`]. Cognitive-record rows store only `source_id`
/// (NOT NULL); the `kind` lives on the `sources` row and is carried here when a
/// projection has resolved it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SourceRef {
    id: SourceId,
    kind: Option<SourceKind>,
}

impl SourceRef {
    /// A source reference with only its id (kind unresolved).
    pub fn new(id: SourceId) -> Self {
        Self { id, kind: None }
    }

    /// A source reference with a resolved kind.
    pub fn with_kind(id: SourceId, kind: SourceKind) -> Self {
        Self {
            id,
            kind: Some(kind),
        }
    }

    /// The source id.
    pub fn id(&self) -> &SourceId {
        &self.id
    }

    /// The resolved source kind, if known.
    pub fn kind(&self) -> Option<SourceKind> {
        self.kind
    }
}

impl Serialize for SourceRef {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = ser.serialize_struct("SourceRef", if self.kind.is_some() { 2 } else { 1 })?;
        st.serialize_field("id", &self.id)?;
        match &self.kind {
            Some(k) => st.serialize_field("kind", k.as_str())?,
            None => st.skip_field("kind")?,
        }
        st.end()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceRefRaw {
    id: SourceId,
    #[serde(default)]
    kind: Option<String>,
}

impl<'de> Deserialize<'de> for SourceRef {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = SourceRefRaw::deserialize(de)?;
        let kind = match raw.kind {
            Some(k) => Some(k.parse::<SourceKind>().map_err(serde::de::Error::custom)?),
            None => None,
        };
        Ok(Self { id: raw.id, kind })
    }
}

// ── ParentRef (immediate parents / derivation links) ────────────────────────

/// An immediate parent / derivation link — a record's `derived_from` source
/// (design §19.3 `derived_from@1`: derived record → immediate
/// Event/Memory/Episode/Summary/Skill). Typed variants cover the common
/// endpoints; [`ParentRef::Endpoint`] is the polymorphic escape hatch for a
/// mixed kind/id endpoint (e.g. evidence's `source_record_kind`/`id`) whose kind
/// is not one of the closed variants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentRef {
    /// An authority event parent.
    Event(EventId),
    /// A record parent (memory/summary/skill/rule).
    Record(RecordId),
    /// An episode parent.
    Episode(EpisodeId),
    /// An entity parent.
    Entity(EntityId),
    /// A polymorphic kind+id endpoint (validated bounded references).
    Endpoint {
        /// The endpoint kind.
        kind: String,
        /// The endpoint id.
        id: String,
    },
}

impl ParentRef {
    /// Construct a validated polymorphic endpoint parent.
    pub fn endpoint(kind: impl Into<String>, id: impl Into<String>) -> MemoryResult<Self> {
        Ok(ParentRef::Endpoint {
            kind: validated_field("parent endpoint kind", kind, PROV_FIELD_MAX_LEN)?,
            id: validated_field("parent endpoint id", id, PROV_FIELD_MAX_LEN)?,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum ParentRefRaw {
    Event(EventId),
    Record(RecordId),
    Episode(EpisodeId),
    Entity(EntityId),
    Endpoint { kind: String, id: String },
}

impl<'de> Deserialize<'de> for ParentRef {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        Ok(match ParentRefRaw::deserialize(de)? {
            ParentRefRaw::Event(e) => ParentRef::Event(e),
            ParentRefRaw::Record(r) => ParentRef::Record(r),
            ParentRefRaw::Episode(e) => ParentRef::Episode(e),
            ParentRefRaw::Entity(e) => ParentRef::Entity(e),
            ParentRefRaw::Endpoint { kind, id } => {
                ParentRef::endpoint(kind, id).map_err(serde::de::Error::custom)?
            }
        })
    }
}

// ── Locator (policy-safe structured locator) ─────────────────────────────

/// A **policy-safe, structured** locator that serializes to the `locator_json`
/// column (design §4.2 `mentions.locator_json` / `evidence.locator_json`,
/// `json_valid`-guarded in migration 0017).
///
/// Policy-safe by construction: every variant references *where* a claim comes
/// from (a document id + span, a conversation turn, an authority event, a
/// record, or a URL), and there is **no free-text content field** in which a
/// span's protected text could be smuggled. Every reference string is bounded
/// and rejects control characters, and deserialization uses `deny_unknown_fields`
/// so an arbitrary JSON blob carrying an extra `content`/`text` field is
/// rejected rather than silently accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Locator {
    /// A reference into a document/source by id and optional character span.
    Document {
        /// The document/source identifier.
        document_id: String,
        /// The character span within the document, if known.
        #[serde(skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// A conversation turn reference.
    ConversationTurn {
        /// The conversation identifier.
        conversation_id: String,
        /// The zero-based turn index.
        turn_index: u32,
        /// The character span within the turn, if known.
        #[serde(skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// An authority event reference.
    Event {
        /// The referenced event.
        event_id: EventId,
        /// The character span within the event payload, if known.
        #[serde(skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// A record reference.
    Record {
        /// The referenced record.
        record_id: RecordId,
        /// The character span within the record content, if known.
        #[serde(skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// A URL reference (http/https) with an optional span.
    Url {
        /// The http(s) URL.
        url: String,
        /// The character span within the fetched content, if known.
        #[serde(skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
}

impl Locator {
    /// A document locator: a bounded document id and optional span.
    pub fn document(document_id: impl Into<String>, span: Option<Span>) -> MemoryResult<Self> {
        Ok(Locator::Document {
            document_id: validated_field("locator document id", document_id, LOCATOR_REF_MAX_LEN)?,
            span,
        })
    }

    /// A conversation-turn locator.
    pub fn conversation_turn(
        conversation_id: impl Into<String>,
        turn_index: u32,
        span: Option<Span>,
    ) -> MemoryResult<Self> {
        Ok(Locator::ConversationTurn {
            conversation_id: validated_field(
                "locator conversation id",
                conversation_id,
                LOCATOR_REF_MAX_LEN,
            )?,
            turn_index,
            span,
        })
    }

    /// An authority-event locator (the [`EventId`] is already validated).
    pub fn event(event_id: EventId, span: Option<Span>) -> Self {
        Locator::Event { event_id, span }
    }

    /// A record locator (the [`RecordId`] is already validated).
    pub fn record(record_id: RecordId, span: Option<Span>) -> Self {
        Locator::Record { record_id, span }
    }

    /// A URL locator; the URL must be an http(s) URL and free of control
    /// characters.
    pub fn url(url: impl Into<String>, span: Option<Span>) -> MemoryResult<Self> {
        let url = validated_field("locator url", url, LOCATOR_REF_MAX_LEN)?;
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(encoding_err("locator url must be an http(s) URL"));
        }
        Ok(Locator::Url { url, span })
    }

    /// Serialize to the validated `locator_json` string. `serde_json` always
    /// emits `json_valid` text, satisfying the column guard.
    pub fn to_json(&self) -> MemoryResult<String> {
        serde_json::to_string(self)
            .map_err(|e| encoding_err(format!("locator serialization failed: {e}")))
    }

    /// Parse and re-validate a `locator_json` string into a typed locator.
    /// Rejects unknown fields (policy-safety) and any leaked control content.
    pub fn from_json(s: &str) -> MemoryResult<Self> {
        serde_json::from_str(s).map_err(|e| encoding_err(format!("invalid locator json: {e}")))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum LocatorRaw {
    Document {
        document_id: String,
        #[serde(default)]
        span: Option<Span>,
    },
    ConversationTurn {
        conversation_id: String,
        turn_index: u32,
        #[serde(default)]
        span: Option<Span>,
    },
    Event {
        event_id: EventId,
        #[serde(default)]
        span: Option<Span>,
    },
    Record {
        record_id: RecordId,
        #[serde(default)]
        span: Option<Span>,
    },
    Url {
        url: String,
        #[serde(default)]
        span: Option<Span>,
    },
}

impl<'de> Deserialize<'de> for Locator {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        Ok(match LocatorRaw::deserialize(de)? {
            LocatorRaw::Document { document_id, span } => {
                Locator::document(document_id, span).map_err(serde::de::Error::custom)?
            }
            LocatorRaw::ConversationTurn {
                conversation_id,
                turn_index,
                span,
            } => Locator::conversation_turn(conversation_id, turn_index, span)
                .map_err(serde::de::Error::custom)?,
            LocatorRaw::Event { event_id, span } => Locator::event(event_id, span),
            LocatorRaw::Record { record_id, span } => Locator::record(record_id, span),
            LocatorRaw::Url { url, span } => {
                Locator::url(url, span).map_err(serde::de::Error::custom)?
            }
        })
    }
}

// ── ProvenanceTime ─────────────────────────────────────────────────────────

/// The two independent times a provenance record can carry: transaction-time
/// creation (`created_at`) and valid/observation time (`observed_at`). Both are
/// optional because not every type stores both (design keeps Valid Time and
/// Transaction Time independent).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceTime {
    /// Transaction-time creation instant, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<UtcTimestamp>,
    /// Valid/observation instant, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<UtcTimestamp>,
}

impl ProvenanceTime {
    /// Whether neither time is present.
    pub fn is_empty(&self) -> bool {
        self.created_at.is_none() && self.observed_at.is_none()
    }
}

// ── Provenance ───────────────────────────────────────────────────────────

/// The structured provenance of a cognitive record or semantic claim — the
/// single value object that unifies *source, actor, method, model/algorithm
/// identity, time, immediate parents/derivation links, policy-safe locator, and
/// creation Event* (glossary "Provenance").
///
/// Constructed via [`Provenance::new`] + the chaining `with_*` builders, or
/// derived from an existing type through [`HasProvenance::provenance`]. Fields
/// are optional/empty where a given type does not carry them (e.g. a `records`
/// row has no inline parents — those are `derived_from@1` Memory Links, F2.2 —
/// so [`Provenance::parents`] is empty for a bare record).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// The contributing source (id + optional kind). Present on every
    /// cognitive-record type (their `source_id` column is NOT NULL); absent for
    /// derived/operational provenance without a direct source (consolidation
    /// runs, retrieval traces).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceRef>,
    /// The actor that produced/asserted the record, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    /// The method/version that produced or asserted the record, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<Method>,
    /// The model/algorithm identity + version (for extracted/derived/
    /// consolidated records and retrieval traces), if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelIdentity>,
    /// Creation / observation times.
    #[serde(default, skip_serializing_if = "ProvenanceTime::is_empty")]
    pub time: ProvenanceTime,
    /// Immediate parents / derivation links (design §19.3 `derived_from@1`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<ParentRef>,
    /// The policy-safe structured locator, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<Locator>,
    /// The creating authority event, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_event_id: Option<EventId>,
}

impl Provenance {
    /// An empty provenance (all fields unset).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the contributing source.
    pub fn with_source(mut self, source: SourceRef) -> Self {
        self.source = Some(source);
        self
    }

    /// Set the actor.
    pub fn with_actor(mut self, actor: Actor) -> Self {
        self.actor = Some(actor);
        self
    }

    /// Set the method.
    pub fn with_method(mut self, method: Method) -> Self {
        self.method = Some(method);
        self
    }

    /// Set the model/algorithm identity.
    pub fn with_model(mut self, model: ModelIdentity) -> Self {
        self.model = Some(model);
        self
    }

    /// Set the times.
    pub fn with_time(mut self, time: ProvenanceTime) -> Self {
        self.time = time;
        self
    }

    /// Append one immediate parent / derivation link.
    pub fn with_parent(mut self, parent: ParentRef) -> Self {
        self.parents.push(parent);
        self
    }

    /// Replace the immediate parents / derivation links.
    pub fn with_parents(mut self, parents: Vec<ParentRef>) -> Self {
        self.parents = parents;
        self
    }

    /// Set the policy-safe locator.
    pub fn with_locator(mut self, locator: Locator) -> Self {
        self.locator = Some(locator);
        self
    }

    /// Set the creating authority event.
    pub fn with_created_event(mut self, event_id: EventId) -> Self {
        self.created_event_id = Some(event_id);
        self
    }
}

/// The shared seam every applicable cognitive-record type uses to express its
/// provenance as a structured [`Provenance`], mapping its flat columns onto the
/// one model. Returns an error only when a stored `locator_json` cannot be
/// parsed as a policy-safe [`Locator`] (mentions/evidence); types without a
/// locator never fail.
pub trait HasProvenance {
    /// Assemble this type's provenance.
    fn provenance(&self) -> MemoryResult<Provenance>;
}

// ── HasProvenance mappings (existing columns → structured Provenance) ────────
//
// These map each type's flat provenance columns onto the one [`Provenance`]
// model. They read-only project existing fields (no write-path wiring); the
// only fallible step is parsing a stored `locator_json`.

use super::consolidation::ConsolidationRun;
use super::entity::{Alias, Entity, Evidence, Mention};
use super::episode::Episode;
use super::feedback::Feedback;
use super::goal::Goal;
use super::observation::{RetrievalTrace, ToolObservation};
use super::record::Record;

/// Build an optional [`Method`] from an optional name + version.
fn opt_method(name: &Option<String>, version: &Option<String>) -> MemoryResult<Option<Method>> {
    match name {
        Some(n) => Ok(Some(Method::new(n.clone(), version.clone())?)),
        None => Ok(None),
    }
}

/// Build an optional polymorphic parent endpoint from an optional kind + id.
fn opt_endpoint(kind: &Option<String>, id: &Option<String>) -> MemoryResult<Option<ParentRef>> {
    match (kind, id) {
        (Some(k), Some(i)) => Ok(Some(ParentRef::endpoint(k.clone(), i.clone())?)),
        _ => Ok(None),
    }
}

impl HasProvenance for Record {
    fn provenance(&self) -> MemoryResult<Provenance> {
        // Immediate parents are `derived_from@1` Memory Links (F2.2), not
        // columns on the record — so `parents` is empty here.
        Ok(Provenance::new()
            .with_source(SourceRef::new(self.source_id.clone()))
            .with_created_event(self.created_event_id.clone())
            .with_time(ProvenanceTime {
                created_at: Some(self.created_at),
                observed_at: None,
            }))
    }
}

impl HasProvenance for Entity {
    fn provenance(&self) -> MemoryResult<Provenance> {
        Ok(Provenance::new()
            .with_source(SourceRef::new(self.source_id.clone()))
            .with_created_event(self.created_event_id.clone())
            .with_time(ProvenanceTime {
                created_at: Some(self.created_at),
                observed_at: None,
            }))
    }
}

impl HasProvenance for Alias {
    fn provenance(&self) -> MemoryResult<Provenance> {
        let mut p = Provenance::new()
            .with_source(SourceRef::new(self.source_id.clone()))
            .with_time(ProvenanceTime {
                created_at: self.created_at,
                observed_at: None,
            });
        if let Some(ev) = &self.created_event_id {
            p = p.with_created_event(ev.clone());
        }
        Ok(p)
    }
}

impl HasProvenance for Mention {
    fn provenance(&self) -> MemoryResult<Provenance> {
        let mut p = Provenance::new()
            .with_source(SourceRef::new(self.source_id.clone()))
            .with_time(ProvenanceTime {
                created_at: None,
                observed_at: self.observed_at,
            });
        // Extraction identity: `extractor`/`extractor_version` → method.
        if let Some(m) = opt_method(&self.extractor, &self.extractor_version)? {
            p = p.with_method(m);
        }
        // The mentioning record is the immediate parent endpoint.
        if let Some(parent) = opt_endpoint(&self.record_kind, &self.record_id)? {
            p = p.with_parent(parent);
        }
        if let Some(loc) = &self.locator_json {
            p = p.with_locator(Locator::from_json(loc)?);
        }
        if let Some(ev) = &self.created_event_id {
            p = p.with_created_event(ev.clone());
        }
        Ok(p)
    }
}

impl HasProvenance for Evidence {
    fn provenance(&self) -> MemoryResult<Provenance> {
        let mut p = Provenance::new()
            .with_source(SourceRef::new(self.source_id.clone()))
            .with_time(ProvenanceTime {
                created_at: None,
                observed_at: self.observed_at,
            });
        if let Some(a) = &self.actor_id {
            p = p.with_actor(Actor::new(a.clone())?);
        }
        if let Some(m) = opt_method(&self.method, &self.method_version)? {
            p = p.with_method(m);
        }
        // Immediate parents: the source record and source event the evidence
        // was drawn from.
        if let Some(parent) = opt_endpoint(&self.source_record_kind, &self.source_record_id)? {
            p = p.with_parent(parent);
        }
        if let Some(ev) = &self.source_event_id {
            p = p.with_parent(ParentRef::Event(ev.clone()));
        }
        if let Some(loc) = &self.locator_json {
            p = p.with_locator(Locator::from_json(loc)?);
        }
        if let Some(ev) = &self.created_event_id {
            p = p.with_created_event(ev.clone());
        }
        Ok(p)
    }
}

impl HasProvenance for Episode {
    fn provenance(&self) -> MemoryResult<Provenance> {
        // Episodes carry no `created_event_id` column; `opened_at` is their
        // observation time.
        Ok(Provenance::new()
            .with_source(SourceRef::new(self.source_id.clone()))
            .with_time(ProvenanceTime {
                created_at: None,
                observed_at: self.opened_at,
            }))
    }
}

impl HasProvenance for Goal {
    fn provenance(&self) -> MemoryResult<Provenance> {
        let mut p = Provenance::new()
            .with_source(SourceRef::new(self.source_id.clone()))
            .with_time(ProvenanceTime {
                created_at: self.created_at,
                observed_at: None,
            });
        if let Some(ev) = &self.created_event_id {
            p = p.with_created_event(ev.clone());
        }
        Ok(p)
    }
}

impl HasProvenance for ToolObservation {
    fn provenance(&self) -> MemoryResult<Provenance> {
        let mut p = Provenance::new()
            .with_source(SourceRef::new(self.source_id.clone()))
            .with_time(ProvenanceTime {
                created_at: self.created_at,
                observed_at: None,
            });
        // Tool identity + version is the producing model/algorithm identity.
        if let (Some(id), Some(version)) = (&self.tool_id, &self.tool_version) {
            p = p.with_model(ModelIdentity::new(id.clone(), version.clone())?);
        }
        // The invocation start event is the creating authority event.
        if let Some(ev) = &self.start_event_id {
            p = p.with_created_event(ev.clone());
        }
        Ok(p)
    }
}

impl HasProvenance for Feedback {
    fn provenance(&self) -> MemoryResult<Provenance> {
        let mut p = Provenance::new()
            .with_source(SourceRef::new(self.source_id.clone()))
            .with_time(ProvenanceTime {
                created_at: self.created_at,
                observed_at: None,
            });
        if let Some(a) = &self.actor_id {
            p = p.with_actor(Actor::new(a.clone())?);
        }
        if let Some(ev) = &self.event_id {
            p = p.with_created_event(ev.clone());
        }
        Ok(p)
    }
}

impl HasProvenance for ConsolidationRun {
    fn provenance(&self) -> MemoryResult<Provenance> {
        // A consolidation run has no `source_id`; its provenance is the
        // deterministic algorithm identity + version that produced the output.
        // The immediate parents are captured by `input_set_hash` (the sorted
        // parent hash, design §7.3) rather than inline ids.
        Ok(Provenance::new()
            .with_model(ModelIdentity::new(
                self.algorithm.clone(),
                self.version.clone(),
            )?)
            .with_time(ProvenanceTime {
                created_at: self.started_at,
                observed_at: self.completed_at,
            }))
    }
}

impl HasProvenance for RetrievalTrace {
    fn provenance(&self) -> MemoryResult<Provenance> {
        // A retrieval trace has no `source_id`; its provenance is the model /
        // classifier identity that shaped the response (feeds rebuild).
        let mut p = Provenance::new().with_time(ProvenanceTime {
            created_at: self.created_at,
            observed_at: None,
        });
        if let Some(v) = &self.embed_model_version {
            p = p.with_model(ModelIdentity::new("embedding_model", v.clone())?);
        }
        if let Some(v) = &self.classifier_version {
            p = p.with_method(Method::new("query_classifier", Some(v.clone()))?);
        }
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::model::PolicyPartition;

    fn span() -> Span {
        Span::new(2, 7).unwrap()
    }

    // ── Actor / Method / ModelIdentity ──────────────────────────────────
    #[test]
    fn actor_validates_and_roundtrips() {
        let a = Actor::new("user-1").unwrap();
        assert_eq!(a.as_str(), "user-1");
        let back: Actor = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back, a);
        // Empty / control-char / oversized rejected.
        assert!(Actor::new("").is_err());
        assert!(Actor::new("bad\nactor").is_err());
        assert!(Actor::new("x".repeat(PROV_FIELD_MAX_LEN + 1)).is_err());
        assert!(serde_json::from_str::<Actor>("\"bad\\nactor\"").is_err());
    }

    #[test]
    fn method_validates_and_roundtrips() {
        let m = Method::new("gliner", Some("1.2".into())).unwrap();
        assert_eq!((m.name(), m.version()), ("gliner", Some("1.2")));
        let back: Method = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
        // Versionless is allowed.
        let m2 = Method::new("regex", None).unwrap();
        assert_eq!(m2.version(), None);
        // Empty name and control-char version rejected.
        assert!(Method::new("", None).is_err());
        assert!(Method::new("ok", Some("v\t1".into())).is_err());
        // Unknown fields rejected.
        assert!(serde_json::from_str::<Method>("{\"name\":\"x\",\"leak\":\"y\"}").is_err());
    }

    #[test]
    fn model_identity_requires_id_and_version() {
        let m = ModelIdentity::new("all-MiniLM-L6-v2", "1.0").unwrap();
        assert_eq!((m.id(), m.version()), ("all-MiniLM-L6-v2", "1.0"));
        let back: ModelIdentity =
            serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
        assert!(ModelIdentity::new("", "1.0").is_err());
        assert!(ModelIdentity::new("m", "").is_err());
    }

    // ── SourceRef ───────────────────────────────────────────────────────
    #[test]
    fn source_ref_roundtrips_with_and_without_kind() {
        let bare = SourceRef::new(SourceId::new_v7());
        let back: SourceRef = serde_json::from_str(&serde_json::to_string(&bare).unwrap()).unwrap();
        assert_eq!(back, bare);
        assert_eq!(back.kind(), None);

        let kinded = SourceRef::with_kind(SourceId::new_v7(), SourceKind::Conversation);
        let back: SourceRef =
            serde_json::from_str(&serde_json::to_string(&kinded).unwrap()).unwrap();
        assert_eq!(back, kinded);
        assert_eq!(back.kind(), Some(SourceKind::Conversation));

        // Unknown source kind rejected on deserialize.
        let bad = format!("{{\"id\":\"{}\",\"kind\":\"bogus\"}}", SourceId::new_v7());
        assert!(serde_json::from_str::<SourceRef>(&bad).is_err());
    }

    // ── ParentRef ───────────────────────────────────────────────────────
    #[test]
    fn parent_ref_variants_roundtrip() {
        let parents = vec![
            ParentRef::Event(EventId::new_v7()),
            ParentRef::Record(RecordId::new_v7()),
            ParentRef::Episode(EpisodeId::new_v7()),
            ParentRef::Entity(EntityId::new_v7()),
            ParentRef::endpoint("memory", RecordId::new_v7().into_string()).unwrap(),
        ];
        for p in parents {
            let back: ParentRef =
                serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
            assert_eq!(back, p);
        }
        // Endpoint validation.
        assert!(ParentRef::endpoint("", "id").is_err());
        assert!(ParentRef::endpoint("kind", "bad\nid").is_err());
    }

    // ── Locator: policy-safe + structural round-trip ────────────────────
    #[test]
    fn locator_variants_roundtrip_through_json() {
        let locators = vec![
            Locator::document("doc-42", Some(span())).unwrap(),
            Locator::conversation_turn("conv-1", 3, None).unwrap(),
            Locator::event(EventId::new_v7(), Some(span())),
            Locator::record(RecordId::new_v7(), None),
            Locator::url("https://example.com/a", Some(span())).unwrap(),
        ];
        for loc in locators {
            let json = loc.to_json().unwrap();
            let back = Locator::from_json(&json).unwrap();
            assert_eq!(back, loc);
        }
    }

    #[test]
    fn locator_rejects_content_leak_and_bad_shapes() {
        // Control characters in a reference string (raw content) are rejected.
        assert!(Locator::document("line1\nline2", None).is_err());
        assert!(Locator::url("https://x/\npath", None).is_err());
        // A non-http url is rejected.
        assert!(Locator::url("ftp://example.com", None).is_err());
        // Oversized reference rejected.
        assert!(Locator::document("x".repeat(LOCATOR_REF_MAX_LEN + 1), None).is_err());
        // An arbitrary JSON blob carrying a smuggled content field is rejected
        // (deny_unknown_fields) rather than silently accepted.
        let leak = "{\"kind\":\"document\",\"document_id\":\"d\",\"content\":\"secret text\"}";
        assert!(Locator::from_json(leak).is_err());
        // An unknown locator kind is rejected.
        assert!(Locator::from_json("{\"kind\":\"mystery\",\"x\":1}").is_err());
        // Inverted span is rejected via Span validation.
        let bad_span =
            "{\"kind\":\"document\",\"document_id\":\"d\",\"span\":{\"start\":9,\"end\":1}}";
        assert!(Locator::from_json(bad_span).is_err());
    }

    #[test]
    fn locator_json_is_valid_json() {
        let loc = Locator::document("doc", Some(span())).unwrap();
        let json = loc.to_json().unwrap();
        // Parses as generic JSON (mirrors the column's json_valid guard).
        // Externally tagged: {"document":{"document_id":"doc","span":{...}}}.
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["document"]["document_id"], "doc");
    }

    // ── Provenance builder + round-trip ─────────────────────────────────
    #[test]
    fn provenance_builder_roundtrips() {
        let p = Provenance::new()
            .with_source(SourceRef::with_kind(SourceId::new_v7(), SourceKind::Native))
            .with_actor(Actor::new("user-1").unwrap())
            .with_method(Method::new("gliner", Some("1.2".into())).unwrap())
            .with_model(ModelIdentity::new("gliner", "1.2").unwrap())
            .with_time(ProvenanceTime {
                created_at: Some(UtcTimestamp::now()),
                observed_at: Some(UtcTimestamp::now()),
            })
            .with_parent(ParentRef::Event(EventId::new_v7()))
            .with_locator(Locator::document("doc-1", Some(span())).unwrap())
            .with_created_event(EventId::new_v7());
        let back: Provenance = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn empty_provenance_serializes_compactly() {
        let p = Provenance::new();
        // All fields skip when unset.
        assert_eq!(serde_json::to_string(&p).unwrap(), "{}");
        let back: Provenance = serde_json::from_str("{}").unwrap();
        assert_eq!(back, p);
    }

    // ── HasProvenance mappings ──────────────────────────────────────────
    #[test]
    fn record_provenance_maps_source_event_and_time() {
        use crate::memory::model::record::{Record, RecordKind, RecordPayload};
        use crate::memory::model::SchemaVersion;
        use crate::memory::model::ValidInterval;

        let source_id = SourceId::new_v7();
        let event_id = EventId::new_v7();
        let created_at = UtcTimestamp::now();
        let r = Record {
            id: RecordId::new_v7(),
            record_kind: RecordKind::Memory,
            schema_version: SchemaVersion::new(1),
            payload: RecordPayload::Plaintext("hi".into()),
            content_hash: None,
            truth_state: None,
            staleness_class: None,
            valid_interval: ValidInterval::open(),
            policy: PolicyPartition::new("user", "chat", 0).unwrap(),
            source_id: source_id.clone(),
            policy_version: "p1".into(),
            created_event_id: event_id.clone(),
            created_at,
            superseded_by: None,
            episode_id: None,
            goal_context_id: None,
            estimated_tokens: None,
            shred_key_id: None,
            key_version: None,
        };
        let prov = r.provenance().unwrap();
        assert_eq!(prov.source.as_ref().unwrap().id(), &source_id);
        assert_eq!(prov.created_event_id.as_ref().unwrap(), &event_id);
        assert_eq!(prov.time.created_at, Some(created_at));
        assert!(prov.parents.is_empty());
        assert!(prov.locator.is_none());
    }

    #[test]
    fn mention_provenance_reconciles_extractor_and_locator() {
        use crate::memory::model::entity::Mention;
        use crate::memory::model::MentionId;

        let loc = Locator::conversation_turn("conv-1", 2, Some(span()))
            .unwrap()
            .to_json()
            .unwrap();
        let m = Mention {
            id: MentionId::new_v7(),
            record_id: Some("rec-1".into()),
            record_kind: Some("memory".into()),
            entity_id: EntityId::new_v7(),
            locator_json: Some(loc),
            span: Some(span()),
            role: Some("subject".into()),
            extractor: Some("gliner".into()),
            extractor_version: Some("1.2".into()),
            score: Some(0.9),
            score_semantics: Some("model_prob".into()),
            policy: PolicyPartition::new("user", "chat", 0).unwrap(),
            source_id: SourceId::new_v7(),
            policy_version: "p1".into(),
            observed_at: Some(UtcTimestamp::now()),
            created_event_id: Some(EventId::new_v7()),
        };
        let prov = m.provenance().unwrap();
        // extractor/version → method
        let method = prov.method.as_ref().unwrap();
        assert_eq!((method.name(), method.version()), ("gliner", Some("1.2")));
        // record endpoint → immediate parent
        assert_eq!(
            prov.parents,
            vec![ParentRef::endpoint("memory", "rec-1").unwrap()]
        );
        // locator parsed back into a typed policy-safe locator
        assert!(matches!(
            prov.locator,
            Some(Locator::ConversationTurn { .. })
        ));
        assert!(prov.time.observed_at.is_some());
    }

    #[test]
    fn mention_provenance_rejects_malformed_locator() {
        use crate::memory::model::entity::Mention;
        use crate::memory::model::MentionId;

        let m = Mention {
            id: MentionId::new_v7(),
            record_id: None,
            record_kind: None,
            entity_id: EntityId::new_v7(),
            // Valid JSON per json_valid, but not a policy-safe Locator shape.
            locator_json: Some("{\"raw\":\"leaked content\"}".into()),
            span: None,
            role: None,
            extractor: None,
            extractor_version: None,
            score: None,
            score_semantics: None,
            policy: PolicyPartition::new("user", "chat", 0).unwrap(),
            source_id: SourceId::new_v7(),
            policy_version: "p1".into(),
            observed_at: None,
            created_event_id: None,
        };
        assert!(m.provenance().is_err());
    }

    #[test]
    fn evidence_provenance_captures_parents_and_actor() {
        use crate::memory::model::entity::{Evidence, EvidencePolarity};
        use crate::memory::model::EvidenceId;

        let source_event = EventId::new_v7();
        let e = Evidence {
            id: EvidenceId::new_v7(),
            subject_kind: Some("record".into()),
            subject_id: Some("rec-9".into()),
            source_record_kind: Some("memory".into()),
            source_record_id: Some("rec-1".into()),
            source_event_id: Some(source_event.clone()),
            locator_json: None,
            actor_id: Some("assistant".into()),
            method: Some("nli".into()),
            method_version: Some("v3".into()),
            polarity: EvidencePolarity::Supports,
            score: None,
            score_semantics: None,
            policy: PolicyPartition::new("user", "chat", 0).unwrap(),
            source_id: SourceId::new_v7(),
            policy_version: "p1".into(),
            observed_at: Some(UtcTimestamp::now()),
            removed_at: None,
            created_event_id: None,
        };
        let prov = e.provenance().unwrap();
        assert_eq!(prov.actor.as_ref().unwrap().as_str(), "assistant");
        assert_eq!(prov.method.as_ref().unwrap().name(), "nli");
        // Both the source record endpoint and the source event are parents.
        assert_eq!(prov.parents.len(), 2);
        assert!(prov
            .parents
            .contains(&ParentRef::endpoint("memory", "rec-1").unwrap()));
        assert!(prov.parents.contains(&ParentRef::Event(source_event)));
    }

    #[test]
    fn consolidation_and_retrieval_trace_carry_model_identity() {
        use crate::memory::model::consolidation::{ConsolidationLevel, ConsolidationRun};
        use crate::memory::model::observation::RetrievalTrace;
        use crate::memory::model::{ConsolidationRunId, RetrievalTraceId};

        let run = ConsolidationRun {
            id: ConsolidationRunId::new_v7(),
            algorithm: "episodic_summary".into(),
            version: "v1".into(),
            input_set_hash: "abc".into(),
            level: ConsolidationLevel::Summary,
            cursor: None,
            status: None,
            started_at: Some(UtcTimestamp::now()),
            completed_at: None,
            output_id: None,
            error_code: None,
        };
        let prov = run.provenance().unwrap();
        let model = prov.model.as_ref().unwrap();
        assert_eq!((model.id(), model.version()), ("episodic_summary", "v1"));
        assert!(prov.source.is_none());

        let trace = RetrievalTrace {
            id: RetrievalTraceId::new_v7(),
            response_id: None,
            task_id: None,
            query_hash: None,
            query_class: None,
            classifier_version: Some("c1".into()),
            profile_id: None,
            graph_revision: None,
            policy_hash: None,
            token_budget: None,
            status: None,
            degradation_json: None,
            embed_model_version: Some("v2".into()),
            rerank_model_version: None,
            created_at: Some(UtcTimestamp::now()),
        };
        let prov = trace.provenance().unwrap();
        assert_eq!(prov.model.as_ref().unwrap().id(), "embedding_model");
        assert_eq!(prov.method.as_ref().unwrap().name(), "query_classifier");
        assert!(prov.source.is_none());
    }
}
