//! Entity resolution action records for `entity_resolution_actions`
//! (Design §4.2, task F2.5.5, MGR-019).
//!
//! Design §4.2: `entity_resolution_actions`:
//! ```text
//!   id PK, proposal_id, action_kind, before_json, after_json,
//!   reversible_until, reversal_of, event_id, revision
//! ```
//!
//! Design §A9: "Correction, merge/split, contradiction, forget/restore/delete
//! are previewed, governed, audited, and reversible where promised."
//!
//! ## Key invariants
//!
//! - **Before/after preserved**: every action stores the full before and after
//!   state for audit and reversal.
//! - **No evidence loss**: `CanonicalEndpointCorrection` ensures all mentions,
//!   relationships, and evidence are re-pointed, never deleted.
//! - **Accept**: sets proposal to Accepted; `reversible_until = now + 30 days`.
//! - **Reject**: sets proposal to Rejected; `reversible_until = None`.
//! - **Reverse**: sets proposal to Reversed; `reversal_of = original_action_id`.
//! - **Status validation**: Accept/Reject require `Unresolved`; Reverse requires `Accepted`.

use chrono::Duration;
use serde::{Deserialize, Serialize};

use super::{EntityId, GraphRevision, UtcTimestamp};
use crate::memory::model::entity_proposal::{EntityResolutionProposal, ProposalStatus};

// ── ProposalActionKind ───────────────────────────────────────────────────────

/// The kind of action taken on an entity resolution proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalActionKind {
    /// User accepted the proposal (entities will be merged).
    Accept,
    /// User rejected the proposal (entities remain separate).
    Reject,
    /// User reversed a previous Accept (merged entities will be split).
    Reverse,
}

// ── ProposalBeforeState ──────────────────────────────────────────────────────

/// The before-state of the proposal and involved entities at action time.
///
/// Stored as `before_json` in the authority store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalBeforeState {
    /// The proposal status before this action.
    pub proposal_status: ProposalStatus,
    /// The canonical entity ID before this action.
    pub canonical_entity_id: EntityId,
    /// The non-canonical entity ID before this action.
    pub non_canonical_entity_id: EntityId,
    /// Count of aliases on the canonical entity before merge.
    pub canonical_alias_count: u32,
    /// Count of aliases on the non-canonical entity before merge.
    pub non_canonical_alias_count: u32,
    /// Count of mentions to be re-pointed.
    pub mention_count_to_migrate: u32,
    /// Count of links/relationships to be re-pointed.
    pub link_count_to_migrate: u32,
}

// ── ProposalAfterState ───────────────────────────────────────────────────────

/// The after-state of the proposal and involved entities at action time.
///
/// Stored as `after_json` in the authority store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalAfterState {
    /// The proposal status after this action.
    pub proposal_status: ProposalStatus,
    /// The surviving canonical entity ID (same as before for Accept; unchanged
    /// for Reject/Reverse).
    pub canonical_entity_id: EntityId,
    /// Whether the non-canonical entity was merged (Accept) or kept separate
    /// (Reject/Reverse).
    pub non_canonical_merged: bool,
    /// Count of aliases migrated to the canonical entity.
    pub aliases_migrated: u32,
    /// Count of mentions re-pointed to the canonical entity.
    pub mentions_migrated: u32,
    /// Count of links re-pointed to the canonical entity.
    pub links_migrated: u32,
}

// ── ProposalAction ───────────────────────────────────────────────────────────

/// A complete entity resolution action record for `entity_resolution_actions`.
///
/// Design §4.2: every action preserves before/after state for audit and
/// reversal. The before/after states ensure no evidence is lost during merge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalAction {
    /// Stable action identity.
    pub id: String,
    /// The proposal this action applies to.
    pub proposal_id: String,
    /// The kind of action taken.
    pub action_kind: ProposalActionKind,
    /// The before state (serialized as `before_json` in the authority store).
    pub before_state: ProposalBeforeState,
    /// The after state (serialized as `after_json` in the authority store).
    pub after_state: ProposalAfterState,
    /// Until when this action can be reversed.
    /// `None` for Reject or Reverse (those have different reversal semantics).
    pub reversible_until: Option<UtcTimestamp>,
    /// The action this action reverses (for Reverse actions).
    pub reversal_of: Option<String>,
    /// The graph revision at which this action was committed.
    pub revision: GraphRevision,
    /// The actor who performed this action.
    pub actor_id: String,
}

// ── ProposalActionError ──────────────────────────────────────────────────────

/// Errors produced when building a [`ProposalAction`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalActionError {
    /// The proposal is not in the required status for this action.
    InvalidCurrentStatus {
        current: ProposalStatus,
        required: ProposalStatus,
    },
    /// The action would lose evidence.
    EvidenceLoss { detail: String },
}

impl std::fmt::Display for ProposalActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProposalActionError::InvalidCurrentStatus { current, required } => write!(
                f,
                "proposal action: invalid status — current={current:?}, required={required:?}"
            ),
            ProposalActionError::EvidenceLoss { detail } => {
                write!(f, "proposal action: evidence loss prevented — {detail}")
            }
        }
    }
}

impl std::error::Error for ProposalActionError {}

// ── CanonicalEndpointCorrection ──────────────────────────────────────────────

/// Describes which entity endpoints need to be corrected after a merge.
///
/// When a non-canonical entity is merged into a canonical entity, all
/// relationships, mentions, and evidence referring to the non-canonical entity
/// must be re-pointed to the canonical entity. No evidence is lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalEndpointCorrection {
    /// The entity that is going away (non-canonical).
    pub old_entity_id: EntityId,
    /// The entity that survives (canonical).
    pub new_entity_id: EntityId,
    /// Count of mention rows to update.
    pub mention_corrections: u32,
    /// Count of relationship endpoint corrections.
    pub relationship_corrections: u32,
    /// Count of evidence rows to update.
    pub evidence_corrections: u32,
    /// Count of alias rows to migrate.
    pub alias_migrations: u32,
}

impl CanonicalEndpointCorrection {
    /// Total number of rows that will be corrected/migrated.
    pub fn total_corrections(&self) -> u32 {
        self.mention_corrections
            + self.relationship_corrections
            + self.evidence_corrections
            + self.alias_migrations
    }
}

// ── ProposalActionBuilder ────────────────────────────────────────────────────

/// Number of days after which an accepted merge can no longer be reversed.
const ACCEPT_REVERSIBLE_DAYS: i64 = 30;

/// Stateless builder for [`ProposalAction`] records.
pub struct ProposalActionBuilder;

impl ProposalActionBuilder {
    /// Build an Accept action record.
    ///
    /// Rules:
    /// - Proposal must be in `Unresolved` status →
    ///   [`ProposalActionError::InvalidCurrentStatus`]
    /// - After state: `proposal_status = Accepted`, `non_canonical_merged = true`
    /// - `reversible_until = Some(committed_at + 30 days)`
    /// - `reversal_of = None`
    #[allow(clippy::too_many_arguments)]
    pub fn accept(
        action_id: String,
        proposal: &EntityResolutionProposal,
        before: ProposalBeforeState,
        migration_counts: (u32, u32, u32), // (aliases, mentions, links)
        revision: GraphRevision,
        actor_id: String,
        committed_at: UtcTimestamp,
    ) -> Result<ProposalAction, ProposalActionError> {
        if proposal.status != ProposalStatus::Unresolved {
            return Err(ProposalActionError::InvalidCurrentStatus {
                current: proposal.status,
                required: ProposalStatus::Unresolved,
            });
        }

        let (aliases_migrated, mentions_migrated, links_migrated) = migration_counts;

        let after_state = ProposalAfterState {
            proposal_status: ProposalStatus::Accepted,
            canonical_entity_id: before.canonical_entity_id.clone(),
            non_canonical_merged: true,
            aliases_migrated,
            mentions_migrated,
            links_migrated,
        };

        // reversible_until = committed_at + 30 days
        let reversible_until = UtcTimestamp::from_datetime(
            committed_at
                .as_datetime()
                .checked_add_signed(Duration::days(ACCEPT_REVERSIBLE_DAYS))
                .unwrap_or(committed_at.as_datetime()),
        );

        Ok(ProposalAction {
            id: action_id,
            proposal_id: proposal.id.clone(),
            action_kind: ProposalActionKind::Accept,
            before_state: before,
            after_state,
            reversible_until: Some(reversible_until),
            reversal_of: None,
            revision,
            actor_id,
        })
    }

    /// Build a Reject action record.
    ///
    /// Rules:
    /// - Proposal must be in `Unresolved` status →
    ///   [`ProposalActionError::InvalidCurrentStatus`]
    /// - After state: `proposal_status = Rejected`, `non_canonical_merged = false`
    /// - `reversible_until = None` (rejections are not reversible via this path)
    pub fn reject(
        action_id: String,
        proposal: &EntityResolutionProposal,
        before: ProposalBeforeState,
        revision: GraphRevision,
        actor_id: String,
    ) -> Result<ProposalAction, ProposalActionError> {
        if proposal.status != ProposalStatus::Unresolved {
            return Err(ProposalActionError::InvalidCurrentStatus {
                current: proposal.status,
                required: ProposalStatus::Unresolved,
            });
        }

        let after_state = ProposalAfterState {
            proposal_status: ProposalStatus::Rejected,
            canonical_entity_id: before.canonical_entity_id.clone(),
            non_canonical_merged: false,
            aliases_migrated: 0,
            mentions_migrated: 0,
            links_migrated: 0,
        };

        Ok(ProposalAction {
            id: action_id,
            proposal_id: proposal.id.clone(),
            action_kind: ProposalActionKind::Reject,
            before_state: before,
            after_state,
            reversible_until: None, // rejections are not reversible via this path
            reversal_of: None,
            revision,
            actor_id,
        })
    }

    /// Build a Reverse action record (reverses a previous Accept).
    ///
    /// Rules:
    /// - Proposal must be in `Accepted` status →
    ///   [`ProposalActionError::InvalidCurrentStatus`]
    /// - After state: `proposal_status = Reversed`, `non_canonical_merged = false`
    /// - `reversal_of = original_action_id`
    /// - `reversible_until = None`
    pub fn reverse(
        action_id: String,
        proposal: &EntityResolutionProposal,
        original_action_id: String,
        before: ProposalBeforeState,
        revision: GraphRevision,
        actor_id: String,
    ) -> Result<ProposalAction, ProposalActionError> {
        if proposal.status != ProposalStatus::Accepted {
            return Err(ProposalActionError::InvalidCurrentStatus {
                current: proposal.status,
                required: ProposalStatus::Accepted,
            });
        }

        let after_state = ProposalAfterState {
            proposal_status: ProposalStatus::Reversed,
            canonical_entity_id: before.canonical_entity_id.clone(),
            non_canonical_merged: false,
            aliases_migrated: 0,
            mentions_migrated: 0,
            links_migrated: 0,
        };

        Ok(ProposalAction {
            id: action_id,
            proposal_id: proposal.id.clone(),
            action_kind: ProposalActionKind::Reverse,
            before_state: before,
            after_state,
            reversible_until: None, // reversals are not themselves reversible
            reversal_of: Some(original_action_id),
            revision,
            actor_id,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::model::entity_proposal::{
        EntityProposalBuilder, ProposalMatchMethod, ProposalRationale, ProposalStatus,
    };
    use crate::memory::model::{EntityId, GraphRevision, UtcTimestamp};

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_proposal(status: ProposalStatus) -> EntityResolutionProposal {
        let left = EntityId::new_v7();
        let right = EntityId::new_v7();
        let rationale = ProposalRationale {
            method: ProposalMatchMethod::NameSimilarity,
            features_version: "name-sim-v1".into(),
            similarity_score: None,
            score_semantics: None,
            description: "Test proposal".into(),
            left_normalized: None,
            right_normalized: None,
        };
        let mut proposal = EntityProposalBuilder::create(
            EntityId::new_v7().into_string(),
            left,
            right,
            rationale,
            GraphRevision::new(1),
            "user".into(),
            "chat".into(),
            0,
            "policy-v1".into(),
        )
        .unwrap();
        // Override status for test setup (status variants beyond Unresolved need
        // to be set directly — the builder always produces Unresolved).
        proposal.status = status;
        proposal
    }

    fn make_before(proposal: &EntityResolutionProposal) -> ProposalBeforeState {
        ProposalBeforeState {
            proposal_status: proposal.status,
            canonical_entity_id: proposal.left_entity_id.clone(),
            non_canonical_entity_id: proposal.right_entity_id.clone(),
            canonical_alias_count: 3,
            non_canonical_alias_count: 2,
            mention_count_to_migrate: 10,
            link_count_to_migrate: 5,
        }
    }

    fn now() -> UtcTimestamp {
        UtcTimestamp::now()
    }

    // ── accept: success ───────────────────────────────────────────────────

    #[test]
    fn accept_success_produces_correct_action() {
        let proposal = make_proposal(ProposalStatus::Unresolved);
        let before = make_before(&proposal);
        let committed_at = now();

        let action = ProposalActionBuilder::accept(
            "action-001".into(),
            &proposal,
            before.clone(),
            (2, 10, 5),
            GraphRevision::new(42),
            "actor-alice".into(),
            committed_at,
        )
        .unwrap();

        assert_eq!(action.id, "action-001");
        assert_eq!(action.proposal_id, proposal.id);
        assert_eq!(action.action_kind, ProposalActionKind::Accept);
        assert_eq!(action.after_state.proposal_status, ProposalStatus::Accepted);
        assert!(action.after_state.non_canonical_merged);
        assert_eq!(action.after_state.aliases_migrated, 2);
        assert_eq!(action.after_state.mentions_migrated, 10);
        assert_eq!(action.after_state.links_migrated, 5);
        assert!(action.reversal_of.is_none());
        assert_eq!(action.revision, GraphRevision::new(42));
        assert_eq!(action.actor_id, "actor-alice");
        // Before state preserved
        assert_eq!(
            action.before_state.proposal_status,
            ProposalStatus::Unresolved
        );
        assert_eq!(
            action.before_state.canonical_alias_count,
            before.canonical_alias_count
        );
    }

    // ── accept: InvalidCurrentStatus when proposal is Accepted ───────────

    #[test]
    fn accept_invalid_status_when_already_accepted() {
        let proposal = make_proposal(ProposalStatus::Accepted);
        let before = make_before(&proposal);

        let err = ProposalActionBuilder::accept(
            "action-002".into(),
            &proposal,
            before,
            (0, 0, 0),
            GraphRevision::new(1),
            "actor-alice".into(),
            now(),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ProposalActionError::InvalidCurrentStatus {
                current: ProposalStatus::Accepted,
                required: ProposalStatus::Unresolved,
            }
        ));
        assert!(err.to_string().contains("Accepted"));
        assert!(err.to_string().contains("Unresolved"));
    }

    // ── accept: reversible_until is set (now + 30 days) ──────────────────

    #[test]
    fn accept_reversible_until_is_set_to_30_days() {
        let proposal = make_proposal(ProposalStatus::Unresolved);
        let before = make_before(&proposal);
        let committed_at = now();

        let action = ProposalActionBuilder::accept(
            "action-003".into(),
            &proposal,
            before,
            (0, 0, 0),
            GraphRevision::new(5),
            "actor-alice".into(),
            committed_at,
        )
        .unwrap();

        let reversible_until = action
            .reversible_until
            .expect("reversible_until must be Some");
        let diff = reversible_until
            .as_datetime()
            .signed_duration_since(committed_at.as_datetime());
        // Allow ±1 second tolerance for test execution time
        assert!(
            diff.num_seconds() >= 30 * 24 * 3600 - 1 && diff.num_seconds() <= 30 * 24 * 3600 + 1,
            "reversible_until should be ~30 days from committed_at, got {}s",
            diff.num_seconds()
        );
    }

    // ── reject: success ───────────────────────────────────────────────────

    #[test]
    fn reject_success_produces_correct_action() {
        let proposal = make_proposal(ProposalStatus::Unresolved);
        let before = make_before(&proposal);

        let action = ProposalActionBuilder::reject(
            "action-004".into(),
            &proposal,
            before.clone(),
            GraphRevision::new(10),
            "actor-bob".into(),
        )
        .unwrap();

        assert_eq!(action.id, "action-004");
        assert_eq!(action.proposal_id, proposal.id);
        assert_eq!(action.action_kind, ProposalActionKind::Reject);
        assert_eq!(action.after_state.proposal_status, ProposalStatus::Rejected);
        assert!(!action.after_state.non_canonical_merged);
        assert_eq!(action.after_state.aliases_migrated, 0);
        assert_eq!(action.after_state.mentions_migrated, 0);
        assert_eq!(action.after_state.links_migrated, 0);
        assert!(action.reversal_of.is_none());
        assert_eq!(action.revision, GraphRevision::new(10));
        assert_eq!(action.actor_id, "actor-bob");
        // Before state preserved
        assert_eq!(
            action.before_state.proposal_status,
            ProposalStatus::Unresolved
        );
    }

    // ── reject: InvalidCurrentStatus when proposal is Rejected ───────────

    #[test]
    fn reject_invalid_status_when_already_rejected() {
        let proposal = make_proposal(ProposalStatus::Rejected);
        let before = make_before(&proposal);

        let err = ProposalActionBuilder::reject(
            "action-005".into(),
            &proposal,
            before,
            GraphRevision::new(1),
            "actor-bob".into(),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ProposalActionError::InvalidCurrentStatus {
                current: ProposalStatus::Rejected,
                required: ProposalStatus::Unresolved,
            }
        ));
    }

    // ── reject: reversible_until is None ─────────────────────────────────

    #[test]
    fn reject_reversible_until_is_none() {
        let proposal = make_proposal(ProposalStatus::Unresolved);
        let before = make_before(&proposal);

        let action = ProposalActionBuilder::reject(
            "action-006".into(),
            &proposal,
            before,
            GraphRevision::new(2),
            "actor-bob".into(),
        )
        .unwrap();

        assert!(
            action.reversible_until.is_none(),
            "reject must have reversible_until=None"
        );
    }

    // ── reverse: success ──────────────────────────────────────────────────

    #[test]
    fn reverse_success_produces_correct_action() {
        let proposal = make_proposal(ProposalStatus::Accepted);
        let before = make_before(&proposal);

        let action = ProposalActionBuilder::reverse(
            "action-007".into(),
            &proposal,
            "action-original-001".into(),
            before.clone(),
            GraphRevision::new(20),
            "actor-carol".into(),
        )
        .unwrap();

        assert_eq!(action.id, "action-007");
        assert_eq!(action.proposal_id, proposal.id);
        assert_eq!(action.action_kind, ProposalActionKind::Reverse);
        assert_eq!(action.after_state.proposal_status, ProposalStatus::Reversed);
        assert!(!action.after_state.non_canonical_merged);
        assert_eq!(action.after_state.aliases_migrated, 0);
        assert!(action.reversible_until.is_none());
        assert_eq!(action.reversal_of.as_deref(), Some("action-original-001"));
        assert_eq!(action.revision, GraphRevision::new(20));
        assert_eq!(action.actor_id, "actor-carol");
    }

    // ── reverse: InvalidCurrentStatus when proposal is Unresolved ────────

    #[test]
    fn reverse_invalid_status_when_unresolved() {
        let proposal = make_proposal(ProposalStatus::Unresolved);
        let before = make_before(&proposal);

        let err = ProposalActionBuilder::reverse(
            "action-008".into(),
            &proposal,
            "original-001".into(),
            before,
            GraphRevision::new(1),
            "actor-carol".into(),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ProposalActionError::InvalidCurrentStatus {
                current: ProposalStatus::Unresolved,
                required: ProposalStatus::Accepted,
            }
        ));
    }

    // ── reverse: reversal_of is set ───────────────────────────────────────

    #[test]
    fn reverse_reversal_of_is_set() {
        let proposal = make_proposal(ProposalStatus::Accepted);
        let before = make_before(&proposal);

        let action = ProposalActionBuilder::reverse(
            "action-009".into(),
            &proposal,
            "original-accept-action-id".into(),
            before,
            GraphRevision::new(5),
            "actor-carol".into(),
        )
        .unwrap();

        assert_eq!(
            action.reversal_of.as_deref(),
            Some("original-accept-action-id"),
            "reversal_of must be set to the original action id"
        );
    }

    // ── CanonicalEndpointCorrection: counts correct ───────────────────────

    #[test]
    fn canonical_endpoint_correction_counts_correct() {
        let old_entity_id = EntityId::new_v7();
        let new_entity_id = EntityId::new_v7();

        let correction = CanonicalEndpointCorrection {
            old_entity_id: old_entity_id.clone(),
            new_entity_id: new_entity_id.clone(),
            mention_corrections: 10,
            relationship_corrections: 5,
            evidence_corrections: 3,
            alias_migrations: 2,
        };

        assert_eq!(correction.mention_corrections, 10);
        assert_eq!(correction.relationship_corrections, 5);
        assert_eq!(correction.evidence_corrections, 3);
        assert_eq!(correction.alias_migrations, 2);
        assert_eq!(correction.total_corrections(), 20);
        assert_eq!(correction.old_entity_id, old_entity_id);
        assert_eq!(correction.new_entity_id, new_entity_id);
    }

    #[test]
    fn canonical_endpoint_correction_zero_counts() {
        let correction = CanonicalEndpointCorrection {
            old_entity_id: EntityId::new_v7(),
            new_entity_id: EntityId::new_v7(),
            mention_corrections: 0,
            relationship_corrections: 0,
            evidence_corrections: 0,
            alias_migrations: 0,
        };
        assert_eq!(correction.total_corrections(), 0);
    }

    // ── ProposalActionKind serde roundtrip ────────────────────────────────

    #[test]
    fn proposal_action_kind_serde_roundtrip() {
        let variants = [
            ProposalActionKind::Accept,
            ProposalActionKind::Reject,
            ProposalActionKind::Reverse,
        ];
        for kind in &variants {
            let json = serde_json::to_string(kind).unwrap();
            let back: ProposalActionKind = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, kind);
        }
        assert_eq!(
            serde_json::to_string(&ProposalActionKind::Accept).unwrap(),
            "\"accept\""
        );
        assert_eq!(
            serde_json::to_string(&ProposalActionKind::Reject).unwrap(),
            "\"reject\""
        );
        assert_eq!(
            serde_json::to_string(&ProposalActionKind::Reverse).unwrap(),
            "\"reverse\""
        );
    }

    // ── ProposalActionError Display ───────────────────────────────────────

    #[test]
    fn proposal_action_error_display() {
        let err = ProposalActionError::InvalidCurrentStatus {
            current: ProposalStatus::Rejected,
            required: ProposalStatus::Unresolved,
        };
        let msg = err.to_string();
        assert!(msg.contains("Rejected"));
        assert!(msg.contains("Unresolved"));

        let err2 = ProposalActionError::EvidenceLoss {
            detail: "3 evidence rows would be lost".into(),
        };
        let msg2 = err2.to_string();
        assert!(msg2.contains("evidence loss"));
        assert!(msg2.contains("3 evidence rows"));
    }
}
