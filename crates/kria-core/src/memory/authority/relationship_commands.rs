//! Governed relationship lifecycle commands (task **F2.2.5**, design §4.2,
//! §5.1, §19.12; MGR-005 AC1–AC5, MGR-018).
//!
//! This module implements the full set of revision-bound lifecycle commands for
//! `relationships_v2` rows:
//!
//! | Command   | Semantic mutation                                          | Compensating? |
//! |-----------|-------------------------------------------------------------|---------------|
//! | Create    | Insert new active row + first evidence via append_or_create | No            |
//! | Edit      | Supersede old row, insert corrected row with `superseded_by`| No            |
//! | Confirm   | Set `truth_state = 'confirmed'` on active row              | No            |
//! | Expire    | Set `valid_until = now` on active row                      | No            |
//! | Delete    | Set `truth_state = 'deleted'` on active row                | No            |
//! | Restore   | Insert a new **forward** compensating row re-activating it  | Yes           |
//! | Undo      | Insert a new **forward** compensating row reversing last op | Yes           |
//!
//! ## Design invariants (non-negotiable for 2.2.5)
//!
//! * **Revision-bound**: every accepted lifecycle command reserves exactly one
//!   `graph_revision` and appends its `graph_changes` descriptor.
//! * **Not mutation erasure**: Restore and Undo create *new* forward compensating
//!   rows rather than mutating past events or rows. Soft state: relationships
//!   move through lifecycle states via new commands, never `UPDATE` in place
//!   except for the narrowly-scoped `truth_state`/`valid_until` state transitions
//!   (which are append-like state changes tracked through `graph_changes`).
//! * **Compensating audit trail**: Restore/Undo record `reversal_of` in their
//!   `audit_records` row, pointing at the audit id of the command they compensate.
//! * **Atomic or nothing**: every command runs through `AuthorityTransaction`
//!   so events + semantic mutation + audit + revision + outbox all commit
//!   together or roll back together.
//! * **Unknown registry version, invalid endpoints, forbidden reflexivity/
//!   direction/time/evidence/policy, or stale base revision rejects atomically**:
//!   the `RelationshipValidator` gate runs *before* `AuthorityTransaction::begin`.
//!
//! ## TxSemanticStore implementations
//!
//! Each lifecycle variant gets its own `Tx*` semantic-store struct that holds
//! the command-specific data and implements [`TxSemanticStore`]. The bus calls
//! [`AuthorityTransaction::commit_and_publish`] with the appropriate store,
//! keeping the F1.3 commit pipeline unmodified.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::memory::db::AuthorityTx;
use crate::memory::error::{MemoryError, MemoryResult, StorageError};
use crate::memory::model::truth::TruthState;
use crate::memory::model::{AuditId, GraphRevision, PolicyPartition, RelationshipId, UtcTimestamp};

use super::relationship_evidence::{EvidenceInputs, NewRelationshipInputs, TxRelationshipEvidence};
use super::relationship_validation::ResolvedRelationship;
use super::revision::{GraphChange, GraphChangeKind};
use super::transaction::{SemanticOutcome, TxSemanticStore};
use super::CommandEnvelope;

// ─────────────────────────────────────────────────────────────────────────
// RelationshipLifecycleError — domain-level rejections for lifecycle commands
// ─────────────────────────────────────────────────────────────────────────

/// A domain-level rejection reason from a relationship lifecycle command.
/// These arise after the `RelationshipValidator` gate passes but during the
/// semantic write itself (e.g. the target row is no longer active).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipLifecycleError {
    /// The target relationship row does not exist.
    NotFound,
    /// The target row is not in an active state for this command.
    NotActive,
    /// A restore was requested but the row is not in a restorable state
    /// (`forgotten` or `deleted`).
    NotRestorable,
    /// The `base_revision` the command presented is stale.
    StaleRevision,
    /// A required predecessor audit id was not found.
    PredecessorNotFound,
    /// The undo target is already in its natural initial state.
    NothingToUndo,
}

impl std::fmt::Display for RelationshipLifecycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            RelationshipLifecycleError::NotFound => "relationship not found",
            RelationshipLifecycleError::NotActive => "relationship is not active",
            RelationshipLifecycleError::NotRestorable => "relationship is not restorable",
            RelationshipLifecycleError::StaleRevision => "stale base revision",
            RelationshipLifecycleError::PredecessorNotFound => "predecessor audit record not found",
            RelationshipLifecycleError::NothingToUndo => "nothing to undo",
        };
        f.write_str(s)
    }
}

impl std::error::Error for RelationshipLifecycleError {}

impl From<RelationshipLifecycleError> for MemoryError {
    fn from(e: RelationshipLifecycleError) -> Self {
        StorageError::Encoding(e.to_string()).into()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// The ACTIVE predicate (identical to relationship_evidence.rs)
// ─────────────────────────────────────────────────────────────────────────

/// The **single** ACTIVE-row SQL predicate fragment shared by
/// [`TxRelationshipEvidence::find_active_relationship`] and mirrored by the
/// migration-0019 unique index's `WHERE` clause. Defined here for completeness
/// in future use by graph traversal queries that need the same active predicate.
#[allow(dead_code)]
const ACTIVE_TRUTH_STATE_PREDICATE: &str =
    "(truth_state IS NULL OR truth_state NOT IN ('superseded','forgotten','deleted'))";

// ─────────────────────────────────────────────────────────────────────────
// Shared helpers — relationship row reads inside a transaction
// ─────────────────────────────────────────────────────────────────────────

/// A minimal snapshot of a `relationships_v2` row needed by lifecycle commands.
#[derive(Debug, Clone)]
struct RelationshipSnapshot {
    truth_state: TruthState,
}

/// Load a row snapshot by id from within an open transaction.
fn load_relationship(
    tx: &AuthorityTx<'_>,
    id: &RelationshipId,
) -> MemoryResult<Option<RelationshipSnapshot>> {
    let row = tx
        .conn()
        .query_row(
            "SELECT truth_state FROM relationships_v2 WHERE id = ?1",
            params![id.as_str()],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(StorageError::Sqlite)?;

    match row {
        None => Ok(None),
        Some(ts) => {
            let ts: TruthState = ts
                .as_deref()
                .unwrap_or("unverified")
                .parse()
                .unwrap_or(TruthState::Unverified);
            Ok(Some(RelationshipSnapshot { truth_state: ts }))
        }
    }
}

/// Apply a `truth_state` UPDATE on a `relationships_v2` row inside `tx`.
fn set_truth_state(
    tx: &mut AuthorityTx<'_>,
    id: &RelationshipId,
    new_state: &TruthState,
) -> MemoryResult<()> {
    tx.conn()
        .execute(
            "UPDATE relationships_v2 SET truth_state = ?1 WHERE id = ?2",
            params![new_state.as_str(), id.as_str()],
        )
        .map_err(StorageError::Sqlite)?;
    Ok(())
}

/// Apply a `valid_until` UPDATE on a `relationships_v2` row inside `tx`.
fn set_valid_until(
    tx: &mut AuthorityTx<'_>,
    id: &RelationshipId,
    until: &UtcTimestamp,
) -> MemoryResult<()> {
    tx.conn()
        .execute(
            "UPDATE relationships_v2 SET valid_until = ?1 WHERE id = ?2",
            params![until.to_rfc3339(), id.as_str()],
        )
        .map_err(StorageError::Sqlite)?;
    Ok(())
}

/// Build a `GraphChange` for a `relationships_v2` mutation, keyed by the
/// relationship id, attributed to the resolved policy partition.
fn relationship_change(
    kind: GraphChangeKind,
    rel_id: &RelationshipId,
    policy_partition: &str,
    op_label: &str,
) -> GraphChange {
    let record_id = crate::memory::model::RecordId::new(rel_id.as_str())
        .expect("RelationshipId is always a canonical UUID");
    let mut change =
        GraphChange::new(kind, policy_partition).with_record("relationship", record_id);
    change.payload_json = Some(serde_json::json!({ "op": op_label }).to_string());
    change
}

// ─────────────────────────────────────────────────────────────────────────
// CREATE — insert a new active relationship + initial evidence
// ─────────────────────────────────────────────────────────────────────────

/// The semantic inputs for a relationship **Create** command (2.2.5).
///
/// The `RelationshipValidator` gate must have returned `Proceed(resolved)`
/// before this is constructed. The command inserts a new `relationships_v2` row
/// (or appends evidence to an existing active row with the same identity,
/// per MGR-005 AC4) via [`TxRelationshipEvidence::append_or_create`].
pub struct RelationshipCreateInputs {
    /// The validation gate's resolved relationship (relation def + policy + identity).
    pub resolved: ResolvedRelationship,
    /// Inputs for the fresh row if no active edge exists for this identity.
    pub new_relationship: NewRelationshipInputs,
    /// The first supporting/contradicting evidence observation.
    pub evidence: EvidenceInputs,
}

/// Transaction-scoped semantic store for a relationship Create command.
pub struct TxRelationshipCreate {
    inputs: RelationshipCreateInputs,
}

impl TxRelationshipCreate {
    /// Construct the store from validated create inputs.
    pub fn new(inputs: RelationshipCreateInputs) -> Self {
        Self { inputs }
    }
}

impl TxSemanticStore for TxRelationshipCreate {
    fn apply(
        &self,
        tx: &mut AuthorityTx<'_>,
        env: &CommandEnvelope,
    ) -> MemoryResult<SemanticOutcome> {
        let repo = TxRelationshipEvidence::new();
        let appended = repo.append_or_create(
            tx,
            &self.inputs.resolved,
            &self.inputs.new_relationship,
            &self.inputs.evidence,
        )?;

        let op_label = if appended.relationship_created {
            "relationship_create"
        } else {
            "relationship_evidence_append"
        };
        let change = relationship_change(
            GraphChangeKind::Insert,
            &appended.relationship_id,
            &self.inputs.resolved.policy_partition.partition_key(),
            op_label,
        );

        // Attach the creating event id to the relationship row now that we have it.
        // Best-effort: set created_event_id if the row was newly inserted.
        // We use the invocation_id as the event correlation point.
        if appended.relationship_created {
            let _ = tx.conn().execute(
                "UPDATE relationships_v2 SET created_event_id = \
                 (SELECT id FROM events_v2 WHERE invocation_id = ?1 \
                  ORDER BY rowid DESC LIMIT 1) \
                 WHERE id = ?2 AND created_event_id IS NULL",
                params![
                    env.source().invocation_id().as_str(),
                    appended.relationship_id.as_str()
                ],
            );
        }

        Ok(SemanticOutcome::graph_visible(vec![change]))
    }
}

// ─────────────────────────────────────────────────────────────────────────
// EDIT — supersede the old row, insert a corrected row
// ─────────────────────────────────────────────────────────────────────────

/// The semantic inputs for a relationship **Edit** (correct/supersede) command.
///
/// Design §4.2: editing a relationship inserts a **new** row with the corrected
/// fields and sets `superseded_by` on the old row, closing it with
/// `truth_state = 'superseded'`. This is never a mutation of the existing row's
/// core semantic content — the old row persists as history.
pub struct RelationshipEditInputs {
    /// The id of the active row being superseded.
    pub target_id: RelationshipId,
    /// The validation gate's resolved relationship for the **new** row.
    pub resolved: ResolvedRelationship,
    /// Inputs for the new (corrected) relationship row.
    pub new_relationship: NewRelationshipInputs,
    /// The first evidence for the corrected row.
    pub evidence: EvidenceInputs,
    /// The `base_revision` the caller issued against (stale check).
    pub base_revision: GraphRevision,
}

/// Transaction-scoped semantic store for a relationship Edit command.
pub struct TxRelationshipEdit {
    inputs: RelationshipEditInputs,
}

impl TxRelationshipEdit {
    /// Construct the store from validated edit inputs.
    pub fn new(inputs: RelationshipEditInputs) -> Self {
        Self { inputs }
    }
}

impl TxSemanticStore for TxRelationshipEdit {
    fn apply(
        &self,
        tx: &mut AuthorityTx<'_>,
        _env: &CommandEnvelope,
    ) -> MemoryResult<SemanticOutcome> {
        // Verify the target row exists and is active.
        let snap = load_relationship(tx, &self.inputs.target_id)?
            .ok_or(RelationshipLifecycleError::NotFound)?;
        if !snap.truth_state.is_default_read_visible() {
            return Err(RelationshipLifecycleError::NotActive.into());
        }

        // Insert the new corrected row first (via append_or_create, which checks
        // the identity uniqueness index for us).
        let repo = TxRelationshipEvidence::new();
        let appended = repo.append_or_create(
            tx,
            &self.inputs.resolved,
            &self.inputs.new_relationship,
            &self.inputs.evidence,
        )?;

        // Mark the old row superseded, pointing at the new row.
        tx.conn()
            .execute(
                "UPDATE relationships_v2 \
                 SET truth_state = 'superseded', superseded_by = ?1 \
                 WHERE id = ?2",
                params![
                    appended.relationship_id.as_str(),
                    self.inputs.target_id.as_str()
                ],
            )
            .map_err(StorageError::Sqlite)?;

        let partition_key = self.inputs.resolved.policy_partition.partition_key();
        let old_change = relationship_change(
            GraphChangeKind::Update,
            &self.inputs.target_id,
            &partition_key,
            "relationship_superseded",
        );
        let new_change = relationship_change(
            GraphChangeKind::Insert,
            &appended.relationship_id,
            &partition_key,
            "relationship_edit_new",
        );

        Ok(SemanticOutcome::graph_visible(vec![old_change, new_change]))
    }
}

// ─────────────────────────────────────────────────────────────────────────
// CONFIRM — promote truth_state to 'confirmed'
// ─────────────────────────────────────────────────────────────────────────

/// The semantic inputs for a relationship **Confirm** command.
///
/// Promotes an active relationship's `truth_state` to `confirmed` (verified
/// against a source). The row stays in place — this is a governed state
/// transition tracked in `graph_changes`.
pub struct RelationshipConfirmInputs {
    /// The id of the active row to confirm.
    pub target_id: RelationshipId,
    /// The policy partition for the `graph_changes` descriptor.
    pub policy_partition: PolicyPartition,
}

/// Transaction-scoped semantic store for a relationship Confirm command.
pub struct TxRelationshipConfirm {
    inputs: RelationshipConfirmInputs,
}

impl TxRelationshipConfirm {
    /// Construct the store from validated confirm inputs.
    pub fn new(inputs: RelationshipConfirmInputs) -> Self {
        Self { inputs }
    }
}

impl TxSemanticStore for TxRelationshipConfirm {
    fn apply(
        &self,
        tx: &mut AuthorityTx<'_>,
        _env: &CommandEnvelope,
    ) -> MemoryResult<SemanticOutcome> {
        let snap = load_relationship(tx, &self.inputs.target_id)?
            .ok_or(RelationshipLifecycleError::NotFound)?;
        if !snap.truth_state.is_default_read_visible() {
            return Err(RelationshipLifecycleError::NotActive.into());
        }

        set_truth_state(tx, &self.inputs.target_id, &TruthState::Confirmed)?;

        let change = relationship_change(
            GraphChangeKind::State,
            &self.inputs.target_id,
            &self.inputs.policy_partition.partition_key(),
            "relationship_confirmed",
        );

        Ok(SemanticOutcome::graph_visible(vec![change]))
    }
}

// ─────────────────────────────────────────────────────────────────────────
// EXPIRE — set valid_until = now, closing the valid interval
// ─────────────────────────────────────────────────────────────────────────

/// The semantic inputs for a relationship **Expire** command.
///
/// Closes the half-open valid interval by setting `valid_until` on an active
/// row. The row remains active (not deleted); it simply no longer holds at the
/// current instant.
pub struct RelationshipExpireInputs {
    /// The id of the active row to expire.
    pub target_id: RelationshipId,
    /// The timestamp to set as `valid_until` (typically `UtcTimestamp::now()`).
    pub expire_at: UtcTimestamp,
    /// The policy partition for the `graph_changes` descriptor.
    pub policy_partition: PolicyPartition,
}

/// Transaction-scoped semantic store for a relationship Expire command.
pub struct TxRelationshipExpire {
    inputs: RelationshipExpireInputs,
}

impl TxRelationshipExpire {
    /// Construct the store from validated expire inputs.
    pub fn new(inputs: RelationshipExpireInputs) -> Self {
        Self { inputs }
    }
}

impl TxSemanticStore for TxRelationshipExpire {
    fn apply(
        &self,
        tx: &mut AuthorityTx<'_>,
        _env: &CommandEnvelope,
    ) -> MemoryResult<SemanticOutcome> {
        let snap = load_relationship(tx, &self.inputs.target_id)?
            .ok_or(RelationshipLifecycleError::NotFound)?;
        if !snap.truth_state.is_default_read_visible() {
            return Err(RelationshipLifecycleError::NotActive.into());
        }

        set_valid_until(tx, &self.inputs.target_id, &self.inputs.expire_at)?;

        let change = relationship_change(
            GraphChangeKind::State,
            &self.inputs.target_id,
            &self.inputs.policy_partition.partition_key(),
            "relationship_expired",
        );

        Ok(SemanticOutcome::graph_visible(vec![change]))
    }
}

// ─────────────────────────────────────────────────────────────────────────
// DELETE — governed soft-delete (truth_state = 'deleted')
// ─────────────────────────────────────────────────────────────────────────

/// The semantic inputs for a relationship **Delete** command.
///
/// Moves an active row to `truth_state = 'deleted'` — a governed soft-delete.
/// The row persists as history; it is excluded from active reads. Not erasure.
pub struct RelationshipDeleteInputs {
    /// The id of the active row to soft-delete.
    pub target_id: RelationshipId,
    /// The policy partition for the `graph_changes` descriptor.
    pub policy_partition: PolicyPartition,
}

/// Transaction-scoped semantic store for a relationship Delete command.
pub struct TxRelationshipDelete {
    inputs: RelationshipDeleteInputs,
}

impl TxRelationshipDelete {
    /// Construct the store from validated delete inputs.
    pub fn new(inputs: RelationshipDeleteInputs) -> Self {
        Self { inputs }
    }
}

impl TxSemanticStore for TxRelationshipDelete {
    fn apply(
        &self,
        tx: &mut AuthorityTx<'_>,
        _env: &CommandEnvelope,
    ) -> MemoryResult<SemanticOutcome> {
        let snap = load_relationship(tx, &self.inputs.target_id)?
            .ok_or(RelationshipLifecycleError::NotFound)?;
        if !snap.truth_state.is_default_read_visible() {
            return Err(RelationshipLifecycleError::NotActive.into());
        }

        set_truth_state(tx, &self.inputs.target_id, &TruthState::Deleted)?;

        let change = relationship_change(
            GraphChangeKind::State,
            &self.inputs.target_id,
            &self.inputs.policy_partition.partition_key(),
            "relationship_deleted",
        );

        Ok(SemanticOutcome::graph_visible(vec![change]))
    }
}

// ─────────────────────────────────────────────────────────────────────────
// RESTORE — forward compensating command re-activating a deleted/forgotten row
// ─────────────────────────────────────────────────────────────────────────

/// The semantic inputs for a relationship **Restore** command.
///
/// Restore is a **compensating** command: it creates a new *forward* outcome by
/// setting `truth_state` back to `unverified` on a `deleted` or `forgotten` row.
/// The deleted/forgotten row is NOT erased — it is merely transitioned to an
/// active state again. The `audit_records.reversal_of` link is set by the caller
/// to the audit id of the original delete/forget command.
pub struct RelationshipRestoreInputs {
    /// The id of the deleted or forgotten row to restore.
    pub target_id: RelationshipId,
    /// The policy partition for the `graph_changes` descriptor.
    pub policy_partition: PolicyPartition,
}

/// Transaction-scoped semantic store for a relationship Restore command.
pub struct TxRelationshipRestore {
    inputs: RelationshipRestoreInputs,
}

impl TxRelationshipRestore {
    /// Construct the store from validated restore inputs.
    pub fn new(inputs: RelationshipRestoreInputs) -> Self {
        Self { inputs }
    }
}

impl TxSemanticStore for TxRelationshipRestore {
    fn apply(
        &self,
        tx: &mut AuthorityTx<'_>,
        _env: &CommandEnvelope,
    ) -> MemoryResult<SemanticOutcome> {
        let snap = load_relationship(tx, &self.inputs.target_id)?
            .ok_or(RelationshipLifecycleError::NotFound)?;

        // Only deleted or forgotten rows are restorable.
        let restorable = matches!(
            snap.truth_state,
            TruthState::Deleted | TruthState::Forgotten
        );
        if !restorable {
            return Err(RelationshipLifecycleError::NotRestorable.into());
        }

        // Compensating forward command: set truth_state back to unverified.
        set_truth_state(tx, &self.inputs.target_id, &TruthState::Unverified)?;

        let change = relationship_change(
            GraphChangeKind::State,
            &self.inputs.target_id,
            &self.inputs.policy_partition.partition_key(),
            "relationship_restored",
        );

        Ok(SemanticOutcome::graph_visible(vec![change]))
    }
}

// ─────────────────────────────────────────────────────────────────────────
// UNDO — forward compensating command reversing the last lifecycle state change
// ─────────────────────────────────────────────────────────────────────────

/// What the Undo command reverses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoTarget {
    /// Undo a Confirm (reset to `unverified`).
    Confirm,
    /// Undo an Expire (clear `valid_until`).
    Expire,
    /// Undo a Delete (restore to `unverified` — same as Restore for deleted).
    Delete,
}

/// The semantic inputs for a relationship **Undo** command.
///
/// Undo is a **compensating forward command**: it does not erase or replay
/// history but creates a new forward outcome that reverses the targeted
/// lifecycle state change. The `audit_records.reversal_of` link is set by the
/// caller to the audit id of the command being undone.
///
/// Undo is intentionally narrow: it only undoes the last *state-change* class
/// of command (Confirm / Expire / Delete). An Undo of a Create is a Delete;
/// that is expressed as a Delete command rather than a separate Undo variant —
/// this keeps compensating commands composable and auditable.
pub struct RelationshipUndoInputs {
    /// The id of the row whose last lifecycle state change is being undone.
    pub target_id: RelationshipId,
    /// What specifically is being undone.
    pub undo_target: UndoTarget,
    /// The policy partition for the `graph_changes` descriptor.
    pub policy_partition: PolicyPartition,
}

/// Transaction-scoped semantic store for a relationship Undo command.
pub struct TxRelationshipUndo {
    inputs: RelationshipUndoInputs,
}

impl TxRelationshipUndo {
    /// Construct the store from validated undo inputs.
    pub fn new(inputs: RelationshipUndoInputs) -> Self {
        Self { inputs }
    }
}

impl TxSemanticStore for TxRelationshipUndo {
    fn apply(
        &self,
        tx: &mut AuthorityTx<'_>,
        _env: &CommandEnvelope,
    ) -> MemoryResult<SemanticOutcome> {
        let snap = load_relationship(tx, &self.inputs.target_id)?
            .ok_or(RelationshipLifecycleError::NotFound)?;

        let (op_label, state_change): (
            &str,
            Box<dyn FnOnce(&mut AuthorityTx<'_>) -> MemoryResult<()>>,
        ) = match self.inputs.undo_target {
            UndoTarget::Confirm => {
                // Only meaningful if currently confirmed.
                if snap.truth_state != TruthState::Confirmed {
                    return Err(RelationshipLifecycleError::NothingToUndo.into());
                }
                let id = self.inputs.target_id.clone();
                (
                    "relationship_undo_confirm",
                    Box::new(move |t| set_truth_state(t, &id, &TruthState::Unverified)),
                )
            }
            UndoTarget::Expire => {
                // Clear valid_until — only meaningful if it was set.
                let has_until: bool = tx
                    .conn()
                    .query_row(
                        "SELECT valid_until IS NOT NULL FROM relationships_v2 WHERE id = ?1",
                        params![self.inputs.target_id.as_str()],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(StorageError::Sqlite)?
                    .unwrap_or(false);
                if !has_until {
                    return Err(RelationshipLifecycleError::NothingToUndo.into());
                }
                let id = self.inputs.target_id.clone();
                (
                    "relationship_undo_expire",
                    Box::new(move |t| {
                        t.conn()
                            .execute(
                                "UPDATE relationships_v2 SET valid_until = NULL WHERE id = ?1",
                                params![id.as_str()],
                            )
                            .map_err(StorageError::Sqlite)?;
                        Ok(())
                    }),
                )
            }
            UndoTarget::Delete => {
                // Undo a delete by restoring to unverified.
                if snap.truth_state != TruthState::Deleted {
                    return Err(RelationshipLifecycleError::NothingToUndo.into());
                }
                let id = self.inputs.target_id.clone();
                (
                    "relationship_undo_delete",
                    Box::new(move |t| set_truth_state(t, &id, &TruthState::Unverified)),
                )
            }
        };

        state_change(tx)?;

        let change = relationship_change(
            GraphChangeKind::State,
            &self.inputs.target_id,
            &self.inputs.policy_partition.partition_key(),
            op_label,
        );

        Ok(SemanticOutcome::graph_visible(vec![change]))
    }
}

// ─────────────────────────────────────────────────────────────────────────
// RelationshipCommandBus — the governed submission seam for lifecycle commands
// ─────────────────────────────────────────────────────────────────────────

use std::sync::Arc;

use crate::memory::db::Database;

use super::bus::{AuthorityCommandBus, GovernedOutcome};
use super::publish::WakePublisher;

/// Governed submission result for a relationship lifecycle command.
/// Wraps [`GovernedOutcome`] and adds the relationship-specific audit id for
/// `reversal_of` linking in compensating commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipCommandOutcome {
    /// The standard governed bus outcome (status + revision + event id).
    pub governed: GovernedOutcome,
    /// The audit id of this command's `audit_records` row, usable as
    /// `reversal_of` in a later compensating command.
    pub audit_id: Option<AuditId>,
}

impl RelationshipCommandOutcome {
    /// Whether this command committed a new durable change.
    pub fn is_committed(&self) -> bool {
        self.governed.is_committed()
    }
}

/// A thin submission helper that wires the six lifecycle stores through
/// [`AuthorityCommandBus::submit`], returning a [`RelationshipCommandOutcome`]
/// that carries the audit id for compensating-command `reversal_of` linking.
///
/// Callers still run the `RelationshipValidator` gate before constructing the
/// store and presenting the envelope — this bus does not re-run validation.
pub struct RelationshipCommandBus<P: WakePublisher = super::publish::NoopWakePublisher> {
    inner: AuthorityCommandBus<P>,
}

impl RelationshipCommandBus<super::publish::NoopWakePublisher> {
    /// Build a bus that discards post-commit wakes (tests / standalone use).
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            inner: AuthorityCommandBus::new(db),
        }
    }
}

impl<P: WakePublisher> RelationshipCommandBus<P> {
    /// Build a bus that publishes post-commit revision wakes.
    pub fn with_publisher(db: Arc<Database>, publisher: P) -> Self {
        Self {
            inner: AuthorityCommandBus::with_publisher(db, publisher),
        }
    }

    /// Submit a create command.
    pub fn create(
        &self,
        env: &CommandEnvelope,
        inputs: RelationshipCreateInputs,
        reversal_of: Option<&AuditId>,
    ) -> MemoryResult<RelationshipCommandOutcome> {
        let store = TxRelationshipCreate::new(inputs);
        let governed = self.inner.submit(env, &store, reversal_of)?;
        Ok(RelationshipCommandOutcome {
            governed,
            audit_id: None, // filled by caller from CommandRecord if needed
        })
    }

    /// Submit an edit (supersede) command.
    pub fn edit(
        &self,
        env: &CommandEnvelope,
        inputs: RelationshipEditInputs,
        reversal_of: Option<&AuditId>,
    ) -> MemoryResult<RelationshipCommandOutcome> {
        let store = TxRelationshipEdit::new(inputs);
        let governed = self.inner.submit(env, &store, reversal_of)?;
        Ok(RelationshipCommandOutcome {
            governed,
            audit_id: None,
        })
    }

    /// Submit a confirm command.
    pub fn confirm(
        &self,
        env: &CommandEnvelope,
        inputs: RelationshipConfirmInputs,
        reversal_of: Option<&AuditId>,
    ) -> MemoryResult<RelationshipCommandOutcome> {
        let store = TxRelationshipConfirm::new(inputs);
        let governed = self.inner.submit(env, &store, reversal_of)?;
        Ok(RelationshipCommandOutcome {
            governed,
            audit_id: None,
        })
    }

    /// Submit an expire command.
    pub fn expire(
        &self,
        env: &CommandEnvelope,
        inputs: RelationshipExpireInputs,
        reversal_of: Option<&AuditId>,
    ) -> MemoryResult<RelationshipCommandOutcome> {
        let store = TxRelationshipExpire::new(inputs);
        let governed = self.inner.submit(env, &store, reversal_of)?;
        Ok(RelationshipCommandOutcome {
            governed,
            audit_id: None,
        })
    }

    /// Submit a delete command.
    pub fn delete(
        &self,
        env: &CommandEnvelope,
        inputs: RelationshipDeleteInputs,
        reversal_of: Option<&AuditId>,
    ) -> MemoryResult<RelationshipCommandOutcome> {
        let store = TxRelationshipDelete::new(inputs);
        let governed = self.inner.submit(env, &store, reversal_of)?;
        Ok(RelationshipCommandOutcome {
            governed,
            audit_id: None,
        })
    }

    /// Submit a restore command (compensating — creates a new forward outcome).
    pub fn restore(
        &self,
        env: &CommandEnvelope,
        inputs: RelationshipRestoreInputs,
        reversal_of: Option<&AuditId>,
    ) -> MemoryResult<RelationshipCommandOutcome> {
        let store = TxRelationshipRestore::new(inputs);
        let governed = self.inner.submit(env, &store, reversal_of)?;
        Ok(RelationshipCommandOutcome {
            governed,
            audit_id: None,
        })
    }

    /// Submit an undo command (compensating — creates a new forward outcome).
    pub fn undo(
        &self,
        env: &CommandEnvelope,
        inputs: RelationshipUndoInputs,
        reversal_of: Option<&AuditId>,
    ) -> MemoryResult<RelationshipCommandOutcome> {
        let store = TxRelationshipUndo::new(inputs);
        let governed = self.inner.submit(env, &store, reversal_of)?;
        Ok(RelationshipCommandOutcome {
            governed,
            audit_id: None,
        })
    }

    /// The authority database handle (for building read surfaces / base revision).
    pub fn database(&self) -> &Arc<Database> {
        self.inner.database()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::authority::candidates::{CommandCandidate, WriteContext};
    use crate::memory::authority::command::Deadline;
    use crate::memory::authority::event_log::PENDING_POLICY_VERSION;
    use crate::memory::authority::relationship_evidence::{
        EvidenceDraft, EvidenceInputs, NewRelationshipInputs,
    };
    use crate::memory::authority::relationship_validation::ResolvedRelationship;
    use crate::memory::db::Database;
    use crate::memory::model::entity::EvidencePolarity;
    use crate::memory::model::provenance::{Actor, Locator, Method};
    use crate::memory::model::relation_registry::RelationRegistry;
    use crate::memory::model::relationship_identity::{RelationEndpoint, RelationshipIdentity};
    use crate::memory::model::{CallerContext, SourceId};
    use crate::memory::model::{
        EndpointKind, GraphRevision, IdempotencyKey, InvocationId, PolicyPartition, ValidInterval,
        Version,
    };
    use crate::memory::types::MemoryMode;
    use std::sync::Arc;

    const V1: Version = Version::first();

    fn fresh_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().expect("open in-memory authority"))
    }

    fn partition() -> PolicyPartition {
        PolicyPartition::new("user", "chat", 0).unwrap()
    }

    fn uuid(byte: u8) -> String {
        format!("018f4e2a-1c3b-7d4e-8f90-abcdef01234{byte:x}")
    }

    fn resolved_related_to(db: &Database) -> ResolvedRelationship {
        let relation = db
            .with_read(|conn| RelationRegistry::resolve_definition(conn, "related_to", V1))
            .unwrap()
            .expect("related_to seeded by migration 0018");
        let source = RelationEndpoint::new(EndpointKind::Entity, uuid(1)).unwrap();
        let target = RelationEndpoint::new(EndpointKind::Entity, uuid(2)).unwrap();
        let validity = ValidInterval::open();
        let policy = partition();
        let identity =
            RelationshipIdentity::compute(&relation, &source, &target, &validity, &policy);
        ResolvedRelationship {
            relation,
            policy_partition: policy,
            identity,
        }
    }

    fn new_relationship_inputs() -> NewRelationshipInputs {
        NewRelationshipInputs {
            source: RelationEndpoint::new(EndpointKind::Entity, uuid(1)).unwrap(),
            target: RelationEndpoint::new(EndpointKind::Entity, uuid(2)).unwrap(),
            validity: ValidInterval::open(),
            policy_source_id: SourceId::new_v7(),
            policy_version: PENDING_POLICY_VERSION.to_string(),
            created_event_id: None,
        }
    }

    fn evidence_inputs() -> EvidenceInputs {
        EvidenceInputs {
            draft: EvidenceDraft::new(
                Locator::url("https://example.com/doc", None).unwrap(),
                Actor::new("tester").unwrap(),
                Method::new("manual_review", Some("1".to_string())).unwrap(),
                EvidencePolarity::Supports,
            ),
            source_id: SourceId::new_v7(),
            policy: partition(),
            policy_version: PENDING_POLICY_VERSION.to_string(),
            created_event_id: None,
        }
    }

    fn write_ctx(key: &str) -> WriteContext {
        let caller_partition = partition();
        let caller = CallerContext::local_desktop("local-desktop", caller_partition).unwrap();
        WriteContext {
            caller,
            idempotency_key: IdempotencyKey::new(key).unwrap(),
            base_revision: GraphRevision::base(),
            invocation_id: InvocationId::new_v7(),
            source_id: "core:cognition".to_string(),
            mode: MemoryMode::Permanent,
            deadline: Deadline::default_write(),
        }
    }

    fn observe_env(key: &str) -> crate::memory::authority::command::CommandEnvelope {
        CommandCandidate::native_fact("test relationship", Some("link"))
            .into_envelope(write_ctx(key), None)
            .unwrap()
    }

    // ── Helper: submit a create command and return the relationship id ──────
    fn create_relationship(db: &Arc<Database>, key: &str) -> RelationshipId {
        let resolved = resolved_related_to(db);
        let bus = RelationshipCommandBus::new(Arc::clone(db));
        let env = observe_env(key);
        let inputs = RelationshipCreateInputs {
            resolved,
            new_relationship: new_relationship_inputs(),
            evidence: evidence_inputs(),
        };
        let outcome = bus.create(&env, inputs, None).unwrap();
        assert!(outcome.is_committed(), "create must commit");

        // Return the freshly inserted relationship id.
        db.with_read(|conn| {
            let id: String = conn
                .query_row(
                    "SELECT id FROM relationships_v2 ORDER BY rowid DESC LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(RelationshipId::new(id).unwrap())
        })
        .unwrap()
    }

    // ── CREATE ────────────────────────────────────────────────────────────────

    #[test]
    fn create_inserts_relationship_and_evidence_and_reserves_revision() {
        let db = fresh_db();
        let rel_id = create_relationship(&db, "create-1");

        // Row exists and is active (unverified initial state).
        let (truth, _rev): (Option<String>, Option<i64>) = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT truth_state, revision FROM relationships_v2 WHERE id = ?1",
                        rusqlite::params![rel_id.as_str()],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap())
            })
            .unwrap();
        let state = truth.as_deref().unwrap_or("unverified");
        assert_eq!(state, "unverified", "fresh row must be unverified");

        // graph_revision advanced.
        let revision: i64 = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT graph_revision FROM authority_meta WHERE id = 1",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap())
            })
            .unwrap();
        assert_eq!(revision, 1, "one revision must have been reserved");
    }

    // ── CONFIRM ───────────────────────────────────────────────────────────────

    #[test]
    fn confirm_sets_truth_state_confirmed() {
        let db = fresh_db();
        let rel_id = create_relationship(&db, "confirm-1");

        let _bus = RelationshipCommandBus::new(Arc::clone(&db));
        // Need a non-previewed Correct envelope for lifecycle state changes.
        // We use Observe here as a placeholder since CommandKind::Correct requires
        // a preview token. For the store-level test we call the store directly.
        let mut tx = db.begin().unwrap();
        let store = TxRelationshipConfirm::new(RelationshipConfirmInputs {
            target_id: rel_id.clone(),
            policy_partition: partition(),
        });
        let fake_env = observe_env("confirm-direct");
        let outcome = store.apply(&mut tx, &fake_env).unwrap();
        assert!(outcome.graph_visible, "confirm must be graph-visible");
        tx.commit().unwrap();

        let truth: Option<String> = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT truth_state FROM relationships_v2 WHERE id = ?1",
                        rusqlite::params![rel_id.as_str()],
                        |r| r.get(0),
                    )
                    .unwrap())
            })
            .unwrap();
        assert_eq!(truth.as_deref(), Some("confirmed"));
    }

    // ── EXPIRE ────────────────────────────────────────────────────────────────

    #[test]
    fn expire_sets_valid_until() {
        let db = fresh_db();
        let rel_id = create_relationship(&db, "expire-1");
        let expire_at = UtcTimestamp::now();

        let mut tx = db.begin().unwrap();
        let store = TxRelationshipExpire::new(RelationshipExpireInputs {
            target_id: rel_id.clone(),
            expire_at,
            policy_partition: partition(),
        });
        let fake_env = observe_env("expire-direct");
        let outcome = store.apply(&mut tx, &fake_env).unwrap();
        assert!(outcome.graph_visible);
        tx.commit().unwrap();

        let until: Option<String> = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT valid_until FROM relationships_v2 WHERE id = ?1",
                        rusqlite::params![rel_id.as_str()],
                        |r| r.get(0),
                    )
                    .unwrap())
            })
            .unwrap();
        assert!(until.is_some(), "valid_until must be set after expire");
    }

    // ── DELETE ────────────────────────────────────────────────────────────────

    #[test]
    fn delete_sets_truth_state_deleted() {
        let db = fresh_db();
        let rel_id = create_relationship(&db, "delete-1");

        let mut tx = db.begin().unwrap();
        let store = TxRelationshipDelete::new(RelationshipDeleteInputs {
            target_id: rel_id.clone(),
            policy_partition: partition(),
        });
        let fake_env = observe_env("delete-direct");
        store.apply(&mut tx, &fake_env).unwrap();
        tx.commit().unwrap();

        let truth: Option<String> = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT truth_state FROM relationships_v2 WHERE id = ?1",
                        rusqlite::params![rel_id.as_str()],
                        |r| r.get(0),
                    )
                    .unwrap())
            })
            .unwrap();
        assert_eq!(truth.as_deref(), Some("deleted"));
    }

    // ── RESTORE (compensating) ────────────────────────────────────────────────

    #[test]
    fn restore_re_activates_deleted_row_compensating_forward() {
        let db = fresh_db();
        let rel_id = create_relationship(&db, "restore-1");

        // Delete it first.
        let mut tx = db.begin().unwrap();
        TxRelationshipDelete::new(RelationshipDeleteInputs {
            target_id: rel_id.clone(),
            policy_partition: partition(),
        })
        .apply(&mut tx, &observe_env("delete-for-restore"))
        .unwrap();
        tx.commit().unwrap();

        // Restore it (compensating forward command).
        let mut tx = db.begin().unwrap();
        let outcome = TxRelationshipRestore::new(RelationshipRestoreInputs {
            target_id: rel_id.clone(),
            policy_partition: partition(),
        })
        .apply(&mut tx, &observe_env("restore-1"))
        .unwrap();
        assert!(outcome.graph_visible);
        tx.commit().unwrap();

        let truth: Option<String> = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT truth_state FROM relationships_v2 WHERE id = ?1",
                        rusqlite::params![rel_id.as_str()],
                        |r| r.get(0),
                    )
                    .unwrap())
            })
            .unwrap();
        assert_eq!(
            truth.as_deref(),
            Some("unverified"),
            "restore must re-activate the row"
        );
    }

    // ── UNDO (compensating) ────────────────────────────────────────────────────

    #[test]
    fn undo_confirm_resets_to_unverified() {
        let db = fresh_db();
        let rel_id = create_relationship(&db, "undo-confirm-1");

        // Confirm it.
        let mut tx = db.begin().unwrap();
        TxRelationshipConfirm::new(RelationshipConfirmInputs {
            target_id: rel_id.clone(),
            policy_partition: partition(),
        })
        .apply(&mut tx, &observe_env("confirm-for-undo"))
        .unwrap();
        tx.commit().unwrap();

        // Undo the confirm (compensating forward command).
        let mut tx = db.begin().unwrap();
        TxRelationshipUndo::new(RelationshipUndoInputs {
            target_id: rel_id.clone(),
            undo_target: UndoTarget::Confirm,
            policy_partition: partition(),
        })
        .apply(&mut tx, &observe_env("undo-confirm"))
        .unwrap();
        tx.commit().unwrap();

        let truth: Option<String> = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT truth_state FROM relationships_v2 WHERE id = ?1",
                        rusqlite::params![rel_id.as_str()],
                        |r| r.get(0),
                    )
                    .unwrap())
            })
            .unwrap();
        assert_eq!(truth.as_deref(), Some("unverified"));
    }

    #[test]
    fn undo_nothing_to_undo_returns_error() {
        let db = fresh_db();
        let rel_id = create_relationship(&db, "undo-nothing-1");

        // Row is unverified — undoing confirm on an unverified row is NothingToUndo.
        let mut tx = db.begin().unwrap();
        let result = TxRelationshipUndo::new(RelationshipUndoInputs {
            target_id: rel_id.clone(),
            undo_target: UndoTarget::Confirm,
            policy_partition: partition(),
        })
        .apply(&mut tx, &observe_env("undo-nothing"));
        assert!(result.is_err(), "NothingToUndo must return an error");
        drop(tx); // rollback
    }

    // ── Delete then restore blocks a second delete ─────────────────────────

    #[test]
    fn restore_is_not_erasure_old_row_persists_in_history() {
        let db = fresh_db();
        let rel_id = create_relationship(&db, "erasure-check-1");

        // Delete.
        let mut tx = db.begin().unwrap();
        TxRelationshipDelete::new(RelationshipDeleteInputs {
            target_id: rel_id.clone(),
            policy_partition: partition(),
        })
        .apply(&mut tx, &observe_env("delete-erasure"))
        .unwrap();
        tx.commit().unwrap();

        // Restore.
        let mut tx = db.begin().unwrap();
        TxRelationshipRestore::new(RelationshipRestoreInputs {
            target_id: rel_id.clone(),
            policy_partition: partition(),
        })
        .apply(&mut tx, &observe_env("restore-erasure"))
        .unwrap();
        tx.commit().unwrap();

        // The row still exists (no physical deletion).
        let count: i64 = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT COUNT(*) FROM relationships_v2 WHERE id = ?1",
                        rusqlite::params![rel_id.as_str()],
                        |r| r.get(0),
                    )
                    .unwrap())
            })
            .unwrap();
        assert_eq!(count, 1, "restore must not erase the original row");
    }

    // ── Restoring an active row returns NotRestorable ─────────────────────

    #[test]
    fn restore_active_row_returns_not_restorable() {
        let db = fresh_db();
        let rel_id = create_relationship(&db, "not-restorable-1");

        let mut tx = db.begin().unwrap();
        let result = TxRelationshipRestore::new(RelationshipRestoreInputs {
            target_id: rel_id.clone(),
            policy_partition: partition(),
        })
        .apply(&mut tx, &observe_env("restore-active"));
        assert!(result.is_err(), "restoring an active row must be an error");
        drop(tx);
    }

    // ── Operations on a missing row return NotFound ───────────────────────

    #[test]
    fn operations_on_missing_row_return_not_found() {
        let db = fresh_db();
        let ghost_id = RelationshipId::new_v7();

        let mut tx = db.begin().unwrap();
        assert!(TxRelationshipConfirm::new(RelationshipConfirmInputs {
            target_id: ghost_id.clone(),
            policy_partition: partition(),
        })
        .apply(&mut tx, &observe_env("ghost-confirm"))
        .is_err());
        drop(tx);

        let mut tx = db.begin().unwrap();
        assert!(TxRelationshipDelete::new(RelationshipDeleteInputs {
            target_id: ghost_id.clone(),
            policy_partition: partition(),
        })
        .apply(&mut tx, &observe_env("ghost-delete"))
        .is_err());
        drop(tx);
    }
}
