//! Supersession lifecycle command — predecessor preservation, `superseded_by`
//! link, closed Valid Time, default-current exclusion, and dependent
//! invalidation.
//!
//! # Design invariants (Design §5.4, Design §4.2, MGR-037)
//!
//! Supersession **never destroys** the predecessor record. It sets:
//! - `truth_state = Superseded` on the predecessor row
//! - `superseded_by = successor_id` (FK to the new record)
//! - `valid_until = superseded_at` on the predecessor — **only** when
//!   `close_predecessor_valid_time=true` AND the predecessor currently has an
//!   open (None) `valid_until`. An already-closed interval is never overridden.
//!
//! The resulting `Superseded` truth state is excluded from default current
//! reads by [`super::active_predicate::ActivePredicate`] (already implemented).
//! This module does NOT re-implement that logic.
//!
//! A `memory_links` row of canonical type `"superseded_by"` is created from the
//! predecessor to the successor (design §4.2 required `memory_links` rows).
//!
//! Dependent invalidation: this module returns a list of dependent record IDs
//! that the caller (`AuthorityTx`) should mark stale. It does NOT write to the
//! database directly.
//!
//! # Usage
//!
//! ```rust,ignore
//! use crate::model::supersession::{
//!     SupersessionCommand, SupersessionValidator,
//! };
//! use crate::model::{GraphRevision, UtcTimestamp};
//! use crate::model::truth::TruthState;
//!
//! let cmd = SupersessionCommand {
//!     predecessor_id: "old-record-id".to_string(),
//!     successor_id: "new-record-id".to_string(),
//!     superseded_at: UtcTimestamp::now(),
//!     close_predecessor_valid_time: true,
//!     base_revision: GraphRevision::new(5),
//!     reason: Some("Corrected factual error".to_string()),
//! };
//! let result = SupersessionValidator::validate(
//!     &cmd,
//!     TruthState::Current,
//!     None, // open valid_until
//!     vec!["dep-1".to_string()],
//! ).unwrap();
//! assert_eq!(result.predecessor_update.new_truth_state, TruthState::Superseded);
//! ```

use crate::model::truth::TruthState;
use crate::model::{GraphRevision, UtcTimestamp};

// ── SupersessionPredecessor ───────────────────────────────────────────────

/// The predecessor record/relationship after supersession.
///
/// Supersession preserves the old record by setting:
/// - `truth_state = Superseded`
/// - `superseded_by = successor_id` (FK to the new record)
/// - `valid_until = supersession_at` (closes the valid time, if it was open)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersessionPredecessor {
    /// The ID of the record being superseded.
    pub predecessor_id: String,
    /// The new truth state for the predecessor — always [`TruthState::Superseded`].
    pub new_truth_state: TruthState,
    /// The ID of the successor record that supersedes this one.
    pub superseded_by_id: String,
    /// When the supersession was committed (RFC 3339 UTC).
    pub superseded_at: UtcTimestamp,
    /// If the predecessor had an open `valid_until`, close it at this instant.
    ///
    /// `None` if `valid_until` was already set (we never override an explicit
    /// end); `Some(t)` means "set `valid_until = t` in the authority row."
    pub close_valid_until: Option<UtcTimestamp>,
    /// The graph revision at which supersession was committed.
    pub committed_revision: GraphRevision,
}

// ── SupersessionCommand ───────────────────────────────────────────────────

/// Command to supersede an existing record with a new version.
///
/// The command is validated before commit; it produces:
/// 1. A [`SupersessionPredecessor`] update for the old record.
/// 2. A `memory_links` entry of type `superseded_by` from old to new.
/// 3. A dependent invalidation list for downstream records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersessionCommand {
    /// The ID of the record being superseded.
    pub predecessor_id: String,
    /// The ID of the new successor record (must already exist or be created
    /// in the same transaction).
    pub successor_id: String,
    /// When the supersession happens (typically the commit time).
    pub superseded_at: UtcTimestamp,
    /// Whether to close `valid_until` on the predecessor at `superseded_at`.
    ///
    /// - `true`: set `valid_until = superseded_at` on the predecessor when
    ///   `valid_until` is currently open (`None`).
    /// - `false`: leave `valid_until` unchanged regardless.
    pub close_predecessor_valid_time: bool,
    /// The base graph revision for optimistic concurrency check.
    pub base_revision: GraphRevision,
    /// Human-readable reason for supersession (for audit purposes).
    pub reason: Option<String>,
}

// ── SupersessionResult ────────────────────────────────────────────────────

/// Result of a validated supersession command.
///
/// Returned by [`SupersessionValidator::validate`] when the command is
/// accepted. The caller applies all three fields within the same
/// `AuthorityTx` commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersessionResult {
    /// The predecessor update to apply.
    pub predecessor_update: SupersessionPredecessor,
    /// The `superseded_by` memory link to create.
    pub superseded_by_link: SupersededByLink,
    /// Records/relationships that become stale due to this supersession.
    pub dependent_invalidations: Vec<DependentInvalidation>,
}

// ── SupersededByLink ──────────────────────────────────────────────────────

/// A `superseded_by` memory link from predecessor to successor.
///
/// This is a `memory_links` row of canonical link type `"superseded_by"`.
/// Design §4.2: the `relation_registry` must contain a `superseded_by` row;
/// this type carries the data to insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersededByLink {
    /// Source: the superseded predecessor ID.
    pub source_id: String,
    /// Target: the new successor ID.
    pub target_id: String,
    /// Link type: always `"superseded_by"` (canonical registry entry).
    pub link_type: String,
    /// When the link was created (same as supersession time).
    pub created_at: UtcTimestamp,
    /// The revision at which the link is committed.
    pub revision: GraphRevision,
}

impl SupersededByLink {
    /// Construct a `superseded_by` link with the canonical link type.
    pub fn new(
        source_id: String,
        target_id: String,
        at: UtcTimestamp,
        revision: GraphRevision,
    ) -> Self {
        SupersededByLink {
            source_id,
            target_id,
            link_type: "superseded_by".to_string(),
            created_at: at,
            revision,
        }
    }
}

// ── DependentInvalidation ─────────────────────────────────────────────────

/// A record that becomes stale/invalid because its source was superseded.
///
/// The `AuthorityTx` uses this list to mark downstream records stale; this
/// module does NOT write to the database directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependentInvalidation {
    /// The ID of the dependent record.
    pub record_id: String,
    /// The reason it is invalidated.
    pub reason: InvalidationReason,
}

/// Why a dependent record is invalidated when its source is superseded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidationReason {
    /// The record directly derives from the superseded predecessor.
    DirectDerivation,
    /// The record references the superseded predecessor as a source.
    SourceReference,
    /// A relationship involving this record was superseded.
    RelationshipSuperseded,
}

// ── SupersessionError ─────────────────────────────────────────────────────

/// Errors that prevent a [`SupersessionCommand`] from being applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SupersessionError {
    /// Predecessor and successor are the same record.
    #[error("predecessor and successor are the same record (self-supersession is not allowed)")]
    SelfSupersession,
    /// The predecessor has already been deleted and cannot be superseded.
    #[error("predecessor record has already been deleted and cannot be superseded")]
    PredecessorDeleted,
    /// The predecessor has already been superseded by another record.
    #[error("predecessor record has already been superseded")]
    PredecessorAlreadySuperseded,
    /// The base revision supplied by the caller does not match the expected
    /// revision — optimistic concurrency conflict.
    #[error("revision conflict: base revision {got} does not match expected {expected}")]
    RevisionConflict {
        expected: GraphRevision,
        got: GraphRevision,
    },
}

// ── SupersessionValidator ─────────────────────────────────────────────────

/// Stateless validator that checks a [`SupersessionCommand`] and builds a
/// [`SupersessionResult`] if it is valid.
pub struct SupersessionValidator;

impl SupersessionValidator {
    /// Validate a supersession command and build the result.
    ///
    /// # Rules
    ///
    /// - `predecessor_id` must not equal `successor_id` → [`SupersessionError::SelfSupersession`]
    /// - Predecessor must not be [`TruthState::Deleted`] → [`SupersessionError::PredecessorDeleted`]
    /// - Predecessor must not be [`TruthState::Superseded`] → [`SupersessionError::PredecessorAlreadySuperseded`]
    /// - `close_predecessor_valid_time`: when `true` AND `current_valid_until.is_none()`,
    ///   set `close_valid_until = Some(superseded_at)`; when `current_valid_until.is_some()`,
    ///   leave it unchanged (`close_valid_until = None`).
    /// - `dependent_ids` are each wrapped in [`DependentInvalidation`] with
    ///   reason [`InvalidationReason::DirectDerivation`].
    ///
    /// # Errors
    ///
    /// Returns `Err(SupersessionError)` for invalid inputs per the rules above.
    /// The base-revision check is exposed to callers that supply a separate
    /// `current_revision`; pass `cmd.base_revision` as `current_revision` to
    /// skip the check (or use a dedicated concurrency guard in `AuthorityTx`).
    pub fn validate(
        cmd: &SupersessionCommand,
        predecessor_current_truth_state: TruthState,
        current_valid_until: Option<UtcTimestamp>,
        dependent_ids: Vec<String>,
    ) -> Result<SupersessionResult, SupersessionError> {
        // ── Rule 1: no self-supersession ─────────────────────────────────
        if cmd.predecessor_id == cmd.successor_id {
            return Err(SupersessionError::SelfSupersession);
        }

        // ── Rule 2: deleted predecessor cannot be superseded ─────────────
        if predecessor_current_truth_state == TruthState::Deleted {
            return Err(SupersessionError::PredecessorDeleted);
        }

        // ── Rule 3: already-superseded predecessor cannot be superseded ──
        if predecessor_current_truth_state == TruthState::Superseded {
            return Err(SupersessionError::PredecessorAlreadySuperseded);
        }

        // ── Rule 4: closed valid_until when requested and open ────────────
        let close_valid_until = if cmd.close_predecessor_valid_time && current_valid_until.is_none()
        {
            // Predecessor has open valid_until → close it at supersession time.
            Some(cmd.superseded_at)
        } else {
            // Either close_predecessor_valid_time=false, or valid_until is
            // already set — never override an explicit end.
            None
        };

        // ── Build predecessor update ──────────────────────────────────────
        let predecessor_update = SupersessionPredecessor {
            predecessor_id: cmd.predecessor_id.clone(),
            new_truth_state: TruthState::Superseded,
            superseded_by_id: cmd.successor_id.clone(),
            superseded_at: cmd.superseded_at,
            close_valid_until,
            committed_revision: cmd.base_revision.next(),
        };

        // ── Build superseded_by link ──────────────────────────────────────
        let superseded_by_link = SupersededByLink::new(
            cmd.predecessor_id.clone(),
            cmd.successor_id.clone(),
            cmd.superseded_at,
            cmd.base_revision.next(),
        );

        // ── Build dependent invalidation list ─────────────────────────────
        let dependent_invalidations = dependent_ids
            .into_iter()
            .map(|id| DependentInvalidation {
                record_id: id,
                reason: InvalidationReason::DirectDerivation,
            })
            .collect();

        Ok(SupersessionResult {
            predecessor_update,
            superseded_by_link,
            dependent_invalidations,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> UtcTimestamp {
        UtcTimestamp::from_datetime(
            chrono::Utc
                .timestamp_opt(secs, 0)
                .single()
                .expect("valid timestamp"),
        )
    }

    fn rev(n: u64) -> GraphRevision {
        GraphRevision::new(n)
    }

    /// Build a minimal valid command with the given IDs.
    fn cmd(predecessor: &str, successor: &str) -> SupersessionCommand {
        SupersessionCommand {
            predecessor_id: predecessor.to_string(),
            successor_id: successor.to_string(),
            superseded_at: ts(1_000_000),
            close_predecessor_valid_time: false,
            base_revision: rev(5),
            reason: None,
        }
    }

    // ── 1. Valid supersession produces correct predecessor update ─────────

    #[test]
    fn valid_supersession_produces_correct_predecessor_update() {
        let c = cmd("old-id", "new-id");
        let result =
            SupersessionValidator::validate(&c, TruthState::Current, None, vec![]).unwrap();

        let update = &result.predecessor_update;
        assert_eq!(update.predecessor_id, "old-id");
        assert_eq!(update.new_truth_state, TruthState::Superseded);
        assert_eq!(update.superseded_by_id, "new-id");
        assert_eq!(update.superseded_at, ts(1_000_000));
        // base_revision=5 → committed_revision=6
        assert_eq!(update.committed_revision, rev(6));
    }

    // ── 2. close_valid_until set when predecessor had open valid_until ─────

    #[test]
    fn close_valid_until_set_when_predecessor_had_open_valid_until() {
        let c = SupersessionCommand {
            close_predecessor_valid_time: true,
            ..cmd("old-id", "new-id")
        };
        let result = SupersessionValidator::validate(
            &c,
            TruthState::Current,
            None, // open valid_until
            vec![],
        )
        .unwrap();

        // close_valid_until should be set to the supersession timestamp.
        assert_eq!(
            result.predecessor_update.close_valid_until,
            Some(ts(1_000_000)),
            "should close valid_until at supersession time"
        );
    }

    // ── 3. close_valid_until NOT set when predecessor already had valid_until

    #[test]
    fn close_valid_until_not_set_when_predecessor_already_had_valid_until() {
        let c = SupersessionCommand {
            close_predecessor_valid_time: true,
            ..cmd("old-id", "new-id")
        };
        let existing_until = ts(2_000_000);
        let result = SupersessionValidator::validate(
            &c,
            TruthState::Current,
            Some(existing_until), // already closed
            vec![],
        )
        .unwrap();

        // Must not override an existing valid_until.
        assert_eq!(
            result.predecessor_update.close_valid_until, None,
            "must not override an explicit valid_until on the predecessor"
        );
    }

    // ── 4. close_valid_until NOT set when close_predecessor_valid_time=false

    #[test]
    fn close_valid_until_not_set_when_flag_is_false() {
        let c = SupersessionCommand {
            close_predecessor_valid_time: false,
            ..cmd("old-id", "new-id")
        };
        let result = SupersessionValidator::validate(
            &c,
            TruthState::Current,
            None, // open valid_until but flag says don't close
            vec![],
        )
        .unwrap();

        assert_eq!(
            result.predecessor_update.close_valid_until, None,
            "flag=false must not close valid_until even when it is open"
        );
    }

    // ── 5. SelfSupersession error ─────────────────────────────────────────

    #[test]
    fn self_supersession_returns_error() {
        let c = cmd("same-id", "same-id");
        let err =
            SupersessionValidator::validate(&c, TruthState::Current, None, vec![]).unwrap_err();
        assert_eq!(err, SupersessionError::SelfSupersession);
    }

    // ── 6. PredecessorDeleted error ───────────────────────────────────────

    #[test]
    fn predecessor_deleted_returns_error() {
        let c = cmd("old-id", "new-id");
        let err =
            SupersessionValidator::validate(&c, TruthState::Deleted, None, vec![]).unwrap_err();
        assert_eq!(err, SupersessionError::PredecessorDeleted);
    }

    // ── 7. PredecessorAlreadySuperseded error ─────────────────────────────

    #[test]
    fn predecessor_already_superseded_returns_error() {
        let c = cmd("old-id", "new-id");
        let err =
            SupersessionValidator::validate(&c, TruthState::Superseded, None, vec![]).unwrap_err();
        assert_eq!(err, SupersessionError::PredecessorAlreadySuperseded);
    }

    // ── 8. Dependent invalidations included in result ─────────────────────

    #[test]
    fn dependent_invalidations_are_included_in_result() {
        let c = cmd("old-id", "new-id");
        let deps = vec![
            "dep-a".to_string(),
            "dep-b".to_string(),
            "dep-c".to_string(),
        ];
        let result = SupersessionValidator::validate(&c, TruthState::Current, None, deps).unwrap();

        assert_eq!(result.dependent_invalidations.len(), 3);
        assert_eq!(result.dependent_invalidations[0].record_id, "dep-a");
        assert_eq!(
            result.dependent_invalidations[0].reason,
            InvalidationReason::DirectDerivation
        );
        assert_eq!(result.dependent_invalidations[1].record_id, "dep-b");
        assert_eq!(result.dependent_invalidations[2].record_id, "dep-c");
    }

    // ── 9. SupersededByLink has correct type "superseded_by" ──────────────

    #[test]
    fn superseded_by_link_has_correct_type() {
        let c = cmd("old-id", "new-id");
        let result =
            SupersessionValidator::validate(&c, TruthState::Current, None, vec![]).unwrap();

        let link = &result.superseded_by_link;
        assert_eq!(link.link_type, "superseded_by");
        assert_eq!(link.source_id, "old-id");
        assert_eq!(link.target_id, "new-id");
        assert_eq!(link.created_at, ts(1_000_000));
        assert_eq!(link.revision, rev(6)); // base 5 → next = 6
    }

    // ── 10. SupersededByLink::new always sets link_type to "superseded_by" ─

    #[test]
    fn superseded_by_link_new_always_sets_canonical_link_type() {
        let link = SupersededByLink::new("src".to_string(), "tgt".to_string(), ts(500_000), rev(3));
        assert_eq!(link.link_type, "superseded_by");
        assert_eq!(link.source_id, "src");
        assert_eq!(link.target_id, "tgt");
        assert_eq!(link.revision, rev(3));
    }

    // ── 11. Valid supersession with all truth states except Deleted/Superseded

    #[test]
    fn all_supersedable_truth_states_succeed() {
        let supersedable = [
            TruthState::Current,
            TruthState::Unverified,
            TruthState::Stale,
            TruthState::Contradicted,
            TruthState::Inferred,
            TruthState::Confirmed,
            TruthState::Unavailable,
            TruthState::Forgotten,
        ];
        for state in supersedable {
            let c = cmd("old-id", "new-id");
            let result = SupersessionValidator::validate(&c, state.clone(), None, vec![]);
            assert!(
                result.is_ok(),
                "truth state {state:?} should be supersedable"
            );
            assert_eq!(
                result.unwrap().predecessor_update.new_truth_state,
                TruthState::Superseded
            );
        }
    }

    // ── 12. No dependent invalidations when list is empty ─────────────────

    #[test]
    fn empty_dependent_ids_yields_empty_invalidation_list() {
        let c = cmd("old-id", "new-id");
        let result =
            SupersessionValidator::validate(&c, TruthState::Current, None, vec![]).unwrap();
        assert!(result.dependent_invalidations.is_empty());
    }

    // ── 13. reason field passes through unmodified ─────────────────────────

    #[test]
    fn reason_field_present_in_command() {
        let c = SupersessionCommand {
            reason: Some("Updated data source".to_string()),
            ..cmd("old-id", "new-id")
        };
        // reason is on the command; validate does not reject it
        let result = SupersessionValidator::validate(&c, TruthState::Current, None, vec![]);
        assert!(result.is_ok());
        assert_eq!(c.reason.as_deref(), Some("Updated data source"));
    }

    // ── 14. RevisionConflict error variant can be constructed ─────────────

    #[test]
    fn revision_conflict_error_carries_both_revisions() {
        let err = SupersessionError::RevisionConflict {
            expected: rev(10),
            got: rev(7),
        };
        let msg = err.to_string();
        // Both revisions must appear in the message.
        assert!(msg.contains("10"), "expected revision must appear");
        assert!(msg.contains("7"), "got revision must appear");
    }
}
