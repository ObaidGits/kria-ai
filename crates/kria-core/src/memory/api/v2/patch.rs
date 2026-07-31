//! Authority patch types for the memory v2 API (design §5.2, MGR-008, F3.9).
//!
//! ## Design contract (design §5.2)
//!
//! After every authority commit, the system emits a bounded patch that lets
//! clients converge their cached snapshot to the new revision without a
//! full-corpus reload:
//!
//! ```text
//! Patch { baseRevision, targetRevision, changes[], invalidations[], recoveryCursor }
//! ```
//!
//! Apply rules:
//! - `client_revision == base_revision` → **Apply** the patch atomically.
//! - `client_revision == target_revision` → **Ignore** (duplicate, already at target).
//! - `client_revision > target_revision` → **Ignore** (client is ahead / stale patch).
//! - `client_revision != base_revision` and none of the above → **Ignore(Diverged)**;
//!   the caller must perform a bounded active-query refetch.
//!
//! Gap / refilter / schema / policy changes emit a `RefetchRequired` result
//! carrying the query hash the client must re-execute (design §5.2 §8.2).
//!
//! ## Retention
//!
//! Patches are retained for at most [`PatchRetentionPolicy::MAX_REVISIONS`]
//! revisions **or** [`PatchRetentionPolicy::MAX_AGE_SECS`] seconds (7 days),
//! whichever limit is reached first (MGR-008).

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// ChangeKind
// ─────────────────────────────────────────────────────────────────────────────

/// The kind of authority change recorded in a single [`PatchEntry`].
///
/// Serializes as `snake_case` to match the `graph_changes.change_kind` column
/// values defined in the schema (design §4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// A new record was inserted into the authority store.
    Insert,
    /// An existing record's fields were updated in-place.
    Update,
    /// An existing record underwent a truth-state or lifecycle transition
    /// (e.g. `Current` → `Superseded`, `Active` → `Forgotten`).
    StateChange,
    /// A record was hard-deleted from the authority store.
    Delete,
    /// A record's derived indexes or caches must be invalidated; the authority
    /// row may still exist (e.g. after a policy change).
    Invalidate,
}

// ─────────────────────────────────────────────────────────────────────────────
// PatchEntry
// ─────────────────────────────────────────────────────────────────────────────

/// A single change record within an [`AuthorityPatch`].
///
/// Mirrors the `graph_changes` table row (design §4.1) but uses
/// JSON-serializable `String` fields so it crosses the Tauri IPC bridge
/// and Axum JSON layer without conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchEntry {
    /// Kind of cognitive record affected (e.g. `"memory"`, `"entity"`,
    /// `"relationship"`, `"evidence"`, …). Matches `record_kind` in
    /// `graph_changes`.
    pub record_kind: String,

    /// Stable opaque authority record identifier. Matches `record_id` in
    /// `graph_changes`. Never empty.
    pub record_id: String,

    /// The kind of change that was applied to this record.
    pub change_kind: ChangeKind,

    /// SHA-256 hex hash of the record content **before** this change. `None`
    /// for `Insert` (no prior content) or when the before-state is unavailable.
    pub before_hash: Option<String>,

    /// SHA-256 hex hash of the record content **after** this change. `None`
    /// for `Delete` (no remaining content) or when the after-state is
    /// unavailable (e.g. `Invalidate` entries).
    pub after_hash: Option<String>,

    /// Policy partition key for this record. Clients use this to decide
    /// whether the change is relevant to their active query's scope/sensitivity
    /// filter without exposing hidden record content.
    pub policy_partition: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// AuthorityPatch
// ─────────────────────────────────────────────────────────────────────────────

/// A bounded base→target patch emitted after each authority commit (design §5.2).
///
/// The client applies the patch only when its cached revision equals
/// `base_revision`. All other cases are handled by [`PatchValidator::apply`].
///
/// This type is the wire-format contract between the authority layer and all
/// consuming clients (Tauri adapter, Axum adapter, tests). It must remain
/// serializable across the Tauri IPC bridge and JSON APIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityPatch {
    /// The authority revision this patch starts from. The patch is only
    /// applicable when the client's cached revision equals this value.
    pub base_revision: i64,

    /// The authority revision this patch advances the client to.
    pub target_revision: i64,

    /// Ordered list of authority changes in commit order. Clients apply these
    /// in sequence; the order is deterministic and stable across retransmits.
    pub changes: Vec<PatchEntry>,

    /// Record IDs that require cache invalidation on the client side. These
    /// may overlap with `changes` (e.g. an `Invalidate` entry also appears
    /// here) or be additional records affected by policy or schema changes.
    pub invalidations: Vec<String>,

    /// Cursor for a bounded active-query refetch when the client encounters a
    /// gap, refilter, schema change, or policy change and cannot apply the
    /// patch directly. `None` when a simple apply or ignore is sufficient.
    ///
    /// The cursor format is the same HMAC-authenticated format defined in
    /// [`super::cursor::CursorPayload`] (design §5.2).
    pub recovery_cursor: Option<String>,

    /// Schema version active at the time this patch was emitted. Clients that
    /// do not recognize this version must perform a full bounded refetch.
    pub schema_version: String,

    /// Hash of the effective policy at the time this patch was emitted.
    /// Clients whose cached policy hash differs must perform a bounded refetch
    /// rather than applying the patch directly.
    pub policy_hash: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// IgnoreReason
// ─────────────────────────────────────────────────────────────────────────────

/// Reason a patch was safely ignored by [`PatchValidator::apply`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IgnoreReason {
    /// The client is already at the patch's `target_revision`; the patch has
    /// already been applied (or an identical patch was delivered twice).
    Duplicate,

    /// The client's revision is **ahead** of the patch's `target_revision`;
    /// the patch describes an older transition the client has already passed.
    Stale,

    /// The client's revision is neither `base_revision` nor `target_revision`;
    /// the client has diverged and must perform a bounded refetch.
    Diverged,
}

// ─────────────────────────────────────────────────────────────────────────────
// PatchApplyResult
// ─────────────────────────────────────────────────────────────────────────────

/// Result of attempting to apply an [`AuthorityPatch`] via
/// [`PatchValidator::apply`].
///
/// Clients act on the variant:
/// - [`Applied`] — commit the patch to the local snapshot.
/// - [`Ignored`] — discard the patch; no state change required.
/// - [`RefetchRequired`] — re-execute the active query from scratch using the
///   embedded `query_hash` to find the right entry point.
///
/// [`Applied`]: PatchApplyResult::Applied
/// [`Ignored`]: PatchApplyResult::Ignored
/// [`RefetchRequired`]: PatchApplyResult::RefetchRequired
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PatchApplyResult {
    /// The patch was successfully applied. The client should update its
    /// cached revision to `new_revision`.
    Applied {
        /// The authority revision the client has now advanced to (equals
        /// `patch.target_revision`).
        new_revision: i64,
        /// Number of [`PatchEntry`] items that were applied.
        changes_count: usize,
    },

    /// The patch was safely discarded; no client state change is required.
    Ignored {
        /// Why the patch was discarded.
        reason: IgnoreReason,
    },

    /// A gap, refilter, schema change, or policy change means the patch cannot
    /// be applied directly. The client must re-execute its active query.
    RefetchRequired {
        /// Hash of the query that must be re-executed (matches
        /// `patch.recovery_cursor` semantics, design §5.2).
        query_hash: String,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// PatchRetentionPolicy
// ─────────────────────────────────────────────────────────────────────────────

/// Patch retention limits (MGR-008).
///
/// Patches must be retained for **at most** `MAX_REVISIONS` revisions **or**
/// `MAX_AGE_SECS` seconds, whichever limit is reached first.
pub struct PatchRetentionPolicy;

impl PatchRetentionPolicy {
    /// Maximum number of revisions to retain patches for.
    pub const MAX_REVISIONS: u64 = 10_000;

    /// Maximum age in seconds to retain patches for (7 days).
    pub const MAX_AGE_SECS: u64 = 7 * 24 * 3_600;

    /// Returns `true` when a patch is still within the retention window.
    ///
    /// A patch is within retention if **both** conditions hold:
    /// - `current_revision - patch_revision <= MAX_REVISIONS`
    /// - `patch_age_secs <= MAX_AGE_SECS`
    ///
    /// Either limit being exceeded makes the patch eligible for eviction.
    ///
    /// # Arguments
    ///
    /// - `patch_revision` — the `target_revision` of the patch to test.
    /// - `current_revision` — the current authority graph revision.
    /// - `patch_age_secs` — how many seconds old the patch is.
    ///
    /// # Panics
    ///
    /// Does not panic. If `patch_revision > current_revision` (which should
    /// not occur in normal operation) the revision gap is treated as 0.
    pub fn is_within_retention(
        patch_revision: i64,
        current_revision: i64,
        patch_age_secs: u64,
    ) -> bool {
        // When patch_revision > current_revision (should not happen normally),
        // treat the gap as 0 (the patch is "newer" than current, so definitely
        // within retention by revision count).
        let revision_gap: u64 = if current_revision >= patch_revision {
            (current_revision - patch_revision) as u64
        } else {
            0
        };
        revision_gap <= Self::MAX_REVISIONS && patch_age_secs <= Self::MAX_AGE_SECS
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PatchValidator
// ─────────────────────────────────────────────────────────────────────────────

/// Stateless patch-application logic (design §5.2, MGR-008).
///
/// `PatchValidator::apply` is the single decision point for whether a patch
/// should be applied, ignored, or trigger a refetch. It contains no I/O and
/// holds no state; it operates purely on the client revision and the patch.
pub struct PatchValidator;

impl PatchValidator {
    /// Decide what a client should do with `patch` given its current
    /// `client_revision`.
    ///
    /// Decision table (design §5.2):
    ///
    /// | Condition | Result |
    /// |---|---|
    /// | `client == patch.target_revision` | `Ignored(Duplicate)` |
    /// | `client > patch.target_revision` | `Ignored(Stale)` |
    /// | `client != patch.base_revision` | `Ignored(Diverged)` |
    /// | `client == patch.base_revision` | `Applied { … }` |
    ///
    /// The order of checks matches the table above (duplicate and stale are
    /// detected before diverged so they are reported accurately).
    pub fn apply(client_revision: i64, patch: &AuthorityPatch) -> PatchApplyResult {
        // ── 1. Duplicate: client is already at the target revision ─────────
        if client_revision == patch.target_revision {
            return PatchApplyResult::Ignored {
                reason: IgnoreReason::Duplicate,
            };
        }

        // ── 2. Stale: client is ahead of the patch's target revision ───────
        if client_revision > patch.target_revision {
            return PatchApplyResult::Ignored {
                reason: IgnoreReason::Stale,
            };
        }

        // ── 3. Diverged: client is not at base (and not already caught above)
        if client_revision != patch.base_revision {
            return PatchApplyResult::Ignored {
                reason: IgnoreReason::Diverged,
            };
        }

        // ── 4. Happy path: client is exactly at base revision ──────────────
        PatchApplyResult::Applied {
            new_revision: patch.target_revision,
            changes_count: patch.changes.len(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Build a minimal `AuthorityPatch` with no changes or invalidations.
    fn make_patch(base: i64, target: i64) -> AuthorityPatch {
        AuthorityPatch {
            base_revision: base,
            target_revision: target,
            changes: vec![],
            invalidations: vec![],
            recovery_cursor: None,
            schema_version: "2.0".to_string(),
            policy_hash: "test-policy-hash".to_string(),
        }
    }

    /// Build an `AuthorityPatch` with one change entry.
    fn make_patch_with_change(base: i64, target: i64) -> AuthorityPatch {
        let entry = PatchEntry {
            record_kind: "memory".to_string(),
            record_id: "rec-001".to_string(),
            change_kind: ChangeKind::Update,
            before_hash: Some("before".to_string()),
            after_hash: Some("after".to_string()),
            policy_partition: "personal:default".to_string(),
        };
        AuthorityPatch {
            base_revision: base,
            target_revision: target,
            changes: vec![entry],
            invalidations: vec!["rec-001".to_string()],
            recovery_cursor: None,
            schema_version: "2.0".to_string(),
            policy_hash: "pol-abc".to_string(),
        }
    }

    // ── PatchValidator: happy path ────────────────────────────────────────────

    #[test]
    fn apply_happy_path_client_at_base_returns_applied() {
        let patch = make_patch_with_change(10, 11);
        let result = PatchValidator::apply(10, &patch);
        assert_eq!(
            result,
            PatchApplyResult::Applied {
                new_revision: 11,
                changes_count: 1,
            }
        );
    }

    #[test]
    fn apply_happy_path_advances_to_target_revision() {
        let patch = make_patch(100, 105);
        let result = PatchValidator::apply(100, &patch);
        match result {
            PatchApplyResult::Applied { new_revision, .. } => {
                assert_eq!(new_revision, 105);
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn apply_happy_path_empty_changes_still_applies() {
        let patch = make_patch(7, 8);
        let result = PatchValidator::apply(7, &patch);
        assert_eq!(
            result,
            PatchApplyResult::Applied {
                new_revision: 8,
                changes_count: 0,
            }
        );
    }

    // ── PatchValidator: duplicate ─────────────────────────────────────────────

    #[test]
    fn apply_duplicate_client_already_at_target_returns_ignored_duplicate() {
        let patch = make_patch(9, 10);
        let result = PatchValidator::apply(10, &patch); // client is already at target
        assert_eq!(
            result,
            PatchApplyResult::Ignored {
                reason: IgnoreReason::Duplicate,
            }
        );
    }

    // ── PatchValidator: stale ─────────────────────────────────────────────────

    #[test]
    fn apply_stale_client_ahead_of_target_returns_ignored_stale() {
        let patch = make_patch(5, 8);
        let result = PatchValidator::apply(9, &patch); // client is ahead of target
        assert_eq!(
            result,
            PatchApplyResult::Ignored {
                reason: IgnoreReason::Stale,
            }
        );
    }

    #[test]
    fn apply_stale_client_exactly_one_ahead_returns_ignored_stale() {
        let patch = make_patch(3, 4);
        let result = PatchValidator::apply(5, &patch); // client at 5, target at 4
        assert_eq!(
            result,
            PatchApplyResult::Ignored {
                reason: IgnoreReason::Stale,
            }
        );
    }

    // ── PatchValidator: diverged ──────────────────────────────────────────────

    #[test]
    fn apply_diverged_client_at_neither_base_nor_target_returns_ignored_diverged() {
        let patch = make_patch(10, 12); // base=10, target=12
        let result = PatchValidator::apply(5, &patch); // client at 5 — neither base nor target
        assert_eq!(
            result,
            PatchApplyResult::Ignored {
                reason: IgnoreReason::Diverged,
            }
        );
    }

    #[test]
    fn apply_diverged_client_between_base_and_target_returns_ignored_diverged() {
        let patch = make_patch(10, 15);
        let result = PatchValidator::apply(12, &patch); // between base and target but not at base
        assert_eq!(
            result,
            PatchApplyResult::Ignored {
                reason: IgnoreReason::Diverged,
            }
        );
    }

    // ── PatchRetentionPolicy: boundary tests ──────────────────────────────────

    #[test]
    fn retention_within_exactly_max_revisions_is_true() {
        // Exactly at the boundary: should be within retention.
        assert!(PatchRetentionPolicy::is_within_retention(
            0,
            PatchRetentionPolicy::MAX_REVISIONS as i64,
            0
        ));
    }

    #[test]
    fn retention_one_over_max_revisions_is_false() {
        let gap = PatchRetentionPolicy::MAX_REVISIONS as i64 + 1;
        assert!(!PatchRetentionPolicy::is_within_retention(0, gap, 0));
    }

    #[test]
    fn retention_within_exactly_max_age_secs_is_true() {
        // Exactly at the age boundary (7 days): should be within retention.
        assert!(PatchRetentionPolicy::is_within_retention(
            1,
            1,
            PatchRetentionPolicy::MAX_AGE_SECS
        ));
    }

    #[test]
    fn retention_one_second_over_max_age_is_false() {
        assert!(!PatchRetentionPolicy::is_within_retention(
            1,
            1,
            PatchRetentionPolicy::MAX_AGE_SECS + 1
        ));
    }

    #[test]
    fn retention_both_limits_satisfied_is_true() {
        assert!(PatchRetentionPolicy::is_within_retention(500, 1000, 3600));
    }

    #[test]
    fn retention_max_revisions_const_is_10000() {
        assert_eq!(PatchRetentionPolicy::MAX_REVISIONS, 10_000);
    }

    #[test]
    fn retention_max_age_secs_const_is_7_days() {
        assert_eq!(PatchRetentionPolicy::MAX_AGE_SECS, 7 * 24 * 3_600);
    }

    #[test]
    fn retention_future_patch_revision_treated_as_gap_zero() {
        // patch_revision > current_revision is unusual but must not panic;
        // saturating_sub treats the gap as 0.
        assert!(PatchRetentionPolicy::is_within_retention(999, 1, 0));
    }

    // ── PatchEntry: JSON round-trip ───────────────────────────────────────────

    #[test]
    fn patch_entry_round_trips_json() {
        let entry = PatchEntry {
            record_kind: "entity".to_string(),
            record_id: "ent-abc".to_string(),
            change_kind: ChangeKind::Insert,
            before_hash: None,
            after_hash: Some("sha256:deadbeef".to_string()),
            policy_partition: "work:default".to_string(),
        };
        let json = serde_json::to_string(&entry).expect("serializes");
        let back: PatchEntry = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(entry, back);
    }

    #[test]
    fn patch_entry_with_all_optional_fields_round_trips_json() {
        let entry = PatchEntry {
            record_kind: "relationship".to_string(),
            record_id: "rel-xyz".to_string(),
            change_kind: ChangeKind::StateChange,
            before_hash: Some("sha256:aabbcc".to_string()),
            after_hash: Some("sha256:ddeeff".to_string()),
            policy_partition: "personal:sensitive".to_string(),
        };
        let json = serde_json::to_value(&entry).expect("serializes");
        let back: PatchEntry = serde_json::from_value(json).expect("deserializes");
        assert_eq!(entry, back);
    }

    // ── ChangeKind: snake_case serialization ──────────────────────────────────

    #[test]
    fn change_kind_insert_serializes_as_snake_case() {
        let json = serde_json::to_value(&ChangeKind::Insert).expect("serializes");
        assert_eq!(json, "insert");
    }

    #[test]
    fn change_kind_update_serializes_as_snake_case() {
        let json = serde_json::to_value(&ChangeKind::Update).expect("serializes");
        assert_eq!(json, "update");
    }

    #[test]
    fn change_kind_state_change_serializes_as_snake_case() {
        let json = serde_json::to_value(&ChangeKind::StateChange).expect("serializes");
        assert_eq!(json, "state_change");
    }

    #[test]
    fn change_kind_delete_serializes_as_snake_case() {
        let json = serde_json::to_value(&ChangeKind::Delete).expect("serializes");
        assert_eq!(json, "delete");
    }

    #[test]
    fn change_kind_invalidate_serializes_as_snake_case() {
        let json = serde_json::to_value(&ChangeKind::Invalidate).expect("serializes");
        assert_eq!(json, "invalidate");
    }

    #[test]
    fn all_change_kinds_round_trip_json() {
        let kinds = [
            ChangeKind::Insert,
            ChangeKind::Update,
            ChangeKind::StateChange,
            ChangeKind::Delete,
            ChangeKind::Invalidate,
        ];
        for kind in &kinds {
            let json = serde_json::to_value(kind).expect("serializes");
            let back: ChangeKind = serde_json::from_value(json).expect("deserializes");
            assert_eq!(kind, &back);
        }
    }

    // ── AuthorityPatch: JSON round-trip ───────────────────────────────────────

    #[test]
    fn authority_patch_round_trips_json() {
        let patch = make_patch_with_change(42, 43);
        let json = serde_json::to_string(&patch).expect("serializes");
        let back: AuthorityPatch = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(patch, back);
    }

    #[test]
    fn authority_patch_with_recovery_cursor_round_trips_json() {
        let mut patch = make_patch(1, 2);
        patch.recovery_cursor = Some("cursor-opaque-token".to_string());
        let json = serde_json::to_value(&patch).expect("serializes");
        let back: AuthorityPatch = serde_json::from_value(json).expect("deserializes");
        assert_eq!(patch, back);
    }

    // ── PatchApplyResult: JSON round-trip ─────────────────────────────────────

    #[test]
    fn patch_apply_result_applied_round_trips_json() {
        let result = PatchApplyResult::Applied {
            new_revision: 11,
            changes_count: 3,
        };
        let json = serde_json::to_value(&result).expect("serializes");
        assert_eq!(json["kind"], "applied");
        let back: PatchApplyResult = serde_json::from_value(json).expect("deserializes");
        assert_eq!(result, back);
    }

    #[test]
    fn patch_apply_result_ignored_round_trips_json() {
        let result = PatchApplyResult::Ignored {
            reason: IgnoreReason::Duplicate,
        };
        let json = serde_json::to_value(&result).expect("serializes");
        assert_eq!(json["kind"], "ignored");
        let back: PatchApplyResult = serde_json::from_value(json).expect("deserializes");
        assert_eq!(result, back);
    }

    #[test]
    fn patch_apply_result_refetch_required_round_trips_json() {
        let result = PatchApplyResult::RefetchRequired {
            query_hash: "qhash-abc123".to_string(),
        };
        let json = serde_json::to_value(&result).expect("serializes");
        assert_eq!(json["kind"], "refetch_required");
        let back: PatchApplyResult = serde_json::from_value(json).expect("deserializes");
        assert_eq!(result, back);
    }

    // ── IgnoreReason: serialization ───────────────────────────────────────────

    #[test]
    fn ignore_reason_duplicate_serializes_as_snake_case() {
        let json = serde_json::to_value(&IgnoreReason::Duplicate).expect("serializes");
        assert_eq!(json, "duplicate");
    }

    #[test]
    fn ignore_reason_stale_serializes_as_snake_case() {
        let json = serde_json::to_value(&IgnoreReason::Stale).expect("serializes");
        assert_eq!(json, "stale");
    }

    #[test]
    fn ignore_reason_diverged_serializes_as_snake_case() {
        let json = serde_json::to_value(&IgnoreReason::Diverged).expect("serializes");
        assert_eq!(json, "diverged");
    }
}
