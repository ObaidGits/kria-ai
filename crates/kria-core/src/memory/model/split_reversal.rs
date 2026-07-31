//! Split/reversal reconstruction for conservative entity resolution
//! (Design §A9, MGR-019, task F2.5.6).
//!
//! Design §A9: "Correction, merge/split, contradiction, forget/restore/delete
//! are previewed, governed, audited, and reversible where promised."
//!
//! MGR-019: "Implement split/reversal reconstruction preserving exact
//! memberships/links/history."
//!
//! ## Key invariants
//!
//! - Only `Accept` actions can be reversed; Reject/Reverse actions return
//!   [`SplitReconstructionError::NotAnAcceptAction`].
//! - `can_proceed = !unresolvable_items.iter().any(|i| i.blocks_split)`.
//! - `total_items = resolvable_items.len() as u32`.
//! - Revision drift is detected when `current > revision_at_merge + max_allowed`.
//! - Alias conflicts are detected by case-insensitive (Unicode lower) intersection.
//! - Mention provenance is always appended — never deduplicated during reconstruction.

use serde::{Deserialize, Serialize};

use super::{EntityId, GraphRevision};
use crate::memory::model::proposal_action::{ProposalAction, ProposalActionKind};

// ── LinkEndpointKind ──────────────────────────────────────────────────────

/// Which endpoint of a relationship needs to be corrected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkEndpointKind {
    /// The source endpoint of the relationship.
    Source,
    /// The target endpoint of the relationship.
    Target,
}

// ── SplitReconstructionItem ───────────────────────────────────────────────

/// An item that needs to be reconstructed during a split.
///
/// Each variant captures what was migrated during the original merge and
/// must now be returned to the non-canonical entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitReconstructionItem {
    /// An alias that was migrated from the non-canonical to the canonical
    /// entity and must be returned to the non-canonical entity.
    AliasReturn {
        alias_id: String,
        from_entity_id: EntityId,
        to_entity_id: EntityId,
    },
    /// A mention that was re-pointed from the non-canonical to the canonical
    /// entity and must be re-pointed back.
    MentionReturn {
        mention_id: String,
        from_entity_id: EntityId,
        to_entity_id: EntityId,
    },
    /// A link/relationship endpoint that must be corrected back.
    LinkEndpointReturn {
        relationship_id: String,
        endpoint_kind: LinkEndpointKind,
        from_entity_id: EntityId,
        to_entity_id: EntityId,
    },
    /// Evidence that was re-pointed and must be corrected back.
    EvidenceReturn {
        evidence_id: String,
        from_entity_id: EntityId,
        to_entity_id: EntityId,
    },
}

// ── UnresolvableReason ────────────────────────────────────────────────────

/// Why an item cannot be reconstructed during a split.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvableReason {
    /// The alias was deleted after the merge.
    AliasDeletedAfterMerge,
    /// The mention was deleted after the merge.
    MentionDeletedAfterMerge,
    /// The relationship was deleted after the merge.
    RelationshipDeletedAfterMerge,
    /// A conflicting alias was added to the canonical entity after the merge
    /// (duplicate alias in multi-scope scenario).
    DuplicateAliasAdded,
    /// The link direction was changed after the merge.
    LinkDirectionChanged,
    /// A superseded record complicates reconstruction.
    SupersededRecordConflict,
    /// Concurrent revision drift — another write happened concurrently.
    ConcurrentRevisionDrift {
        revision_at_merge: GraphRevision,
        current_revision: GraphRevision,
    },
}

// ── UnresolvableItem ──────────────────────────────────────────────────────

/// An item that cannot be reconstructed due to concurrent drift or post-merge
/// mutations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvableItem {
    /// Description of what cannot be reconstructed.
    pub description: String,
    /// The reason the item cannot be reconstructed.
    pub reason: UnresolvableReason,
    /// Whether this item blocks the split (P0 blocker).
    pub blocks_split: bool,
}

// ── SplitReconstructionPlan ───────────────────────────────────────────────

/// The full plan for undoing a merge (splitting merged entities back apart).
///
/// Generated from the `before_state` of the accepted `ProposalAction` plus
/// validation of current state. If the current state has drifted (concurrent
/// revision drift), the plan records unresolvable items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitReconstructionPlan {
    /// The proposal being reversed.
    pub proposal_id: String,
    /// The original accept action being reversed.
    pub original_action_id: String,
    /// The entity that was the canonical entity in the original merge.
    pub canonical_entity_id: EntityId,
    /// The entity that was the non-canonical entity in the original merge.
    pub non_canonical_entity_id: EntityId,
    /// Items that can be reconstructed (no drift).
    pub resolvable_items: Vec<SplitReconstructionItem>,
    /// Items that cannot be reconstructed due to concurrent revision drift.
    pub unresolvable_items: Vec<UnresolvableItem>,
    /// Whether the split can proceed (true when no P0 unresolvable items exist).
    pub can_proceed: bool,
    /// Total items to reconstruct.
    pub total_items: u32,
}

// ── SplitReconstructionError ──────────────────────────────────────────────

/// Errors produced when building a [`SplitReconstructionPlan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitReconstructionError {
    /// The original action is not an Accept (only Accept can be reversed).
    NotAnAcceptAction,
    /// Revision drift exceeds the allowed limit.
    RevisionDrift { drift: u64, max_allowed: u64 },
    /// No resolvable items and no unresolvable items (nothing to reconstruct).
    EmptyReconstruction,
}

impl std::fmt::Display for SplitReconstructionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SplitReconstructionError::NotAnAcceptAction => write!(
                f,
                "split reconstruction: only Accept actions can be reversed; \
                 got a non-Accept action"
            ),
            SplitReconstructionError::RevisionDrift { drift, max_allowed } => write!(
                f,
                "split reconstruction: revision drift {drift} exceeds max allowed {max_allowed}"
            ),
            SplitReconstructionError::EmptyReconstruction => write!(
                f,
                "split reconstruction: no resolvable or unresolvable items — nothing to reconstruct"
            ),
        }
    }
}

impl std::error::Error for SplitReconstructionError {}

// ── SplitReconstructionBuilder ────────────────────────────────────────────

/// Stateless builder for [`SplitReconstructionPlan`].
pub struct SplitReconstructionBuilder;

impl SplitReconstructionBuilder {
    /// Build a split reconstruction plan from an accepted proposal action.
    ///
    /// Rules:
    /// - Only `ProposalActionKind::Accept` actions can be reversed →
    ///   [`SplitReconstructionError::NotAnAcceptAction`] otherwise.
    /// - Both `resolvable_items` and `unresolvable_items` being empty →
    ///   [`SplitReconstructionError::EmptyReconstruction`].
    /// - `can_proceed = !unresolvable_items.iter().any(|i| i.blocks_split)`.
    /// - `total_items = resolvable_items.len() as u32`.
    pub fn build(
        original_action: &ProposalAction,
        resolvable_items: Vec<SplitReconstructionItem>,
        unresolvable_items: Vec<UnresolvableItem>,
    ) -> Result<SplitReconstructionPlan, SplitReconstructionError> {
        // Only Accept actions can be reversed.
        if original_action.action_kind != ProposalActionKind::Accept {
            return Err(SplitReconstructionError::NotAnAcceptAction);
        }

        // Reject empty reconstruction (nothing to do).
        if resolvable_items.is_empty() && unresolvable_items.is_empty() {
            return Err(SplitReconstructionError::EmptyReconstruction);
        }

        let can_proceed = !unresolvable_items.iter().any(|i| i.blocks_split);
        let total_items = resolvable_items.len() as u32;

        Ok(SplitReconstructionPlan {
            proposal_id: original_action.proposal_id.clone(),
            original_action_id: original_action.id.clone(),
            canonical_entity_id: original_action.before_state.canonical_entity_id.clone(),
            non_canonical_entity_id: original_action.before_state.non_canonical_entity_id.clone(),
            resolvable_items,
            unresolvable_items,
            can_proceed,
            total_items,
        })
    }

    /// Validate that the revision has not drifted excessively.
    ///
    /// Returns `Err(SplitReconstructionError::RevisionDrift)` when
    /// `current_revision` is more than `max_allowed_drift` revisions ahead
    /// of `revision_at_merge`.
    pub fn check_revision_drift(
        revision_at_merge: GraphRevision,
        current_revision: GraphRevision,
        max_allowed_drift: u64,
    ) -> Result<(), SplitReconstructionError> {
        let at_merge = revision_at_merge.get();
        let current = current_revision.get();
        // If current is behind or equal to merge revision, no drift.
        let drift = current.saturating_sub(at_merge);
        if drift > max_allowed_drift {
            return Err(SplitReconstructionError::RevisionDrift {
                drift,
                max_allowed: max_allowed_drift,
            });
        }
        Ok(())
    }

    /// Detect alias conflicts in multi-scope scenarios.
    ///
    /// Returns the number of alias conflicts found.
    /// An alias conflict occurs when a normalized alias (Unicode lowercase)
    /// exists in both `canonical_aliases` and `non_canonical_aliases`.
    pub fn detect_alias_conflicts(
        canonical_aliases: &[String],
        non_canonical_aliases: &[String],
    ) -> u32 {
        let canonical_set: std::collections::HashSet<String> =
            canonical_aliases.iter().map(|a| a.to_lowercase()).collect();

        non_canonical_aliases
            .iter()
            .filter(|a| canonical_set.contains(&a.to_lowercase()))
            .count() as u32
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::model::entity_proposal::{
        EntityProposalBuilder, ProposalMatchMethod, ProposalRationale, ProposalStatus,
    };
    use crate::memory::model::proposal_action::{
        ProposalActionBuilder, ProposalActionKind, ProposalBeforeState,
    };
    use crate::memory::model::{EntityId, GraphRevision};

    // ── helpers ──────────────────────────────────────────────────────────

    fn make_accept_action() -> ProposalAction {
        let left = EntityId::new_v7();
        let right = EntityId::new_v7();
        let rationale = ProposalRationale {
            method: ProposalMatchMethod::NameSimilarity,
            features_version: "name-sim-v1".into(),
            similarity_score: None,
            score_semantics: None,
            description: "Test".into(),
            left_normalized: None,
            right_normalized: None,
        };
        let proposal = EntityProposalBuilder::create(
            EntityId::new_v7().into_string(),
            left.clone(),
            right.clone(),
            rationale,
            GraphRevision::new(1),
            "user".into(),
            "chat".into(),
            0,
            "policy-v1".into(),
        )
        .unwrap();
        let before = ProposalBeforeState {
            proposal_status: ProposalStatus::Unresolved,
            canonical_entity_id: left,
            non_canonical_entity_id: right,
            canonical_alias_count: 3,
            non_canonical_alias_count: 2,
            mention_count_to_migrate: 5,
            link_count_to_migrate: 2,
        };
        ProposalActionBuilder::accept(
            "action-accept-001".into(),
            &proposal,
            before,
            (2, 5, 2),
            GraphRevision::new(10),
            "actor-alice".into(),
            crate::memory::model::UtcTimestamp::now(),
        )
        .unwrap()
    }

    fn make_reject_action() -> ProposalAction {
        let left = EntityId::new_v7();
        let right = EntityId::new_v7();
        let rationale = ProposalRationale {
            method: ProposalMatchMethod::NameSimilarity,
            features_version: "name-sim-v1".into(),
            similarity_score: None,
            score_semantics: None,
            description: "Test".into(),
            left_normalized: None,
            right_normalized: None,
        };
        let proposal = EntityProposalBuilder::create(
            EntityId::new_v7().into_string(),
            left.clone(),
            right.clone(),
            rationale,
            GraphRevision::new(1),
            "user".into(),
            "chat".into(),
            0,
            "policy-v1".into(),
        )
        .unwrap();
        let before = ProposalBeforeState {
            proposal_status: ProposalStatus::Unresolved,
            canonical_entity_id: left,
            non_canonical_entity_id: right,
            canonical_alias_count: 1,
            non_canonical_alias_count: 1,
            mention_count_to_migrate: 0,
            link_count_to_migrate: 0,
        };
        ProposalActionBuilder::reject(
            "action-reject-001".into(),
            &proposal,
            before,
            GraphRevision::new(10),
            "actor-alice".into(),
        )
        .unwrap()
    }

    fn alias_item(alias_id: &str, from: EntityId, to: EntityId) -> SplitReconstructionItem {
        SplitReconstructionItem::AliasReturn {
            alias_id: alias_id.into(),
            from_entity_id: from,
            to_entity_id: to,
        }
    }

    fn non_blocking_item(reason: UnresolvableReason) -> UnresolvableItem {
        UnresolvableItem {
            description: "test".into(),
            reason,
            blocks_split: false,
        }
    }

    fn blocking_item(reason: UnresolvableReason) -> UnresolvableItem {
        UnresolvableItem {
            description: "blocker".into(),
            reason,
            blocks_split: true,
        }
    }

    // ── build: success with all resolvable items ──────────────────────

    #[test]
    fn build_success_with_resolvable_items() {
        let action = make_accept_action();
        let canonical = action.before_state.canonical_entity_id.clone();
        let non_canonical = action.before_state.non_canonical_entity_id.clone();
        let items = vec![
            alias_item("alias-1", canonical.clone(), non_canonical.clone()),
            SplitReconstructionItem::MentionReturn {
                mention_id: "mention-1".into(),
                from_entity_id: canonical.clone(),
                to_entity_id: non_canonical.clone(),
            },
        ];
        let plan = SplitReconstructionBuilder::build(&action, items.clone(), vec![]).unwrap();

        assert_eq!(plan.proposal_id, action.proposal_id);
        assert_eq!(plan.original_action_id, action.id);
        assert_eq!(plan.canonical_entity_id, canonical);
        assert_eq!(plan.non_canonical_entity_id, non_canonical);
        assert_eq!(plan.resolvable_items.len(), 2);
        assert!(plan.unresolvable_items.is_empty());
        assert!(plan.can_proceed, "no blocking items → can proceed");
        assert_eq!(plan.total_items, 2);
    }

    // ── build: NotAnAcceptAction when action is Reject ────────────────

    #[test]
    fn build_not_accept_action_when_reject() {
        let action = make_reject_action();
        let err = SplitReconstructionBuilder::build(
            &action,
            vec![SplitReconstructionItem::AliasReturn {
                alias_id: "a".into(),
                from_entity_id: EntityId::new_v7(),
                to_entity_id: EntityId::new_v7(),
            }],
            vec![],
        )
        .unwrap_err();

        assert_eq!(err, SplitReconstructionError::NotAnAcceptAction);
        assert!(err.to_string().contains("Accept"));
    }

    // ── build: EmptyReconstruction when both lists are empty ──────────

    #[test]
    fn build_empty_reconstruction_error() {
        let action = make_accept_action();
        let err = SplitReconstructionBuilder::build(&action, vec![], vec![]).unwrap_err();

        assert_eq!(err, SplitReconstructionError::EmptyReconstruction);
        assert!(err.to_string().contains("nothing to reconstruct"));
    }

    // ── can_proceed: true when no blocking items ──────────────────────

    #[test]
    fn can_proceed_true_when_no_blocking_items() {
        let action = make_accept_action();
        let canonical = action.before_state.canonical_entity_id.clone();
        let non_canonical = action.before_state.non_canonical_entity_id.clone();
        let unresolvable = vec![
            non_blocking_item(UnresolvableReason::AliasDeletedAfterMerge),
            non_blocking_item(UnresolvableReason::MentionDeletedAfterMerge),
        ];
        let resolvable = vec![alias_item("a-1", canonical, non_canonical)];
        let plan = SplitReconstructionBuilder::build(&action, resolvable, unresolvable).unwrap();

        assert!(
            plan.can_proceed,
            "non-blocking items must not block the split"
        );
    }

    // ── can_proceed: false when at least one item blocks ─────────────

    #[test]
    fn can_proceed_false_when_blocking_item_present() {
        let action = make_accept_action();
        let canonical = action.before_state.canonical_entity_id.clone();
        let non_canonical = action.before_state.non_canonical_entity_id.clone();
        let unresolvable = vec![
            non_blocking_item(UnresolvableReason::AliasDeletedAfterMerge),
            blocking_item(UnresolvableReason::ConcurrentRevisionDrift {
                revision_at_merge: GraphRevision::new(10),
                current_revision: GraphRevision::new(20),
            }),
        ];
        let resolvable = vec![alias_item("a-1", canonical, non_canonical)];
        let plan = SplitReconstructionBuilder::build(&action, resolvable, unresolvable).unwrap();

        assert!(!plan.can_proceed, "a blocking item must prevent the split");
    }

    // ── check_revision_drift: Ok when no drift ────────────────────────

    #[test]
    fn check_revision_drift_ok_when_no_drift() {
        let at_merge = GraphRevision::new(10);
        let current = GraphRevision::new(10);
        let result = SplitReconstructionBuilder::check_revision_drift(at_merge, current, 0);
        assert!(result.is_ok(), "same revision → no drift → Ok");
    }

    #[test]
    fn check_revision_drift_ok_within_limit() {
        let at_merge = GraphRevision::new(10);
        let current = GraphRevision::new(15);
        let result = SplitReconstructionBuilder::check_revision_drift(at_merge, current, 5);
        assert!(result.is_ok(), "drift 5, max 5 → Ok (boundary)");
    }

    // ── check_revision_drift: Err when drift > max_allowed ───────────

    #[test]
    fn check_revision_drift_err_when_exceeds_max() {
        let at_merge = GraphRevision::new(10);
        let current = GraphRevision::new(20);
        let err =
            SplitReconstructionBuilder::check_revision_drift(at_merge, current, 5).unwrap_err();

        match err {
            SplitReconstructionError::RevisionDrift { drift, max_allowed } => {
                assert_eq!(drift, 10);
                assert_eq!(max_allowed, 5);
            }
            other => panic!("expected RevisionDrift, got {other:?}"),
        }
    }

    #[test]
    fn check_revision_drift_err_zero_max_any_drift_blocks() {
        let at_merge = GraphRevision::new(5);
        let current = GraphRevision::new(6);
        let err =
            SplitReconstructionBuilder::check_revision_drift(at_merge, current, 0).unwrap_err();

        assert!(matches!(
            err,
            SplitReconstructionError::RevisionDrift {
                drift: 1,
                max_allowed: 0
            }
        ));
        assert!(err.to_string().contains("drift 1"));
    }

    // ── detect_alias_conflicts: counts overlapping normalized aliases ─

    #[test]
    fn detect_alias_conflicts_counts_overlaps() {
        let canonical = vec![
            "Alice".into(),
            "alice@example.com".into(),
            "AliceWonder".into(),
        ];
        let non_canonical = vec!["alice".into(), "Bob".into(), "alicewonder".into()];
        let count = SplitReconstructionBuilder::detect_alias_conflicts(&canonical, &non_canonical);
        // "alice" vs "Alice" → conflict; "alicewonder" vs "AliceWonder" → conflict
        assert_eq!(count, 2);
    }

    // ── detect_alias_conflicts: zero when no overlap ──────────────────

    #[test]
    fn detect_alias_conflicts_zero_when_no_overlap() {
        let canonical = vec!["Charlie".into(), "charlie@example.com".into()];
        let non_canonical = vec!["Dave".into(), "dave@example.com".into()];
        let count = SplitReconstructionBuilder::detect_alias_conflicts(&canonical, &non_canonical);
        assert_eq!(count, 0);
    }

    #[test]
    fn detect_alias_conflicts_empty_inputs() {
        assert_eq!(
            SplitReconstructionBuilder::detect_alias_conflicts(&[], &[]),
            0
        );
        assert_eq!(
            SplitReconstructionBuilder::detect_alias_conflicts(&["Alice".into()], &[]),
            0
        );
    }

    // ── SplitReconstructionError Display ─────────────────────────────

    #[test]
    fn error_display_messages() {
        let e1 = SplitReconstructionError::NotAnAcceptAction;
        assert!(e1.to_string().contains("Accept"));

        let e2 = SplitReconstructionError::RevisionDrift {
            drift: 7,
            max_allowed: 3,
        };
        assert!(e2.to_string().contains("7"));
        assert!(e2.to_string().contains("3"));

        let e3 = SplitReconstructionError::EmptyReconstruction;
        assert!(e3.to_string().contains("nothing"));
    }

    // ── LinkEndpointReturn and EvidenceReturn items ───────────────────

    #[test]
    fn build_with_link_and_evidence_items() {
        let action = make_accept_action();
        let canonical = action.before_state.canonical_entity_id.clone();
        let non_canonical = action.before_state.non_canonical_entity_id.clone();
        let items = vec![
            SplitReconstructionItem::LinkEndpointReturn {
                relationship_id: "rel-1".into(),
                endpoint_kind: LinkEndpointKind::Source,
                from_entity_id: canonical.clone(),
                to_entity_id: non_canonical.clone(),
            },
            SplitReconstructionItem::EvidenceReturn {
                evidence_id: "ev-1".into(),
                from_entity_id: canonical.clone(),
                to_entity_id: non_canonical.clone(),
            },
        ];
        let plan = SplitReconstructionBuilder::build(&action, items, vec![]).unwrap();
        assert_eq!(plan.total_items, 2);
        assert!(plan.can_proceed);
    }

    // ── unresolvable items without blocking still allow proceed ───────

    #[test]
    fn unresolvable_only_non_blocking_allows_proceed() {
        let action = make_accept_action();
        // No resolvable items, but one unresolvable non-blocking item
        let unresolvable = vec![non_blocking_item(
            UnresolvableReason::SupersededRecordConflict,
        )];
        let plan = SplitReconstructionBuilder::build(&action, vec![], unresolvable).unwrap();
        assert!(plan.can_proceed);
        assert_eq!(plan.total_items, 0);
    }

    // ── serde roundtrip for SplitReconstructionItem ───────────────────

    #[test]
    fn split_reconstruction_item_serde_roundtrip() {
        let from = EntityId::new_v7();
        let to = EntityId::new_v7();
        let item = SplitReconstructionItem::AliasReturn {
            alias_id: "alias-xyz".into(),
            from_entity_id: from.clone(),
            to_entity_id: to.clone(),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: SplitReconstructionItem = serde_json::from_str(&json).unwrap();
        assert_eq!(item, back);
    }

    // ── LinkEndpointKind serde roundtrip ─────────────────────────────

    #[test]
    fn link_endpoint_kind_serde_roundtrip() {
        for kind in [LinkEndpointKind::Source, LinkEndpointKind::Target] {
            let json = serde_json::to_string(&kind).unwrap();
            let back: LinkEndpointKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, back);
        }
        assert_eq!(
            serde_json::to_string(&LinkEndpointKind::Source).unwrap(),
            "\"source\""
        );
        assert_eq!(
            serde_json::to_string(&LinkEndpointKind::Target).unwrap(),
            "\"target\""
        );
    }
}
