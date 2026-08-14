//! Unresolved entity resolution proposals for conservative reversible entity
//! resolution (design §7.1, task F2.5.3, MGR-019).
//!
//! Design §7.1: "Create unresolved proposals for name/fuzzy/vector similarity
//! with feature version, rationale, policy, base revision, and no topology
//! mutation."
//!
//! Design §4.2: `entity_resolution_proposals` schema:
//! ```text
//!   id PK, left_entity_id, right_entity_id, rationale_json, features_version,
//!   status CHECK(unresolved/accepted/rejected/reversed), base_revision,
//!   policy columns, created_event_id, resolved_event_id
//! ```
//!
//! ## Key invariants
//!
//! - A proposal creates **NO topology mutation**: no entity edges, no merges,
//!   no aliases are created. The proposal is a record-only artifact.
//! - A proposal starts in `Unresolved` status; only governance changes it.
//! - `left_entity_id != right_entity_id` — self-proposals are rejected.
//! - `similarity_score` without `score_semantics` → both stripped (bare score).

use serde::{Deserialize, Serialize};

use super::{EntityId, GraphRevision};
use crate::model::identifier::NormalizedIdentifier;

// ── ProposalMatchMethod ──────────────────────────────────────────────────────

/// How an entity resolution proposal was generated.
///
/// All variants are **weak** methods (design §7.1): they can only propose, not
/// automatically resolve. Strong exact typed identifiers (email, URL, external
/// ID) resolve directly and do not go through this proposal path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalMatchMethod {
    /// Name similarity match (weak — Unicode case-folded comparison).
    NameSimilarity,
    /// Fuzzy text similarity (weak — e.g. Levenshtein distance, trigram).
    FuzzySimilarity,
    /// Vector/embedding similarity (weak — cosine distance between embeddings).
    VectorSimilarity,
}

// ── ProposalStatus ───────────────────────────────────────────────────────────

/// The lifecycle status of an entity resolution proposal.
///
/// Design §4.2: `CHECK(unresolved/accepted/rejected/reversed)`.
///
/// Status transitions are governed; only the governance layer may advance
/// beyond `Unresolved`. A new proposal is always `Unresolved`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    /// The proposal has not been acted on.
    Unresolved,
    /// A user accepted the proposal (entities are merged).
    Accepted,
    /// A user rejected the proposal (entities remain separate).
    Rejected,
    /// An accepted proposal was reversed (merged entities were split).
    Reversed,
}

// ── ProposalRationale ────────────────────────────────────────────────────────

/// The rationale for an entity resolution proposal.
///
/// Stored as the `rationale_json` column in `entity_resolution_proposals`.
/// All fields are policy-safe (no hidden IDs or private content that is not
/// already visible to the governance actor reading the proposal).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposalRationale {
    /// The method used to generate this proposal.
    pub method: ProposalMatchMethod,
    /// The version of the feature/algorithm that generated this proposal.
    ///
    /// Required: must always be non-empty. Design §7.1 requires `features_version`.
    pub features_version: String,
    /// A similarity score or distance (algorithm-specific).
    ///
    /// Named by the algorithm — never presented as a probability.
    /// `None` when stripped because `score_semantics` was absent (bare score).
    pub similarity_score: Option<f64>,
    /// What the score means (required when `similarity_score` is present).
    ///
    /// `None` when no score is recorded, or when the score was stripped.
    pub score_semantics: Option<String>,
    /// A brief human-readable description of why these entities might match.
    pub description: String,
    /// The normalized left identifier that contributed to this proposal.
    pub left_normalized: Option<NormalizedIdentifier>,
    /// The normalized right identifier that contributed to this proposal.
    pub right_normalized: Option<NormalizedIdentifier>,
}

// ── EntityResolutionProposal ─────────────────────────────────────────────────

/// An unresolved entity resolution proposal.
///
/// Design §4.2: created by conservative resolution when names/fuzzy/vector
/// similarity suggests two entities might be the same. **NO topology mutation**
/// is performed until the user accepts the proposal.
///
/// ## Invariants
/// - `left_entity_id != right_entity_id` (no self-proposals)
/// - `status = Unresolved` at creation; only governance changes it
/// - No entity merge/link is created by the proposal itself
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityResolutionProposal {
    /// Stable proposal identity.
    pub id: String,
    /// The left entity in the pair.
    pub left_entity_id: EntityId,
    /// The right entity in the pair.
    pub right_entity_id: EntityId,
    /// The rationale (serialized as `rationale_json` in the authority store).
    pub rationale: ProposalRationale,
    /// The status (starts `Unresolved`; changes only through governance).
    pub status: ProposalStatus,
    /// The graph revision at which this proposal was created.
    pub base_revision: GraphRevision,
    /// Policy context — the namespace owning this proposal.
    pub policy_namespace: String,
    /// Policy context — the scope within the namespace.
    pub policy_scope: String,
    /// Policy context — sensitivity level (`0..=3`).
    pub policy_sensitivity: u8,
    /// Policy context — the effective policy version tag.
    pub policy_version: String,
}

// ── ProposalError ────────────────────────────────────────────────────────────

/// Errors produced when building an [`EntityResolutionProposal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalError {
    /// Left and right entities are the same — no self-proposals allowed.
    SelfProposal,
    /// Policy sensitivity out of valid range `0..=3`.
    InvalidSensitivity { got: u8, max: u8 },
}

impl std::fmt::Display for ProposalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProposalError::SelfProposal => write!(
                f,
                "entity resolution proposal: left_entity_id and right_entity_id must differ \
                 (self-proposals are not permitted)"
            ),
            ProposalError::InvalidSensitivity { got, max } => write!(
                f,
                "entity resolution proposal: policy_sensitivity {got} is out of range 0..={max}"
            ),
        }
    }
}

impl std::error::Error for ProposalError {}

// ── EntityProposalBuilder ────────────────────────────────────────────────────

/// Stateless builder for [`EntityResolutionProposal`].
pub struct EntityProposalBuilder;

/// Maximum allowed policy sensitivity (mirrors `SENSITIVITY_MAX` in `mod.rs`).
const PROPOSAL_SENSITIVITY_MAX: u8 = 3;

impl EntityProposalBuilder {
    /// Create a new unresolved entity resolution proposal.
    ///
    /// ## Rules
    /// - `left_entity_id` must not equal `right_entity_id`
    ///   → [`ProposalError::SelfProposal`]
    /// - `policy_sensitivity` must be in `0..=3`
    ///   → [`ProposalError::InvalidSensitivity`]
    /// - `status` is always [`ProposalStatus::Unresolved`] at creation
    /// - **NO topology mutation** is performed by this call
    /// - `similarity_score` without `score_semantics` in the rationale → both
    ///   stripped via [`Self::validate_rationale`]
    pub fn create(
        id: String,
        left_entity_id: EntityId,
        right_entity_id: EntityId,
        rationale: ProposalRationale,
        base_revision: GraphRevision,
        policy_namespace: String,
        policy_scope: String,
        policy_sensitivity: u8,
        policy_version: String,
    ) -> Result<EntityResolutionProposal, ProposalError> {
        // Enforce: no self-proposals.
        if left_entity_id == right_entity_id {
            return Err(ProposalError::SelfProposal);
        }

        // Enforce: sensitivity in range.
        if policy_sensitivity > PROPOSAL_SENSITIVITY_MAX {
            return Err(ProposalError::InvalidSensitivity {
                got: policy_sensitivity,
                max: PROPOSAL_SENSITIVITY_MAX,
            });
        }

        // Strip bare scores from the rationale.
        let rationale = Self::validate_rationale(rationale);

        Ok(EntityResolutionProposal {
            id,
            left_entity_id,
            right_entity_id,
            rationale,
            // Status is ALWAYS Unresolved at creation — invariant enforced here.
            status: ProposalStatus::Unresolved,
            base_revision,
            policy_namespace,
            policy_scope,
            policy_sensitivity,
            policy_version,
        })
    }

    /// Validate that a rationale's score fields are consistent.
    ///
    /// Returns a clean rationale where bare scores are stripped:
    /// - `similarity_score` present, `score_semantics` absent → both set to `None`.
    /// - `score_semantics` present, `similarity_score` absent → `score_semantics` set to `None`
    ///   (no phantom annotation without a corresponding score value).
    /// - Both present → kept as-is.
    /// - Neither present → no-op.
    pub fn validate_rationale(mut rationale: ProposalRationale) -> ProposalRationale {
        match (
            rationale.similarity_score,
            rationale.score_semantics.as_ref(),
        ) {
            // Both present → keep.
            (Some(_), Some(_)) => {}
            // Score without semantics → bare score, strip both.
            (Some(_), None) => {
                rationale.similarity_score = None;
                rationale.score_semantics = None;
            }
            // Semantics without score → no score annotation, discard semantics.
            (None, Some(_)) => {
                rationale.score_semantics = None;
            }
            // Neither → nothing to do.
            (None, None) => {}
        }
        rationale
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_rationale(score: Option<f64>, semantics: Option<&str>) -> ProposalRationale {
        ProposalRationale {
            method: ProposalMatchMethod::NameSimilarity,
            features_version: "name-sim-v1".into(),
            similarity_score: score,
            score_semantics: semantics.map(Into::into),
            description: "Names are similar after Unicode case-folding".into(),
            left_normalized: None,
            right_normalized: None,
        }
    }

    fn distinct_ids() -> (EntityId, EntityId) {
        (EntityId::new_v7(), EntityId::new_v7())
    }

    fn build(
        left: EntityId,
        right: EntityId,
        rationale: ProposalRationale,
    ) -> Result<EntityResolutionProposal, ProposalError> {
        EntityProposalBuilder::create(
            EntityId::new_v7().into_string(), // use an entity-id-shaped string as proposal id
            left,
            right,
            rationale,
            GraphRevision::new(42),
            "user".into(),
            "chat".into(),
            0,
            "policy-v1".into(),
        )
    }

    // ── create: success case ─────────────────────────────────────────────────

    #[test]
    fn create_success_returns_unresolved_proposal() {
        let (left, right) = distinct_ids();
        let rationale = make_rationale(Some(0.87), Some("unicode_casefold_similarity"));

        let proposal = build(left.clone(), right.clone(), rationale).unwrap();

        assert_eq!(proposal.left_entity_id, left);
        assert_eq!(proposal.right_entity_id, right);
        assert_eq!(proposal.status, ProposalStatus::Unresolved);
        assert_eq!(proposal.base_revision, GraphRevision::new(42));
        assert_eq!(proposal.policy_namespace, "user");
        assert_eq!(proposal.policy_scope, "chat");
        assert_eq!(proposal.policy_sensitivity, 0);
        assert_eq!(proposal.policy_version, "policy-v1");
        // Score and semantics both present → kept.
        assert_eq!(proposal.rationale.similarity_score, Some(0.87));
        assert_eq!(
            proposal.rationale.score_semantics.as_deref(),
            Some("unicode_casefold_similarity")
        );
    }

    // ── create: SelfProposal error ───────────────────────────────────────────

    #[test]
    fn create_self_proposal_returns_error() {
        let id = EntityId::new_v7();
        let rationale = make_rationale(None, None);

        let err = EntityProposalBuilder::create(
            "some-id".into(),
            id.clone(),
            id.clone(), // same as left — should fail
            rationale,
            GraphRevision::base(),
            "user".into(),
            "chat".into(),
            0,
            "v1".into(),
        )
        .unwrap_err();

        assert_eq!(err, ProposalError::SelfProposal);
        assert!(err.to_string().contains("self-proposals are not permitted"));
    }

    // ── create: status always Unresolved ────────────────────────────────────

    #[test]
    fn create_status_is_always_unresolved() {
        let (left, right) = distinct_ids();
        let rationale = make_rationale(None, None);

        // Even if a caller tried to set a different status, the builder forces Unresolved.
        // (There's no way to pass a status — the builder always uses Unresolved.)
        let proposal = build(left, right, rationale).unwrap();
        assert_eq!(
            proposal.status,
            ProposalStatus::Unresolved,
            "status must be Unresolved at creation regardless of inputs"
        );
    }

    // ── create: invalid sensitivity ─────────────────────────────────────────

    #[test]
    fn create_invalid_sensitivity_returns_error() {
        let (left, right) = distinct_ids();
        let rationale = make_rationale(None, None);

        let err = EntityProposalBuilder::create(
            "some-id".into(),
            left,
            right,
            rationale,
            GraphRevision::base(),
            "user".into(),
            "chat".into(),
            4, // invalid: max is 3
            "v1".into(),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ProposalError::InvalidSensitivity { got: 4, max: 3 }
        ));
        assert!(err.to_string().contains("out of range 0..=3"));
    }

    // ── validate_rationale: bare score stripped ──────────────────────────────

    #[test]
    fn validate_rationale_bare_score_stripped() {
        let rationale = make_rationale(Some(0.75), None); // score without semantics
        let clean = EntityProposalBuilder::validate_rationale(rationale);

        assert_eq!(clean.similarity_score, None, "bare score must be stripped");
        assert_eq!(clean.score_semantics, None);
        // Other fields preserved.
        assert_eq!(clean.features_version, "name-sim-v1");
        assert_eq!(clean.method, ProposalMatchMethod::NameSimilarity);
    }

    // ── validate_rationale: score + semantics kept ───────────────────────────

    #[test]
    fn validate_rationale_score_and_semantics_kept() {
        let rationale = make_rationale(Some(0.92), Some("cosine_distance"));
        let clean = EntityProposalBuilder::validate_rationale(rationale);

        assert_eq!(clean.similarity_score, Some(0.92));
        assert_eq!(clean.score_semantics.as_deref(), Some("cosine_distance"));
    }

    // ── validate_rationale: semantics without score discarded ────────────────

    #[test]
    fn validate_rationale_semantics_without_score_discarded() {
        let rationale = make_rationale(None, Some("some_metric")); // semantics without score
        let clean = EntityProposalBuilder::validate_rationale(rationale);

        assert_eq!(clean.similarity_score, None);
        assert_eq!(
            clean.score_semantics, None,
            "semantics without score must be discarded"
        );
    }

    // ── validate_rationale: neither score nor semantics ──────────────────────

    #[test]
    fn validate_rationale_neither_is_noop() {
        let rationale = make_rationale(None, None);
        let clean = EntityProposalBuilder::validate_rationale(rationale);
        assert_eq!(clean.similarity_score, None);
        assert_eq!(clean.score_semantics, None);
    }

    // ── ProposalStatus serde roundtrip ───────────────────────────────────────

    #[test]
    fn proposal_status_serde_roundtrip_all_variants() {
        let variants = [
            ProposalStatus::Unresolved,
            ProposalStatus::Accepted,
            ProposalStatus::Rejected,
            ProposalStatus::Reversed,
        ];
        for status in &variants {
            let json = serde_json::to_string(status).unwrap();
            let back: ProposalStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, status, "serde roundtrip failed for {status:?}");
        }
        // Verify expected snake_case text forms.
        assert_eq!(
            serde_json::to_string(&ProposalStatus::Unresolved).unwrap(),
            "\"unresolved\""
        );
        assert_eq!(
            serde_json::to_string(&ProposalStatus::Accepted).unwrap(),
            "\"accepted\""
        );
        assert_eq!(
            serde_json::to_string(&ProposalStatus::Rejected).unwrap(),
            "\"rejected\""
        );
        assert_eq!(
            serde_json::to_string(&ProposalStatus::Reversed).unwrap(),
            "\"reversed\""
        );
    }

    // ── ProposalMatchMethod serde roundtrip ──────────────────────────────────

    #[test]
    fn proposal_match_method_serde_roundtrip_all_variants() {
        let variants = [
            ProposalMatchMethod::NameSimilarity,
            ProposalMatchMethod::FuzzySimilarity,
            ProposalMatchMethod::VectorSimilarity,
        ];
        for method in &variants {
            let json = serde_json::to_string(method).unwrap();
            let back: ProposalMatchMethod = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, method, "serde roundtrip failed for {method:?}");
        }
        // Verify expected snake_case text forms.
        assert_eq!(
            serde_json::to_string(&ProposalMatchMethod::NameSimilarity).unwrap(),
            "\"name_similarity\""
        );
        assert_eq!(
            serde_json::to_string(&ProposalMatchMethod::FuzzySimilarity).unwrap(),
            "\"fuzzy_similarity\""
        );
        assert_eq!(
            serde_json::to_string(&ProposalMatchMethod::VectorSimilarity).unwrap(),
            "\"vector_similarity\""
        );
    }

    // ── EntityResolutionProposal serde roundtrip ─────────────────────────────

    #[test]
    fn proposal_serde_roundtrip() {
        let (left, right) = distinct_ids();
        let rationale = make_rationale(Some(0.88), Some("trigram_score"));
        let proposal = build(left, right, rationale).unwrap();

        let json = serde_json::to_string(&proposal).unwrap();
        let back: EntityResolutionProposal = serde_json::from_str(&json).unwrap();

        assert_eq!(back.id, proposal.id);
        assert_eq!(back.left_entity_id, proposal.left_entity_id);
        assert_eq!(back.right_entity_id, proposal.right_entity_id);
        assert_eq!(back.status, proposal.status);
        assert_eq!(back.base_revision, proposal.base_revision);
        assert_eq!(back.policy_namespace, proposal.policy_namespace);
        assert_eq!(
            back.rationale.similarity_score,
            proposal.rationale.similarity_score
        );
        assert_eq!(
            back.rationale.score_semantics,
            proposal.rationale.score_semantics
        );
        assert_eq!(
            back.rationale.features_version,
            proposal.rationale.features_version
        );
    }

    // ── features_version is preserved ────────────────────────────────────────

    #[test]
    fn create_preserves_features_version_in_rationale() {
        let (left, right) = distinct_ids();
        let mut rationale = make_rationale(None, None);
        rationale.features_version = "fuzzy-levenshtein-v2.3.1".into();

        let proposal = build(left, right, rationale).unwrap();

        assert_eq!(
            proposal.rationale.features_version,
            "fuzzy-levenshtein-v2.3.1"
        );
    }

    // ── vector similarity method ──────────────────────────────────────────────

    #[test]
    fn create_vector_similarity_proposal() {
        let (left, right) = distinct_ids();
        let rationale = ProposalRationale {
            method: ProposalMatchMethod::VectorSimilarity,
            features_version: "minilm-v1".into(),
            similarity_score: Some(0.95),
            score_semantics: Some("cosine_similarity".into()),
            description: "High cosine similarity between entity embeddings".into(),
            left_normalized: None,
            right_normalized: None,
        };

        let proposal = EntityProposalBuilder::create(
            EntityId::new_v7().into_string(),
            left,
            right,
            rationale,
            GraphRevision::new(7),
            "user".into(),
            "knowledge".into(),
            1,
            "policy-v2".into(),
        )
        .unwrap();

        assert_eq!(proposal.status, ProposalStatus::Unresolved);
        assert_eq!(
            proposal.rationale.method,
            ProposalMatchMethod::VectorSimilarity
        );
        assert_eq!(proposal.rationale.features_version, "minilm-v1");
        assert_eq!(proposal.rationale.similarity_score, Some(0.95));
        assert_eq!(
            proposal.rationale.score_semantics.as_deref(),
            Some("cosine_similarity")
        );
        assert_eq!(proposal.policy_sensitivity, 1);
    }
}
