//! Mention provenance types for conservative entity resolution (design §7.1,
//! task F2.5.2, MGR-019).
//!
//! ## Key invariant
//!
//! When a strong identifier (email, URL, external ID) resolves to an **existing**
//! canonical entity, the provenance of the mention — where it appeared, how it
//! was extracted, its role in the text — MUST still be appended as a new
//! `mentions` row. Resolution does **not** consume or suppress provenance.
//!
//! Design §7.1: "Append mention locator/span/role/extractor/version/
//! score-semantics provenance even when a strong identifier resolves to an
//! existing canonical entity."
//!
//! MGR-019: "THE Cognitive_Memory_System SHALL preserve mention
//! locator/span/role/extractor/version/score-semantics provenance regardless of
//! whether the mention resolves to an existing entity."

use serde::{Deserialize, Serialize};

use super::{
    identifier::{IdentifierType, NormalizedIdentifier},
    EntityId,
};

// ── MentionSpan ──────────────────────────────────────────────────────────────

/// A character span `[start, end)` for a mention location.
///
/// This is a simple struct (not the validated [`super::entity::Span`]) so that
/// the provenance layer can be constructed without depending on the validated
/// `Span` smart constructor. The caller is responsible for providing a
/// semantically valid span; this type is a plain data carrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MentionSpan {
    /// Inclusive start offset (characters).
    pub start: u32,
    /// Exclusive end offset (characters).
    pub end: u32,
}

// ── MentionInput ─────────────────────────────────────────────────────────────

/// Raw input describing a potential mention, before identifier resolution.
///
/// This is the caller-supplied data that the [`MentionProvenanceBuilder`] turns
/// into a [`MentionProvenanceRecord`] after resolution is determined.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MentionInput {
    /// The raw identifier text (e.g. `"ada@example.com"`, `"Ada Lovelace"`).
    pub raw_identifier: String,
    /// The type of identifier (determines strong/weak and normalization).
    pub id_type: IdentifierType,
    /// The source record kind containing this mention (e.g. `"memory"`, `"event"`).
    pub record_kind: String,
    /// The source record ID containing this mention.
    pub record_id: String,
    /// Policy-safe structured locator JSON (where in the record this mention
    /// appears).
    pub locator_json: Option<String>,
    /// Character span within the source record, if known.
    pub span: Option<MentionSpan>,
    /// The role the entity plays in the mention context.
    pub role: Option<String>,
    /// The extractor that found this mention.
    pub extractor: Option<String>,
    /// The extractor version.
    pub extractor_version: Option<String>,
    /// The extraction score (algorithm-specific; not a probability).
    ///
    /// Per MGR-001 AC 3, a score without `score_semantics` is a bare score and
    /// will be stripped by [`MentionProvenanceBuilder::validated_score`].
    pub score: Option<f64>,
    /// What the score means (required when `score` is present to prevent bare
    /// scores).
    pub score_semantics: Option<String>,
}

// ── MentionProvenanceRecord ───────────────────────────────────────────────────

/// A complete mention provenance record to be appended to the authority store.
///
/// This is the data that MUST be written as a `mentions` row regardless of
/// whether the mention's identifier resolved to an existing canonical entity.
///
/// Design §7.1: "Append mention locator/span/role/extractor/version/
/// score-semantics provenance even when a strong identifier resolves to an
/// existing canonical entity."
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MentionProvenanceRecord {
    /// The entity this mention refers to (resolved canonical ID, or a new
    /// entity ID).
    pub entity_id: EntityId,
    /// The source record kind containing this mention (e.g. `"memory"`,
    /// `"event"`).
    pub record_kind: String,
    /// The source record ID containing this mention.
    pub record_id: String,
    /// Policy-safe structured locator JSON (where in the record this mention
    /// appears).
    pub locator_json: Option<String>,
    /// Character span within the source record (if known).
    pub span: Option<MentionSpan>,
    /// The role the entity plays in this mention context.
    pub role: Option<String>,
    /// The extractor that found this mention.
    pub extractor: Option<String>,
    /// The extractor version.
    pub extractor_version: Option<String>,
    /// The extraction score (algorithm-specific; not a probability).
    ///
    /// `None` when the score was stripped because `score_semantics` was absent
    /// (bare score not permitted per MGR-001 AC 3).
    pub score: Option<f64>,
    /// What the score means (required when `score` is present).
    pub score_semantics: Option<String>,
    /// The normalized identifier that led to this resolution, stored for
    /// provenance of how the resolution was performed.
    pub resolved_via: NormalizedIdentifier,
}

// ── MentionResolutionResult ───────────────────────────────────────────────────

/// The result of attempting to resolve a mention's identifier to a canonical
/// entity.
///
/// Regardless of resolution outcome, the mention's provenance MUST be appended.
/// No variant suppresses the [`MentionProvenanceRecord`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MentionResolutionResult {
    /// Strong identifier resolved to an existing canonical entity.
    /// Provenance is appended to that entity's mention list.
    ResolvedToExisting {
        /// The existing canonical entity.
        canonical_entity_id: EntityId,
        /// The provenance record that MUST be appended.
        appended_mention: MentionProvenanceRecord,
    },
    /// Strong identifier resolved to a new entity (not yet in authority).
    /// Both the entity and the mention provenance are new records.
    ResolvedToNew {
        /// The newly-created entity ID.
        new_entity_id: EntityId,
        /// The provenance record that MUST be appended.
        appended_mention: MentionProvenanceRecord,
    },
    /// Weak identifier — no automatic resolution. A proposal may be created
    /// separately (task 2.5.3). Provenance is appended to an unresolved pool.
    Unresolved {
        /// The provenance record that MUST be appended (with a placeholder
        /// entity id).
        appended_mention: MentionProvenanceRecord,
    },
}

impl MentionResolutionResult {
    /// The appended provenance record, regardless of variant.
    pub fn appended_mention(&self) -> &MentionProvenanceRecord {
        match self {
            MentionResolutionResult::ResolvedToExisting {
                appended_mention, ..
            } => appended_mention,
            MentionResolutionResult::ResolvedToNew {
                appended_mention, ..
            } => appended_mention,
            MentionResolutionResult::Unresolved { appended_mention } => appended_mention,
        }
    }
}

// ── MentionProvenanceBuilder ──────────────────────────────────────────────────

/// Builds [`MentionProvenanceRecord`]s for the three resolution outcomes.
///
/// The key behavioral rules enforced by this builder:
///
/// 1. **Provenance always appended**: ALL input fields (locator, span, role,
///    extractor, extractor_version, score, score_semantics) are preserved in
///    the returned record. Resolution to an existing entity does NOT suppress
///    any provenance field.
///
/// 2. **Bare score stripping**: Per MGR-001 AC 3, a `score` without
///    `score_semantics` is a bare score and MUST be stripped (set to `None`).
///    [`validated_score`] enforces this.
///
/// 3. **Score semantics required**: When `score.is_some()` and
///    `score_semantics.is_none()`, both are set to `None` in the output.
///
/// 4. **`resolved_via` captures the normalized form**: The
///    [`NormalizedIdentifier`] (including original, canonical, type, strength)
///    is stored for provenance of how the resolution was performed.
pub struct MentionProvenanceBuilder;

impl MentionProvenanceBuilder {
    /// Build a mention provenance record for an **existing** canonical entity.
    ///
    /// This is the key method for task 2.5.2: when a strong identifier
    /// resolves to an existing entity, provenance MUST still be appended.
    ///
    /// All locator/span/role/extractor/version/score-semantics fields from
    /// `input` are preserved in the returned [`MentionProvenanceRecord`].
    pub fn build_for_existing(
        entity_id: EntityId,
        input: &MentionInput,
        normalized: NormalizedIdentifier,
    ) -> MentionProvenanceRecord {
        let (score, score_semantics) =
            Self::validated_score(input.score, input.score_semantics.clone());
        MentionProvenanceRecord {
            entity_id,
            record_kind: input.record_kind.clone(),
            record_id: input.record_id.clone(),
            locator_json: input.locator_json.clone(),
            span: input.span,
            role: input.role.clone(),
            extractor: input.extractor.clone(),
            extractor_version: input.extractor_version.clone(),
            score,
            score_semantics,
            resolved_via: normalized,
        }
    }

    /// Build a mention provenance record for a **new** entity (not yet in
    /// authority).
    ///
    /// Same provenance rules as [`build_for_existing`](Self::build_for_existing).
    pub fn build_for_new(
        new_entity_id: EntityId,
        input: &MentionInput,
        normalized: NormalizedIdentifier,
    ) -> MentionProvenanceRecord {
        let (score, score_semantics) =
            Self::validated_score(input.score, input.score_semantics.clone());
        MentionProvenanceRecord {
            entity_id: new_entity_id,
            record_kind: input.record_kind.clone(),
            record_id: input.record_id.clone(),
            locator_json: input.locator_json.clone(),
            span: input.span,
            role: input.role.clone(),
            extractor: input.extractor.clone(),
            extractor_version: input.extractor_version.clone(),
            score,
            score_semantics,
            resolved_via: normalized,
        }
    }

    /// Build a mention provenance record for an **unresolved** (weak)
    /// identifier.
    ///
    /// Uses the `placeholder_entity_id` supplied by the caller; the caller must
    /// handle the unresolved case separately (e.g. via proposal creation in
    /// task 2.5.3).
    ///
    /// Same provenance rules as [`build_for_existing`](Self::build_for_existing).
    pub fn build_unresolved(
        input: &MentionInput,
        normalized: NormalizedIdentifier,
        placeholder_entity_id: EntityId,
    ) -> MentionProvenanceRecord {
        let (score, score_semantics) =
            Self::validated_score(input.score, input.score_semantics.clone());
        MentionProvenanceRecord {
            entity_id: placeholder_entity_id,
            record_kind: input.record_kind.clone(),
            record_id: input.record_id.clone(),
            locator_json: input.locator_json.clone(),
            span: input.span,
            role: input.role.clone(),
            extractor: input.extractor.clone(),
            extractor_version: input.extractor_version.clone(),
            score,
            score_semantics,
            resolved_via: normalized,
        }
    }

    /// Validate the score/score_semantics pairing and strip bare scores.
    ///
    /// Returns:
    /// - `(Some(score), Some(semantics))` when both are present.
    /// - `(None, None)` when `score` is present but `score_semantics` is `None`
    ///   (bare score stripped — MGR-001 AC 3).
    /// - `(None, None)` when neither is present.
    /// - `(None, Some(semantics))` is normalised to `(None, None)` (semantics
    ///   without a score is discarded — no phantom annotation).
    pub fn validated_score(
        score: Option<f64>,
        score_semantics: Option<String>,
    ) -> (Option<f64>, Option<String>) {
        match (score, score_semantics) {
            // Both present → keep.
            (Some(s), Some(sem)) => (Some(s), Some(sem)),
            // Score without semantics → bare score, strip both.
            (Some(_), None) => (None, None),
            // Semantics without score → no score annotation, discard semantics.
            (None, Some(_)) => (None, None),
            // Neither → nothing to do.
            (None, None) => (None, None),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::identifier::{IdentifierNormalizer, IdentifierStrength};

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_input(record_kind: &str, record_id: &str) -> MentionInput {
        MentionInput {
            raw_identifier: "ada@example.com".into(),
            id_type: IdentifierType::Email,
            record_kind: record_kind.into(),
            record_id: record_id.into(),
            locator_json: Some(r#"{"section":"header"}"#.into()),
            span: Some(MentionSpan { start: 10, end: 24 }),
            role: Some("author".into()),
            extractor: Some("ner-v1".into()),
            extractor_version: Some("1.2.3".into()),
            score: Some(0.95),
            score_semantics: Some("ner_confidence".into()),
        }
    }

    fn normalized_email() -> NormalizedIdentifier {
        IdentifierNormalizer::normalize_email("ada@example.com").unwrap()
    }

    // ── validated_score ───────────────────────────────────────────────────────

    #[test]
    fn validated_score_both_present_kept() {
        let (s, sem) =
            MentionProvenanceBuilder::validated_score(Some(0.9), Some("cosine_sim".into()));
        assert_eq!(s, Some(0.9));
        assert_eq!(sem.as_deref(), Some("cosine_sim"));
    }

    #[test]
    fn validated_score_bare_score_stripped() {
        // score without semantics → both become None (MGR-001 AC 3).
        let (s, sem) = MentionProvenanceBuilder::validated_score(Some(0.9), None);
        assert_eq!(s, None, "bare score must be stripped");
        assert_eq!(sem, None);
    }

    #[test]
    fn validated_score_semantics_without_score_discarded() {
        let (s, sem) = MentionProvenanceBuilder::validated_score(None, Some("cosine_sim".into()));
        assert_eq!(s, None);
        assert_eq!(sem, None, "semantics without score must be discarded");
    }

    #[test]
    fn validated_score_neither_returns_none_none() {
        let (s, sem) = MentionProvenanceBuilder::validated_score(None, None);
        assert_eq!(s, None);
        assert_eq!(sem, None);
    }

    // ── build_for_existing ────────────────────────────────────────────────────

    #[test]
    fn build_for_existing_all_provenance_fields_preserved() {
        let canonical_id = EntityId::new_v7();
        let input = make_input("memory", "rec-001");
        let norm = normalized_email();

        let record = MentionProvenanceBuilder::build_for_existing(
            canonical_id.clone(),
            &input,
            norm.clone(),
        );

        // entity_id must point to the EXISTING canonical entity.
        assert_eq!(record.entity_id, canonical_id);
        // ALL provenance fields must be preserved — resolution must not suppress any.
        assert_eq!(record.record_kind, "memory");
        assert_eq!(record.record_id, "rec-001");
        assert_eq!(
            record.locator_json.as_deref(),
            Some(r#"{"section":"header"}"#)
        );
        assert_eq!(record.span, Some(MentionSpan { start: 10, end: 24 }));
        assert_eq!(record.role.as_deref(), Some("author"));
        assert_eq!(record.extractor.as_deref(), Some("ner-v1"));
        assert_eq!(record.extractor_version.as_deref(), Some("1.2.3"));
        assert_eq!(record.score, Some(0.95));
        assert_eq!(record.score_semantics.as_deref(), Some("ner_confidence"));
        // resolved_via captures the normalized identifier.
        assert_eq!(record.resolved_via.canonical, norm.canonical);
        assert_eq!(record.resolved_via.strength, IdentifierStrength::Strong);
    }

    #[test]
    fn build_for_existing_bare_score_stripped() {
        let canonical_id = EntityId::new_v7();
        let mut input = make_input("memory", "rec-002");
        // Provide a score without semantics — should be stripped.
        input.score = Some(0.77);
        input.score_semantics = None;

        let record =
            MentionProvenanceBuilder::build_for_existing(canonical_id, &input, normalized_email());

        assert_eq!(record.score, None, "bare score must be stripped");
        assert_eq!(record.score_semantics, None);
        // All other provenance fields still preserved.
        assert_eq!(record.record_kind, "memory");
        assert_eq!(record.role.as_deref(), Some("author"));
        assert_eq!(record.extractor.as_deref(), Some("ner-v1"));
    }

    #[test]
    fn build_for_existing_score_and_semantics_both_preserved() {
        let canonical_id = EntityId::new_v7();
        let input = make_input("memory", "rec-003"); // score=0.95, semantics="ner_confidence"

        let record =
            MentionProvenanceBuilder::build_for_existing(canonical_id, &input, normalized_email());

        assert_eq!(record.score, Some(0.95));
        assert_eq!(record.score_semantics.as_deref(), Some("ner_confidence"));
    }

    // ── build_for_new ─────────────────────────────────────────────────────────

    #[test]
    fn build_for_new_all_provenance_fields_preserved() {
        let new_id = EntityId::new_v7();
        let input = make_input("event", "evt-007");
        let norm = normalized_email();

        let record = MentionProvenanceBuilder::build_for_new(new_id.clone(), &input, norm.clone());

        assert_eq!(record.entity_id, new_id);
        assert_eq!(record.record_kind, "event");
        assert_eq!(record.record_id, "evt-007");
        assert_eq!(
            record.locator_json.as_deref(),
            Some(r#"{"section":"header"}"#)
        );
        assert_eq!(record.span, Some(MentionSpan { start: 10, end: 24 }));
        assert_eq!(record.role.as_deref(), Some("author"));
        assert_eq!(record.extractor.as_deref(), Some("ner-v1"));
        assert_eq!(record.extractor_version.as_deref(), Some("1.2.3"));
        assert_eq!(record.score, Some(0.95));
        assert_eq!(record.score_semantics.as_deref(), Some("ner_confidence"));
        assert_eq!(record.resolved_via.canonical, norm.canonical);
    }

    #[test]
    fn build_for_new_bare_score_stripped() {
        let new_id = EntityId::new_v7();
        let mut input = make_input("event", "evt-008");
        input.score = Some(0.5);
        input.score_semantics = None;

        let record = MentionProvenanceBuilder::build_for_new(new_id, &input, normalized_email());

        assert_eq!(record.score, None);
        assert_eq!(record.score_semantics, None);
        // Other fields preserved.
        assert_eq!(
            record.locator_json.as_deref(),
            Some(r#"{"section":"header"}"#)
        );
    }

    // ── build_unresolved ──────────────────────────────────────────────────────

    #[test]
    fn build_unresolved_provenance_preserved_for_weak_identifier() {
        let placeholder_id = EntityId::new_v7();
        let mut input = make_input("memory", "rec-009");
        // Weak identifier: name type.
        input.raw_identifier = "Ada Lovelace".into();
        input.id_type = IdentifierType::Name;

        let norm = IdentifierNormalizer::normalize_name("Ada Lovelace");

        let record = MentionProvenanceBuilder::build_unresolved(
            &input,
            norm.clone(),
            placeholder_id.clone(),
        );

        assert_eq!(record.entity_id, placeholder_id);
        assert_eq!(record.record_kind, "memory");
        assert_eq!(record.record_id, "rec-009");
        assert_eq!(
            record.locator_json.as_deref(),
            Some(r#"{"section":"header"}"#)
        );
        assert_eq!(record.span, Some(MentionSpan { start: 10, end: 24 }));
        assert_eq!(record.role.as_deref(), Some("author"));
        assert_eq!(record.extractor.as_deref(), Some("ner-v1"));
        assert_eq!(record.extractor_version.as_deref(), Some("1.2.3"));
        assert_eq!(record.score, Some(0.95));
        assert_eq!(record.score_semantics.as_deref(), Some("ner_confidence"));
        assert_eq!(record.resolved_via.strength, IdentifierStrength::Weak);
        assert_eq!(record.resolved_via.canonical, "ada lovelace");
    }

    #[test]
    fn build_unresolved_bare_score_stripped() {
        let placeholder_id = EntityId::new_v7();
        let mut input = make_input("memory", "rec-010");
        input.score = Some(0.3);
        input.score_semantics = None;
        let norm = normalized_email();

        let record = MentionProvenanceBuilder::build_unresolved(&input, norm, placeholder_id);

        assert_eq!(record.score, None);
        assert_eq!(record.score_semantics, None);
    }

    // ── MentionResolutionResult helper ────────────────────────────────────────

    #[test]
    fn resolution_result_appended_mention_accessor_all_variants() {
        let canonical_id = EntityId::new_v7();
        let new_id = EntityId::new_v7();
        let placeholder_id = EntityId::new_v7();
        let input = make_input("memory", "rec-011");
        let norm = normalized_email();

        let existing = MentionResolutionResult::ResolvedToExisting {
            canonical_entity_id: canonical_id.clone(),
            appended_mention: MentionProvenanceBuilder::build_for_existing(
                canonical_id,
                &input,
                norm.clone(),
            ),
        };
        assert_eq!(existing.appended_mention().record_id, "rec-011");

        let new_result = MentionResolutionResult::ResolvedToNew {
            new_entity_id: new_id.clone(),
            appended_mention: MentionProvenanceBuilder::build_for_new(new_id, &input, norm.clone()),
        };
        assert_eq!(new_result.appended_mention().record_kind, "memory");

        let unresolved = MentionResolutionResult::Unresolved {
            appended_mention: MentionProvenanceBuilder::build_unresolved(
                &input,
                norm,
                placeholder_id,
            ),
        };
        assert_eq!(
            unresolved.appended_mention().extractor.as_deref(),
            Some("ner-v1")
        );
    }

    // ── serde roundtrips ──────────────────────────────────────────────────────

    #[test]
    fn mention_span_serde_roundtrip() {
        let span = MentionSpan { start: 5, end: 20 };
        let json = serde_json::to_string(&span).unwrap();
        let back: MentionSpan = serde_json::from_str(&json).unwrap();
        assert_eq!(back, span);
    }

    #[test]
    fn mention_provenance_record_serde_roundtrip() {
        let record = MentionProvenanceBuilder::build_for_existing(
            EntityId::new_v7(),
            &make_input("memory", "rec-012"),
            normalized_email(),
        );
        let json = serde_json::to_string(&record).unwrap();
        let back: MentionProvenanceRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.record_kind, record.record_kind);
        assert_eq!(back.record_id, record.record_id);
        assert_eq!(back.score, record.score);
        assert_eq!(back.score_semantics, record.score_semantics);
    }
}
