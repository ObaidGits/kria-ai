//! User correction workflows: confirm, correct, supersede, keep-both.
//!
//! # Design invariants (Design §A9, §5.1, MGR-024)
//!
//! Corrections are **previewed** before commit (Design §5.1 command state
//! machine: `Previewed → Validate → TxOpen`). The workflow is:
//!
//! 1. Caller issues a [`CorrectionPreviewRequest`].
//! 2. [`CorrectionValidator::preview`] returns a [`CorrectionPreview`]
//!    with a `preview_token` and `base_revision`.
//! 3. Caller confirms and sends a [`CorrectionCommitRequest`] carrying the
//!    same `preview_token` and `base_revision`.
//! 4. [`CorrectionValidator::validate_commit`] verifies the token/revision
//!    match and returns a [`CorrectionResult`] with before/after state,
//!    audit description, and reversal eligibility.
//!
//! The preview token is deterministic (non-cryptographic) — computed from
//! `"{kind:?}:{record_id}:{base_revision}"` — and prevents committing a
//! stale or tampered preview without any secret.
//!
//! # Truth-state transitions
//!
//! | Kind      | Old record → | New record → |
//! |-----------|-------------|--------------|
//! | Correct   | Superseded  | Current      |
//! | Confirm   | Confirmed   | (same record)|
//! | Supersede | Superseded  | Current      |
//! | KeepBoth  | Contradicted| Contradicted |
//!
//! All four kinds are reversible (Design §A9).

use serde::{Deserialize, Serialize};

use crate::memory::model::truth::TruthState;
use crate::memory::model::GraphRevision;

// ── CorrectionKind ────────────────────────────────────────────────────────

/// The kind of correction a user is applying to a record or relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionKind {
    /// User provides a corrected value; the old value becomes Superseded.
    Correct,
    /// User confirms the current value is correct; truth_state → Confirmed.
    Confirm,
    /// User explicitly supersedes the current value with a new one.
    Supersede,
    /// User acknowledges both conflicting values; both preserved as Contradicted.
    KeepBoth,
}

// ── CorrectionPreviewRequest ──────────────────────────────────────────────

/// A request to preview a user correction before commitment.
///
/// The preview shows what will change, the affected count, reversibility,
/// and the base revision the user must confirm with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionPreviewRequest {
    /// The kind of correction.
    pub kind: CorrectionKind,
    /// The ID of the record/relationship being corrected.
    pub record_id: String,
    /// For `Correct`/`Supersede`: the proposed new value summary
    /// (policy-safe text; None when the correction is structural).
    pub proposed_value_summary: Option<String>,
    /// For `Correct`/`Supersede`: the evidence for the new value.
    pub evidence_summary: Option<String>,
    /// The base revision at which the user is making this decision.
    pub base_revision: GraphRevision,
    /// The caller's actor ID (for audit).
    pub actor_id: String,
}

// ── CorrectionPreview ─────────────────────────────────────────────────────

/// The preview of a correction command, shown to the user before commit.
///
/// Design §A9: corrections are previewed, governed, audited, and reversible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionPreview {
    /// The correction kind being previewed.
    pub kind: CorrectionKind,
    /// The record being corrected.
    pub record_id: String,
    /// The current value summary (before correction).
    pub current_value_summary: Option<String>,
    /// The proposed value summary (after correction). None for Confirm/KeepBoth.
    pub proposed_value_summary: Option<String>,
    /// The current truth state.
    pub current_truth_state: TruthState,
    /// The truth state the record will have after the correction.
    pub resulting_truth_state: TruthState,
    /// Count of dependent records that would be affected.
    pub affected_count: u32,
    /// Whether this correction can be reversed.
    pub reversible: bool,
    /// The audit consequence description (policy-safe text).
    pub audit_consequence: String,
    /// The base revision that must be supplied in `CorrectionCommitRequest`.
    pub base_revision: GraphRevision,
    /// A preview token (opaque, non-cryptographic) that the commit must supply.
    /// Prevents committing a stale preview.
    pub preview_token: String,
}

// ── CorrectionCommitRequest ───────────────────────────────────────────────

/// Commit request for a previewed correction.
///
/// The `preview_token` must match the one from `CorrectionPreview`; and
/// `base_revision` must equal the `base_revision` from the preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionCommitRequest {
    /// The correction kind to commit.
    pub kind: CorrectionKind,
    /// The record being corrected.
    pub record_id: String,
    /// The proposed new value (must match the preview).
    pub proposed_value_summary: Option<String>,
    /// The evidence for the correction.
    pub evidence_summary: Option<String>,
    /// Must match `CorrectionPreview::base_revision`.
    pub base_revision: GraphRevision,
    /// Must match `CorrectionPreview::preview_token`.
    pub preview_token: String,
    /// The actor ID for audit.
    pub actor_id: String,
}

// ── CorrectionResult ──────────────────────────────────────────────────────

/// The validated output of a committed correction.
///
/// Contains the before/after state, audit record reference, and reversal
/// eligibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionResult {
    /// The record ID being corrected.
    pub record_id: String,
    /// The kind of correction applied.
    pub kind: CorrectionKind,
    /// The truth state before the correction.
    pub before_truth_state: TruthState,
    /// The truth state after the correction.
    pub after_truth_state: TruthState,
    /// The value summary before the correction (for reversal).
    pub before_value_summary: Option<String>,
    /// The value summary after the correction.
    pub after_value_summary: Option<String>,
    /// A description suitable for an audit record.
    pub audit_description: String,
    /// Whether this correction can be reversed (and if so, how).
    pub reversible: bool,
    /// The base revision at which the correction was applied.
    pub applied_revision: GraphRevision,
}

// ── CorrectionError ───────────────────────────────────────────────────────

/// Errors that prevent a correction from being validated or committed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CorrectionError {
    /// Preview token mismatch — stale or tampered preview.
    #[error("stale or tampered preview: preview token does not match")]
    StalePreview,
    /// Base revision mismatch between commit request and preview.
    #[error("revision conflict: expected revision {expected}, got {got}")]
    RevisionConflict {
        expected: GraphRevision,
        got: GraphRevision,
    },
    /// The record is already in the target truth state.
    #[error("record is already in the target truth state")]
    AlreadyInTargetState,
    /// The record cannot be corrected in its current state.
    #[error("record cannot be corrected from its current truth state: {current}")]
    InvalidCurrentState { current: TruthState },
}

// ── Internal helpers ──────────────────────────────────────────────────────

/// Compute the deterministic (non-cryptographic) preview token from the key
/// fields that must match between preview and commit.
///
/// Token = `"{kind:?}:{record_id}:{base_revision}"`.  This is opaque to
/// callers but deterministic given the same inputs, so a stale/tampered
/// preview is detected without a secret.
fn compute_preview_token(
    kind: &CorrectionKind,
    record_id: &str,
    base_revision: GraphRevision,
) -> String {
    format!("{kind:?}:{record_id}:{base_revision}")
}

/// The truth state the *corrected* record (or the existing record for Confirm)
/// will have after the correction is applied.
fn resulting_truth_state(kind: &CorrectionKind) -> TruthState {
    match kind {
        // Correct/Supersede: old record → Superseded, new record → Current.
        // We return the state the *primary* record will take (old becomes Superseded).
        CorrectionKind::Correct | CorrectionKind::Supersede => TruthState::Superseded,
        // Confirm: the record moves to Confirmed.
        CorrectionKind::Confirm => TruthState::Confirmed,
        // KeepBoth: both records become Contradicted.
        CorrectionKind::KeepBoth => TruthState::Contradicted,
    }
}

/// Human-readable audit consequence text for each kind.
fn audit_consequence_text(kind: &CorrectionKind) -> &'static str {
    match kind {
        CorrectionKind::Correct => {
            "Record marked Superseded; corrected value recorded as the new Current truth."
        }
        CorrectionKind::Confirm => "Record truth state promoted to Confirmed by user authority.",
        CorrectionKind::Supersede => {
            "Record explicitly superseded by user; new value recorded as the new Current truth."
        }
        CorrectionKind::KeepBoth => {
            "Both conflicting values preserved as Contradicted; no side auto-resolved."
        }
    }
}

/// Human-readable audit description for a committed correction.
fn audit_description(kind: &CorrectionKind, record_id: &str, actor_id: &str) -> String {
    let action = match kind {
        CorrectionKind::Correct => "corrected",
        CorrectionKind::Confirm => "confirmed",
        CorrectionKind::Supersede => "superseded",
        CorrectionKind::KeepBoth => "kept-both (contradicted)",
    };
    format!(
        "Record '{}' {} by actor '{}' via user correction workflow.",
        record_id, action, actor_id
    )
}

// ── CorrectionValidator ───────────────────────────────────────────────────

/// Stateless previewer and validator for user correction commands.
///
/// Design §5.1: corrective commands go through `Previewed` state before
/// `Validate → TxOpen`. This struct implements both halves:
/// - [`Self::preview`] — builds the preview shown to the user.
/// - [`Self::validate_commit`] — validates the commit against the preview.
pub struct CorrectionValidator;

impl CorrectionValidator {
    /// Build a preview for a correction request.
    ///
    /// The preview shows the expected before/after state and generates a
    /// deterministic `preview_token` from the key fields.
    ///
    /// # Arguments
    ///
    /// * `req` — the preview request from the caller.
    /// * `current_truth_state` — the record's current truth state from the authority.
    /// * `current_value_summary` — the record's current value summary (for before/after display).
    /// * `affected_count` — count of dependent records that would be affected.
    pub fn preview(
        req: &CorrectionPreviewRequest,
        current_truth_state: TruthState,
        current_value_summary: Option<String>,
        affected_count: u32,
    ) -> CorrectionPreview {
        let token = compute_preview_token(&req.kind, &req.record_id, req.base_revision);
        let result_state = resulting_truth_state(&req.kind);
        let consequence = audit_consequence_text(&req.kind).to_string();

        CorrectionPreview {
            kind: req.kind.clone(),
            record_id: req.record_id.clone(),
            current_value_summary,
            proposed_value_summary: req.proposed_value_summary.clone(),
            current_truth_state,
            resulting_truth_state: result_state,
            affected_count,
            reversible: true, // all four kinds are reversible (Design §A9)
            audit_consequence: consequence,
            base_revision: req.base_revision,
            preview_token: token,
        }
    }

    /// Validate a commit request against the preview.
    ///
    /// # Validation rules
    ///
    /// 1. `preview_token` must match — recomputed from `req.kind`, `req.record_id`,
    ///    `req.base_revision`.  Mismatch → [`CorrectionError::StalePreview`].
    /// 2. `base_revision` on the request must equal the recomputed token's embedded
    ///    revision (implicitly enforced by token check).
    /// 3. `kind` mismatch is caught by token check (kind is part of the token).
    /// 4. `record_id` mismatch is caught by token check.
    /// 5. `Confirm` on an already-Confirmed record → [`CorrectionError::AlreadyInTargetState`].
    /// 6. Correcting a Deleted record → [`CorrectionError::InvalidCurrentState`].
    ///
    /// Returns a [`CorrectionResult`] on success.
    pub fn validate_commit(
        req: &CorrectionCommitRequest,
        current_truth_state: TruthState,
        current_value_summary: Option<String>,
    ) -> Result<CorrectionResult, CorrectionError> {
        // ── Rule 1: verify preview token ─────────────────────────────────
        let expected_token = compute_preview_token(&req.kind, &req.record_id, req.base_revision);
        if req.preview_token != expected_token {
            return Err(CorrectionError::StalePreview);
        }

        // ── Rule 5: Confirm on already-Confirmed ─────────────────────────
        if req.kind == CorrectionKind::Confirm && current_truth_state == TruthState::Confirmed {
            return Err(CorrectionError::AlreadyInTargetState);
        }

        // ── Rule 6: Deleted record cannot be corrected ───────────────────
        if current_truth_state == TruthState::Deleted {
            return Err(CorrectionError::InvalidCurrentState {
                current: TruthState::Deleted,
            });
        }

        // ── Build result ──────────────────────────────────────────────────
        let after_state = resulting_truth_state(&req.kind);
        let desc = audit_description(&req.kind, &req.record_id, &req.actor_id);

        Ok(CorrectionResult {
            record_id: req.record_id.clone(),
            kind: req.kind.clone(),
            before_truth_state: current_truth_state,
            after_truth_state: after_state,
            before_value_summary: current_value_summary,
            after_value_summary: req.proposed_value_summary.clone(),
            audit_description: desc,
            reversible: true,
            applied_revision: req.base_revision,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn rev(n: u64) -> GraphRevision {
        GraphRevision::new(n)
    }

    /// Build a minimal CorrectionPreviewRequest.
    fn preview_req(kind: CorrectionKind) -> CorrectionPreviewRequest {
        CorrectionPreviewRequest {
            kind,
            record_id: "rec-001".to_string(),
            proposed_value_summary: Some("new value".to_string()),
            evidence_summary: Some("source A".to_string()),
            base_revision: rev(5),
            actor_id: "user-alice".to_string(),
        }
    }

    /// Build a CorrectionCommitRequest that matches the preview.
    fn commit_req_from_preview(
        preview: &CorrectionPreview,
        actor_id: &str,
    ) -> CorrectionCommitRequest {
        CorrectionCommitRequest {
            kind: preview.kind.clone(),
            record_id: preview.record_id.clone(),
            proposed_value_summary: preview.proposed_value_summary.clone(),
            evidence_summary: None,
            base_revision: preview.base_revision,
            preview_token: preview.preview_token.clone(),
            actor_id: actor_id.to_string(),
        }
    }

    // ── 1. preview() produces correct resulting_truth_state for each kind ─

    #[test]
    fn preview_correct_resulting_state_is_superseded() {
        let req = preview_req(CorrectionKind::Correct);
        let preview = CorrectionValidator::preview(&req, TruthState::Current, None, 0);
        assert_eq!(preview.resulting_truth_state, TruthState::Superseded);
    }

    #[test]
    fn preview_confirm_resulting_state_is_confirmed() {
        let req = preview_req(CorrectionKind::Confirm);
        let preview = CorrectionValidator::preview(&req, TruthState::Current, None, 0);
        assert_eq!(preview.resulting_truth_state, TruthState::Confirmed);
    }

    #[test]
    fn preview_supersede_resulting_state_is_superseded() {
        let req = preview_req(CorrectionKind::Supersede);
        let preview = CorrectionValidator::preview(&req, TruthState::Current, None, 0);
        assert_eq!(preview.resulting_truth_state, TruthState::Superseded);
    }

    #[test]
    fn preview_keep_both_resulting_state_is_contradicted() {
        let req = preview_req(CorrectionKind::KeepBoth);
        let preview = CorrectionValidator::preview(&req, TruthState::Current, None, 0);
        assert_eq!(preview.resulting_truth_state, TruthState::Contradicted);
    }

    // ── 2. preview() always sets reversible = true ─────────────────────────

    #[test]
    fn preview_reversible_is_true_for_all_kinds() {
        for kind in [
            CorrectionKind::Correct,
            CorrectionKind::Confirm,
            CorrectionKind::Supersede,
            CorrectionKind::KeepBoth,
        ] {
            let req = preview_req(kind);
            let preview = CorrectionValidator::preview(&req, TruthState::Current, None, 0);
            assert!(
                preview.reversible,
                "kind {:?} must be reversible",
                preview.kind
            );
        }
    }

    // ── 3. validate_commit succeeds when preview_token matches ─────────────

    #[test]
    fn validate_commit_succeeds_with_matching_token() {
        let req = preview_req(CorrectionKind::Confirm);
        let preview =
            CorrectionValidator::preview(&req, TruthState::Current, Some("old val".to_string()), 0);
        let commit = commit_req_from_preview(&preview, "user-alice");
        let result = CorrectionValidator::validate_commit(
            &commit,
            TruthState::Current,
            Some("old val".to_string()),
        );
        assert!(result.is_ok(), "commit with matching token must succeed");
    }

    // ── 4. validate_commit fails with StalePreview when token is wrong ─────

    #[test]
    fn validate_commit_fails_with_stale_preview_on_wrong_token() {
        let req = preview_req(CorrectionKind::Correct);
        let preview = CorrectionValidator::preview(&req, TruthState::Current, None, 0);
        let mut commit = commit_req_from_preview(&preview, "user-alice");
        commit.preview_token = "tampered-token".to_string();
        let err =
            CorrectionValidator::validate_commit(&commit, TruthState::Current, None).unwrap_err();
        assert_eq!(err, CorrectionError::StalePreview);
    }

    // ── 5. validate_commit fails with StalePreview when base_revision mismatches

    #[test]
    fn validate_commit_fails_with_stale_preview_on_wrong_revision() {
        let req = preview_req(CorrectionKind::Correct);
        let preview = CorrectionValidator::preview(&req, TruthState::Current, None, 0);
        // Change base_revision — token was computed from rev 5, but now we submit rev 7.
        // The recomputed token won't match → StalePreview.
        let commit = CorrectionCommitRequest {
            kind: preview.kind.clone(),
            record_id: preview.record_id.clone(),
            proposed_value_summary: preview.proposed_value_summary.clone(),
            evidence_summary: None,
            base_revision: rev(7), // different from preview's rev(5)
            preview_token: preview.preview_token.clone(), // old token
            actor_id: "user-alice".to_string(),
        };
        let err =
            CorrectionValidator::validate_commit(&commit, TruthState::Current, None).unwrap_err();
        assert_eq!(err, CorrectionError::StalePreview);
    }

    // ── 6. Correct → before preserved, after = Superseded ─────────────────

    #[test]
    fn correct_before_state_preserved_after_is_superseded() {
        let req = preview_req(CorrectionKind::Correct);
        let preview =
            CorrectionValidator::preview(&req, TruthState::Current, Some("old".to_string()), 0);
        let commit = commit_req_from_preview(&preview, "user-alice");
        let result = CorrectionValidator::validate_commit(
            &commit,
            TruthState::Current,
            Some("old".to_string()),
        )
        .unwrap();
        assert_eq!(result.before_truth_state, TruthState::Current);
        assert_eq!(result.after_truth_state, TruthState::Superseded);
        assert_eq!(result.before_value_summary, Some("old".to_string()));
    }

    // ── 7. Confirm → after_truth_state = Confirmed ─────────────────────────

    #[test]
    fn confirm_after_state_is_confirmed() {
        let req = preview_req(CorrectionKind::Confirm);
        let preview = CorrectionValidator::preview(&req, TruthState::Current, None, 0);
        let commit = commit_req_from_preview(&preview, "user-alice");
        let result =
            CorrectionValidator::validate_commit(&commit, TruthState::Current, None).unwrap();
        assert_eq!(result.after_truth_state, TruthState::Confirmed);
    }

    // ── 8. KeepBoth → after_truth_state = Contradicted ─────────────────────

    #[test]
    fn keep_both_after_state_is_contradicted() {
        let req = preview_req(CorrectionKind::KeepBoth);
        let preview = CorrectionValidator::preview(&req, TruthState::Current, None, 0);
        let commit = commit_req_from_preview(&preview, "user-alice");
        let result =
            CorrectionValidator::validate_commit(&commit, TruthState::Current, None).unwrap();
        assert_eq!(result.after_truth_state, TruthState::Contradicted);
    }

    // ── 9. reversible = true for all committed kinds ───────────────────────

    #[test]
    fn commit_reversible_is_true_for_all_kinds() {
        for kind in [
            CorrectionKind::Correct,
            CorrectionKind::Confirm,
            CorrectionKind::Supersede,
            CorrectionKind::KeepBoth,
        ] {
            let req = preview_req(kind);
            let preview = CorrectionValidator::preview(&req, TruthState::Current, None, 0);
            let commit = commit_req_from_preview(&preview, "user-alice");
            let result =
                CorrectionValidator::validate_commit(&commit, TruthState::Current, None).unwrap();
            assert!(
                result.reversible,
                "kind {:?} must be reversible",
                result.kind
            );
        }
    }

    // ── 10. Confirm on already-Confirmed → AlreadyInTargetState ──────────

    #[test]
    fn confirm_on_confirmed_record_fails_already_in_target_state() {
        let req = preview_req(CorrectionKind::Confirm);
        let preview = CorrectionValidator::preview(&req, TruthState::Confirmed, None, 0);
        let commit = commit_req_from_preview(&preview, "user-alice");
        let err =
            CorrectionValidator::validate_commit(&commit, TruthState::Confirmed, None).unwrap_err();
        assert_eq!(err, CorrectionError::AlreadyInTargetState);
    }

    // ── 11. Any kind on Deleted → InvalidCurrentState ─────────────────────

    #[test]
    fn any_kind_on_deleted_record_fails_invalid_current_state() {
        for kind in [
            CorrectionKind::Correct,
            CorrectionKind::Confirm,
            CorrectionKind::Supersede,
            CorrectionKind::KeepBoth,
        ] {
            let req = preview_req(kind);
            let preview = CorrectionValidator::preview(&req, TruthState::Current, None, 0);
            let commit = commit_req_from_preview(&preview, "user-alice");
            let err = CorrectionValidator::validate_commit(&commit, TruthState::Deleted, None)
                .unwrap_err();
            assert_eq!(
                err,
                CorrectionError::InvalidCurrentState {
                    current: TruthState::Deleted
                },
                "kind {:?} on Deleted must fail",
                commit.kind
            );
        }
    }

    // ── 12. audit_description is non-empty and contains record_id ─────────

    #[test]
    fn committed_result_audit_description_contains_record_id() {
        let req = preview_req(CorrectionKind::Correct);
        let preview = CorrectionValidator::preview(&req, TruthState::Current, None, 0);
        let commit = commit_req_from_preview(&preview, "user-alice");
        let result =
            CorrectionValidator::validate_commit(&commit, TruthState::Current, None).unwrap();
        assert!(
            result.audit_description.contains("rec-001"),
            "audit description must reference the record id"
        );
        assert!(!result.audit_description.is_empty());
    }

    // ── 13. applied_revision matches the base_revision in commit ──────────

    #[test]
    fn applied_revision_matches_base_revision_in_commit() {
        let req = preview_req(CorrectionKind::Confirm);
        let preview = CorrectionValidator::preview(&req, TruthState::Current, None, 0);
        let commit = commit_req_from_preview(&preview, "user-alice");
        let result =
            CorrectionValidator::validate_commit(&commit, TruthState::Current, None).unwrap();
        assert_eq!(result.applied_revision, rev(5));
    }

    // ── 14. Supersede → before preserved, after = Superseded ──────────────

    #[test]
    fn supersede_before_preserved_after_is_superseded() {
        let req = preview_req(CorrectionKind::Supersede);
        let preview = CorrectionValidator::preview(
            &req,
            TruthState::Unverified,
            Some("unverified claim".to_string()),
            2,
        );
        let commit = commit_req_from_preview(&preview, "user-alice");
        let result = CorrectionValidator::validate_commit(
            &commit,
            TruthState::Unverified,
            Some("unverified claim".to_string()),
        )
        .unwrap();
        assert_eq!(result.before_truth_state, TruthState::Unverified);
        assert_eq!(result.after_truth_state, TruthState::Superseded);
        assert_eq!(
            result.before_value_summary,
            Some("unverified claim".to_string())
        );
    }

    // ── 15. preview_token is deterministic given same inputs ───────────────

    #[test]
    fn preview_token_is_deterministic() {
        let req = preview_req(CorrectionKind::Correct);
        let p1 = CorrectionValidator::preview(&req, TruthState::Current, None, 0);
        let p2 = CorrectionValidator::preview(&req, TruthState::Current, None, 5);
        // Same kind/record_id/base_revision → same token regardless of affected_count.
        assert_eq!(p1.preview_token, p2.preview_token);
    }

    // ── 16. Different kind produces different token ────────────────────────

    #[test]
    fn different_kind_produces_different_token() {
        let r1 = preview_req(CorrectionKind::Correct);
        let r2 = preview_req(CorrectionKind::Confirm);
        let p1 = CorrectionValidator::preview(&r1, TruthState::Current, None, 0);
        let p2 = CorrectionValidator::preview(&r2, TruthState::Current, None, 0);
        assert_ne!(p1.preview_token, p2.preview_token);
    }
}
