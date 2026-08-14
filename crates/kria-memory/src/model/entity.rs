//! Entity graph value objects: [`Entity`], [`Alias`], [`Mention`], and
//! [`Evidence`] (design §4.2, task F2.1.1).
//!
//! These are the Rust counterparts of the `entities_v2`, `aliases`, `mentions`,
//! and `evidence_v2` rows. Endpoints that are polymorphic across mixed record
//! kinds in the schema (`mentions.record_id`, `evidence_v2.subject_id`,
//! `evidence_v2.source_record_id`) carry no hard FK; here they are typed as an
//! opaque kind/id pair validated at the write boundary in later tasks.
//!
//! `EvidencePolarity` is a **closed** set (schema `CHECK(supports/contradicts)`);
//! `truth_state` is forward-compatible ([`TruthState`]); `entity_type`,
//! `alias_type`, and the mention/evidence method/role fields are free text.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::{
    encoding_err, AliasId, EntityId, EventId, EvidenceId, MentionId, PolicyPartition, SourceId,
    UtcTimestamp, ValidInterval,
};
use crate::error::MemoryResult;
use crate::model::truth::TruthState;

/// A graph entity (`entities_v2` row — design §4.2). A person/project/tool/
/// concept. `canonical_id` points at the canonical entity when this row is a
/// non-canonical duplicate (name/embedding never auto-merge persons — that is
/// governed entity resolution in F2.5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    /// Stable identity (`entities_v2.id`).
    pub id: EntityId,
    /// The canonical entity this resolves to, if any (self-reference).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_id: Option<EntityId>,
    /// Free-text entity type (e.g. `person`, `project`, `tool`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    /// Human-facing display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Normalized form of the display name (index target).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_name: Option<String>,
    /// Truth/lifecycle disposition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truth_state: Option<TruthState>,
    /// Policy partition.
    pub policy: PolicyPartition,
    /// Contributing source id.
    pub source_id: SourceId,
    /// Effective policy version tag.
    pub policy_version: String,
    /// The creating authority event.
    pub created_event_id: EventId,
    /// Transaction-time creation instant.
    pub created_at: UtcTimestamp,
    /// Authority revision at last write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
}

/// An alternative surface form for an entity (`aliases` row — design §4.2).
/// Unique active identity is `(normalized_alias, alias_type, namespace, scope)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Alias {
    /// Stable identity (`aliases.id`).
    pub id: AliasId,
    /// The entity this alias names.
    pub entity_id: EntityId,
    /// The raw alias surface form.
    pub alias: String,
    /// The normalized alias (part of the uniqueness key).
    pub normalized_alias: String,
    /// The alias type (part of the uniqueness key).
    pub alias_type: String,
    /// Truth/lifecycle disposition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truth_state: Option<TruthState>,
    /// Policy partition (namespace/scope are part of the uniqueness key).
    pub policy: PolicyPartition,
    /// Contributing source id.
    pub source_id: SourceId,
    /// Effective policy version tag.
    pub policy_version: String,
    /// The creating authority event, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_event_id: Option<EventId>,
    /// Transaction-time creation instant, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<UtcTimestamp>,
    /// Half-open valid interval.
    #[serde(default = "ValidInterval::open")]
    pub valid_interval: ValidInterval,
}

/// A provenance-bearing link from a source span/locator to an entity
/// (`mentions` row — design §4.2). `span_end >= span_start` when both present.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mention {
    /// Stable identity (`mentions.id`).
    pub id: MentionId,
    /// The record this mention appears in (polymorphic endpoint kind + id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    /// The kind of the mentioning record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_kind: Option<String>,
    /// The mentioned entity.
    pub entity_id: EntityId,
    /// Policy-safe structured locator (validated JSON); refined in 2.1.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator_json: Option<String>,
    /// Mention span (character offsets); `span_end >= span_start`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    /// The role the entity plays in the mention.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Extraction method identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extractor: Option<String>,
    /// Extraction method version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extractor_version: Option<String>,
    /// Extraction score (semantics named separately).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// What `score` means (design forbids bare "confidence").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_semantics: Option<String>,
    /// Policy partition.
    pub policy: PolicyPartition,
    /// Contributing source id.
    pub source_id: SourceId,
    /// Effective policy version tag.
    pub policy_version: String,
    /// When the mention was observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<UtcTimestamp>,
    /// The creating authority event, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_event_id: Option<EventId>,
}

/// A validated mention/text span: `[start, end)` with `end >= start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Span {
    start: u32,
    end: u32,
}

impl Span {
    /// Construct a span, rejecting `end < start`.
    pub fn new(start: u32, end: u32) -> MemoryResult<Self> {
        if end < start {
            return Err(encoding_err(format!(
                "inverted span: end {end} < start {start}"
            )));
        }
        Ok(Self { start, end })
    }

    /// The inclusive start offset.
    pub fn start(&self) -> u32 {
        self.start
    }

    /// The exclusive end offset.
    pub fn end(&self) -> u32 {
        self.end
    }
}

/// Serde surrogate so a span validates on deserialize.
#[derive(Deserialize)]
struct SpanRaw {
    start: u32,
    end: u32,
}

impl<'de> Deserialize<'de> for Span {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = SpanRaw::deserialize(de)?;
        Span::new(raw.start, raw.end).map_err(serde::de::Error::custom)
    }
}

/// Whether an evidence artifact supports or contradicts its subject
/// (`evidence_v2.polarity` `CHECK(supports/contradicts)`). A **closed** set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidencePolarity {
    /// The evidence supports the subject claim.
    Supports,
    /// The evidence contradicts the subject claim.
    Contradicts,
}

impl EvidencePolarity {
    /// The canonical text form stored in `polarity`.
    pub fn as_str(self) -> &'static str {
        match self {
            EvidencePolarity::Supports => "supports",
            EvidencePolarity::Contradicts => "contradicts",
        }
    }
}

impl FromStr for EvidencePolarity {
    type Err = crate::error::MemoryError;
    fn from_str(s: &str) -> MemoryResult<Self> {
        Ok(match s {
            "supports" => EvidencePolarity::Supports,
            "contradicts" => EvidencePolarity::Contradicts,
            other => return Err(encoding_err(format!("unknown evidence polarity {other:?}"))),
        })
    }
}

impl std::fmt::Display for EvidencePolarity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EvidencePolarity {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A supporting/contradicting observation about a subject (`evidence_v2` row —
/// design §4.2). Subject and source-record endpoints are polymorphic (kind +
/// id).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    /// Stable identity (`evidence_v2.id`).
    pub id: EvidenceId,
    /// The subject this evidence bears on (polymorphic endpoint kind + id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    /// The source record the evidence came from (polymorphic endpoint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_record_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_record_id: Option<String>,
    /// The originating authority event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_id: Option<EventId>,
    /// Policy-safe structured locator (validated JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator_json: Option<String>,
    /// The actor that recorded the evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    /// The extraction/assessment method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// The method version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method_version: Option<String>,
    /// Supports or contradicts (closed set).
    pub polarity: EvidencePolarity,
    /// Evidence score (semantics named separately).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// What `score` means.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_semantics: Option<String>,
    /// Policy partition.
    pub policy: PolicyPartition,
    /// Contributing source id.
    pub source_id: SourceId,
    /// Effective policy version tag.
    pub policy_version: String,
    /// When the evidence was observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<UtcTimestamp>,
    /// When the evidence was retracted, if ever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed_at: Option<UtcTimestamp>,
    /// The creating authority event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_event_id: Option<EventId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_rejects_inverted_and_roundtrips() {
        assert!(Span::new(5, 3).is_err());
        let s = Span::new(2, 7).unwrap();
        assert_eq!((s.start(), s.end()), (2, 7));
        let back: Span = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        // Inverted span rejected on deserialize.
        assert!(serde_json::from_str::<Span>("{\"start\":9,\"end\":1}").is_err());
    }

    #[test]
    fn evidence_polarity_roundtrip_and_rejects_unknown() {
        assert_eq!(
            EvidencePolarity::from_str("supports").unwrap(),
            EvidencePolarity::Supports
        );
        assert_eq!(
            EvidencePolarity::from_str("contradicts").unwrap(),
            EvidencePolarity::Contradicts
        );
        assert!(EvidencePolarity::from_str("maybe").is_err());
        assert!(serde_json::from_str::<EvidencePolarity>("\"maybe\"").is_err());
    }

    #[test]
    fn entity_serde_roundtrips() {
        let e = Entity {
            id: EntityId::new_v7(),
            canonical_id: None,
            entity_type: Some("person".into()),
            display_name: Some("Ada Lovelace".into()),
            normalized_name: Some("ada lovelace".into()),
            truth_state: Some(TruthState::Current),
            policy: PolicyPartition::new("user", "chat", 0).unwrap(),
            source_id: SourceId::new_v7(),
            policy_version: "p1".into(),
            created_event_id: EventId::new_v7(),
            created_at: UtcTimestamp::now(),
            revision: Some(1),
        };
        let back: Entity = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }
}
