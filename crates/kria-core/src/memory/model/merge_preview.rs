//! Merge preview for conservative entity resolution (Design §A9, §5.1, MGR-019).
//!
//! # Design invariants
//!
//! Merges are **previewed** before commit (Design §5.1 command state machine:
//! `Previewed → Validate → TxOpen`). The workflow is:
//!
//! 1. Caller calls [`MergePreviewBuilder::build`] to get a [`MergePreview`] with
//!    a `preview_token` and `base_revision`.
//! 2. Caller confirms and sends a [`MergeCommitRequest`] carrying the same
//!    `preview_token` and `base_revision`.
//! 3. [`MergePreviewBuilder::validate_commit`] verifies the token/revision match.
//! 4. [`MergePreviewBuilder::mark_stale`] is called when the graph revision has
//!    advanced since the preview was generated.
//!
//! # Key rules
//!
//! - **Merge never broadens policy**: `PolicyMeet` uses `max(sensitivity)` and
//!   requires matching `namespace`/`scope`. Differing namespace or scope
//!   → `has_conflict=true`.
//! - **Preview token**: deterministic from `"{proposal_id}:{canonical_id}:{base_revision}"`.
//! - **All merges are reversible**: `reversible = true` always.
//! - **Stale detection**: `is_stale = false` at preview time. `mark_stale()` sets
//!   it to `true`. `validate_commit` rejects stale previews.
//! - **Warnings**: added when `policy_meet.has_conflict=true` or
//!   `summary.alias_conflicts > 0`.

use serde::{Deserialize, Serialize};

use crate::memory::model::{EntityId, GraphRevision};

// ── PolicyMeet ────────────────────────────────────────────────────────────

/// The effective policy result of merging two entities.
///
/// Merge never broadens policy: the meet is the most restrictive
/// policy of the two contributing entities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyMeet {
    /// The resulting namespace (must be consistent between both entities).
    pub namespace: String,
    /// The resulting scope (must be consistent between both entities).
    pub scope: String,
    /// The resulting sensitivity (max of both entities' sensitivity).
    pub sensitivity: u8,
    /// Whether there is a policy conflict (namespace/scope differ).
    pub has_conflict: bool,
    /// Description of the conflict, if any.
    pub conflict_description: Option<String>,
}

impl PolicyMeet {
    /// Compute the policy meet for two entities.
    ///
    /// Rules:
    /// - `sensitivity = max(left_sensitivity, right_sensitivity)` — never broadens.
    /// - If `left_namespace != right_namespace` or `left_scope != right_scope`
    ///   → `has_conflict = true` with a conflict description.
    /// - The resulting `namespace` and `scope` are taken from the left entity
    ///   when they match; when they conflict the left value is stored and the
    ///   conflict is reported.
    pub fn compute(
        left_namespace: &str,
        left_scope: &str,
        left_sensitivity: u8,
        right_namespace: &str,
        right_scope: &str,
        right_sensitivity: u8,
    ) -> PolicyMeet {
        let sensitivity = left_sensitivity.max(right_sensitivity);
        let namespace_conflict = left_namespace != right_namespace;
        let scope_conflict = left_scope != right_scope;
        let has_conflict = namespace_conflict || scope_conflict;

        let conflict_description = if has_conflict {
            let mut parts = Vec::new();
            if namespace_conflict {
                parts.push(format!(
                    "namespace mismatch: {:?} vs {:?}",
                    left_namespace, right_namespace
                ));
            }
            if scope_conflict {
                parts.push(format!(
                    "scope mismatch: {:?} vs {:?}",
                    left_scope, right_scope
                ));
            }
            Some(parts.join("; "))
        } else {
            None
        };

        PolicyMeet {
            namespace: left_namespace.to_string(),
            scope: left_scope.to_string(),
            sensitivity,
            has_conflict,
            conflict_description,
        }
    }
}

// ── MergePreviewSummary ───────────────────────────────────────────────────

/// Summary counts for a merge preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergePreviewSummary {
    /// Number of aliases from the left entity.
    pub left_alias_count: u32,
    /// Number of aliases from the right entity.
    pub right_alias_count: u32,
    /// Number of alias conflicts (same normalized alias in both entities).
    pub alias_conflicts: u32,
    /// Number of mentions across both entities.
    pub total_mention_count: u32,
    /// Number of links/relationships across both entities.
    pub total_link_count: u32,
    /// Number of evidence items across both entities.
    pub total_evidence_count: u32,
    /// Total records that would be affected by the merge.
    pub affected_count: u32,
}

// ── MergePreview ──────────────────────────────────────────────────────────

/// A merge preview showing all aspects of a proposed entity merge.
///
/// Design §A9: merges are previewed before commit. This struct is the
/// preview shown to the user; it includes the canonical choice, policy meet,
/// alias/mention/link/evidence counts, conflicts, reversibility, and the
/// preview token the user must supply to commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergePreview {
    /// The proposal being previewed.
    pub proposal_id: String,
    /// The canonical entity ID (the one that will survive the merge).
    pub canonical_entity_id: EntityId,
    /// The non-canonical entity ID (the one that will be merged into the canonical).
    pub non_canonical_entity_id: EntityId,
    /// The policy meet result.
    pub policy_meet: PolicyMeet,
    /// Summary counts.
    pub summary: MergePreviewSummary,
    /// Whether the merge can be reversed.
    pub reversible: bool,
    /// The base revision at which this preview was generated.
    pub base_revision: GraphRevision,
    /// Preview token (deterministic, opaque) for commit.
    pub preview_token: String,
    /// Any warnings about the merge (e.g. policy conflicts, alias clashes).
    pub warnings: Vec<String>,
    /// Whether the preview is stale (base_revision has advanced since preview).
    pub is_stale: bool,
}

// ── MergeCommitRequest ────────────────────────────────────────────────────

/// A request to commit a previewed entity merge.
///
/// The `preview_token` must match the one from the corresponding
/// [`MergePreview`]; and `base_revision` must equal the `base_revision`
/// from the preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeCommitRequest {
    /// The proposal being committed.
    pub proposal_id: String,
    /// The canonical entity ID.
    pub canonical_entity_id: EntityId,
    /// The non-canonical entity ID.
    pub non_canonical_entity_id: EntityId,
    /// Must match the preview_token.
    pub preview_token: String,
    /// Must match the base_revision from the preview.
    pub base_revision: GraphRevision,
    /// The actor committing the merge.
    pub actor_id: String,
}

// ── MergePreviewError ─────────────────────────────────────────────────────

/// Errors that prevent a merge preview from being built or committed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MergePreviewError {
    /// Canonical and non-canonical entities are the same.
    #[error("merge preview: canonical_entity_id and non_canonical_entity_id must differ (self-merge is not permitted)")]
    SelfMerge,
    /// Preview token does not match (stale or tampered preview).
    #[error("merge preview: preview token does not match — stale or tampered preview")]
    StalePreview,
    /// Policy conflict prevents merge.
    #[error("merge preview: policy conflict: {reason}")]
    PolicyConflict { reason: String },
}

// ── Internal helper ───────────────────────────────────────────────────────

/// Compute the deterministic (non-cryptographic) preview token.
///
/// Token = `"{proposal_id}:{canonical_id}:{base_revision}"`.
fn compute_merge_preview_token(
    proposal_id: &str,
    canonical_id: &EntityId,
    base_revision: GraphRevision,
) -> String {
    format!("{proposal_id}:{canonical_id}:{base_revision}")
}

// ── MergePreviewBuilder ───────────────────────────────────────────────────

/// Stateless builder and validator for merge preview commands.
///
/// Design §5.1: destructive/corrective commands go through `Previewed` state
/// before `Validate → TxOpen`. This struct implements both halves:
/// - [`Self::build`] — builds the preview shown to the user.
/// - [`Self::validate_commit`] — validates the commit against the preview.
/// - [`Self::mark_stale`] — marks a preview as stale when the graph has advanced.
pub struct MergePreviewBuilder;

impl MergePreviewBuilder {
    /// Build a merge preview.
    ///
    /// Rules:
    /// - `canonical_entity_id != non_canonical_entity_id` — otherwise
    ///   [`MergePreviewError::SelfMerge`].
    /// - `preview_token` is computed deterministically from
    ///   `"{proposal_id}:{canonical_id}:{base_revision}"`.
    /// - `reversible = true` always (merges are always reversible via split).
    /// - `is_stale = false` at preview time (caller checks staleness on commit).
    /// - `policy_meet` is computed from the two entities' policy fields.
    /// - `warnings` include policy conflicts and alias conflicts.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        proposal_id: String,
        canonical_entity_id: EntityId,
        non_canonical_entity_id: EntityId,
        left_namespace: &str,
        left_scope: &str,
        left_sensitivity: u8,
        right_namespace: &str,
        right_scope: &str,
        right_sensitivity: u8,
        summary: MergePreviewSummary,
        base_revision: GraphRevision,
    ) -> Result<MergePreview, MergePreviewError> {
        // Enforce: no self-merge.
        if canonical_entity_id == non_canonical_entity_id {
            return Err(MergePreviewError::SelfMerge);
        }

        // Compute the policy meet.
        let policy_meet = PolicyMeet::compute(
            left_namespace,
            left_scope,
            left_sensitivity,
            right_namespace,
            right_scope,
            right_sensitivity,
        );

        // Build warnings.
        let mut warnings = Vec::new();
        if policy_meet.has_conflict {
            if let Some(ref desc) = policy_meet.conflict_description {
                warnings.push(format!("Policy conflict: {desc}"));
            } else {
                warnings.push("Policy conflict between entities".to_string());
            }
        }
        if summary.alias_conflicts > 0 {
            warnings.push(format!(
                "Alias conflicts: {} normalized alias(es) appear in both entities",
                summary.alias_conflicts
            ));
        }

        // Compute the deterministic preview token.
        let preview_token =
            compute_merge_preview_token(&proposal_id, &canonical_entity_id, base_revision);

        Ok(MergePreview {
            proposal_id,
            canonical_entity_id,
            non_canonical_entity_id,
            policy_meet,
            summary,
            reversible: true, // merges are always reversible via split (Design §A9)
            base_revision,
            preview_token,
            warnings,
            is_stale: false, // caller checks staleness on commit
        })
    }

    /// Validate a commit request against a preview.
    ///
    /// Rules:
    /// - `is_stale` must be `false` — stale previews are rejected.
    /// - `preview_token` must match (recomputed from `proposal_id +
    ///   canonical_id + base_revision`). Mismatch → [`MergePreviewError::StalePreview`].
    /// - `base_revision` must match.
    /// - `canonical_entity_id` must match.
    ///
    /// Returns `Ok(())` on success.
    pub fn validate_commit(
        req: &MergeCommitRequest,
        preview: &MergePreview,
    ) -> Result<(), MergePreviewError> {
        // Reject stale previews first.
        if preview.is_stale {
            return Err(MergePreviewError::StalePreview);
        }

        // Recompute expected token from commit fields and compare.
        let expected_token = compute_merge_preview_token(
            &req.proposal_id,
            &req.canonical_entity_id,
            req.base_revision,
        );
        if req.preview_token != expected_token {
            return Err(MergePreviewError::StalePreview);
        }

        // Verify token matches the preview's stored token.
        if req.preview_token != preview.preview_token {
            return Err(MergePreviewError::StalePreview);
        }

        Ok(())
    }

    /// Mark a preview as stale.
    ///
    /// Called when the graph revision has advanced since the preview was
    /// generated. A stale preview cannot be committed.
    pub fn mark_stale(preview: &mut MergePreview) {
        preview.is_stale = true;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn rev(n: u64) -> GraphRevision {
        GraphRevision::new(n)
    }

    fn summary_empty() -> MergePreviewSummary {
        MergePreviewSummary {
            left_alias_count: 0,
            right_alias_count: 0,
            alias_conflicts: 0,
            total_mention_count: 0,
            total_link_count: 0,
            total_evidence_count: 0,
            affected_count: 0,
        }
    }

    fn distinct_ids() -> (EntityId, EntityId) {
        (EntityId::new_v7(), EntityId::new_v7())
    }

    fn build_simple(
        canonical: EntityId,
        non_canonical: EntityId,
        summary: MergePreviewSummary,
    ) -> Result<MergePreview, MergePreviewError> {
        MergePreviewBuilder::build(
            "proposal-001".to_string(),
            canonical,
            non_canonical,
            "user",
            "chat",
            1,
            "user",
            "chat",
            1,
            summary,
            rev(10),
        )
    }

    fn commit_from_preview(preview: &MergePreview, actor_id: &str) -> MergeCommitRequest {
        MergeCommitRequest {
            proposal_id: preview.proposal_id.clone(),
            canonical_entity_id: preview.canonical_entity_id.clone(),
            non_canonical_entity_id: preview.non_canonical_entity_id.clone(),
            preview_token: preview.preview_token.clone(),
            base_revision: preview.base_revision,
            actor_id: actor_id.to_string(),
        }
    }

    // ── 1. PolicyMeet::compute: same namespace/scope → no conflict, max sensitivity

    #[test]
    fn policy_meet_same_ns_scope_no_conflict_max_sensitivity() {
        let pm = PolicyMeet::compute("user", "chat", 1, "user", "chat", 2);
        assert_eq!(pm.namespace, "user");
        assert_eq!(pm.scope, "chat");
        assert_eq!(pm.sensitivity, 2); // max(1, 2)
        assert!(!pm.has_conflict);
        assert!(pm.conflict_description.is_none());
    }

    #[test]
    fn policy_meet_same_ns_scope_left_higher_sensitivity() {
        let pm = PolicyMeet::compute("ns", "sc", 3, "ns", "sc", 0);
        assert_eq!(pm.sensitivity, 3); // max(3, 0)
        assert!(!pm.has_conflict);
    }

    // ── 2. PolicyMeet::compute: different namespace → has_conflict=true

    #[test]
    fn policy_meet_different_namespace_has_conflict() {
        let pm = PolicyMeet::compute("ns_a", "chat", 0, "ns_b", "chat", 0);
        assert!(pm.has_conflict);
        assert!(pm.conflict_description.is_some());
        let desc = pm.conflict_description.unwrap();
        assert!(desc.contains("namespace mismatch"));
        assert!(desc.contains("ns_a"));
        assert!(desc.contains("ns_b"));
    }

    #[test]
    fn policy_meet_different_scope_has_conflict() {
        let pm = PolicyMeet::compute("user", "scope_a", 0, "user", "scope_b", 0);
        assert!(pm.has_conflict);
        let desc = pm.conflict_description.unwrap();
        assert!(desc.contains("scope mismatch"));
    }

    #[test]
    fn policy_meet_both_namespace_and_scope_conflict() {
        let pm = PolicyMeet::compute("ns_a", "sc_a", 1, "ns_b", "sc_b", 2);
        assert!(pm.has_conflict);
        let desc = pm.conflict_description.unwrap();
        assert!(desc.contains("namespace mismatch"));
        assert!(desc.contains("scope mismatch"));
        assert_eq!(pm.sensitivity, 2); // max still computed
    }

    // ── 3. MergePreviewBuilder::build: success

    #[test]
    fn build_success_produces_correct_preview() {
        let (canonical, non_canonical) = distinct_ids();
        let preview =
            build_simple(canonical.clone(), non_canonical.clone(), summary_empty()).unwrap();

        assert_eq!(preview.proposal_id, "proposal-001");
        assert_eq!(preview.canonical_entity_id, canonical);
        assert_eq!(preview.non_canonical_entity_id, non_canonical);
        assert!(preview.reversible, "merges must always be reversible");
        assert!(!preview.is_stale, "new preview must not be stale");
        assert!(preview.warnings.is_empty(), "no conflicts → no warnings");
        assert!(!preview.preview_token.is_empty());
    }

    #[test]
    fn build_with_policy_conflict_adds_warning() {
        let (canonical, non_canonical) = distinct_ids();
        let preview = MergePreviewBuilder::build(
            "p-002".to_string(),
            canonical,
            non_canonical,
            "ns_a",
            "sc",
            0,
            "ns_b",
            "sc",
            0,
            summary_empty(),
            rev(5),
        )
        .unwrap();

        assert!(preview.policy_meet.has_conflict);
        assert!(
            !preview.warnings.is_empty(),
            "policy conflict must add a warning"
        );
        assert!(preview.warnings[0].contains("Policy conflict"));
    }

    #[test]
    fn build_with_alias_conflicts_adds_warning() {
        let (canonical, non_canonical) = distinct_ids();
        let mut summary = summary_empty();
        summary.alias_conflicts = 3;
        let preview = build_simple(canonical, non_canonical, summary).unwrap();

        let has_alias_warning = preview
            .warnings
            .iter()
            .any(|w| w.contains("Alias conflict"));
        assert!(has_alias_warning, "alias conflicts must add a warning");
    }

    // ── 4. MergePreviewBuilder::build: SelfMerge error

    #[test]
    fn build_self_merge_returns_error() {
        let id = EntityId::new_v7();
        let err = MergePreviewBuilder::build(
            "p-self".to_string(),
            id.clone(),
            id.clone(), // same as canonical
            "user",
            "chat",
            0,
            "user",
            "chat",
            0,
            summary_empty(),
            rev(1),
        )
        .unwrap_err();

        assert_eq!(err, MergePreviewError::SelfMerge);
        assert!(err.to_string().contains("self-merge"));
    }

    // ── 5. validate_commit: success with matching token

    #[test]
    fn validate_commit_success_with_matching_token() {
        let (canonical, non_canonical) = distinct_ids();
        let preview = build_simple(canonical, non_canonical, summary_empty()).unwrap();
        let req = commit_from_preview(&preview, "actor-alice");
        let result = MergePreviewBuilder::validate_commit(&req, &preview);
        assert!(result.is_ok(), "commit with matching token must succeed");
    }

    // ── 6. validate_commit: StalePreview on wrong token

    #[test]
    fn validate_commit_stale_preview_on_wrong_token() {
        let (canonical, non_canonical) = distinct_ids();
        let preview = build_simple(canonical, non_canonical, summary_empty()).unwrap();
        let mut req = commit_from_preview(&preview, "actor-alice");
        req.preview_token = "tampered-token".to_string();
        let err = MergePreviewBuilder::validate_commit(&req, &preview).unwrap_err();
        assert_eq!(err, MergePreviewError::StalePreview);
    }

    #[test]
    fn validate_commit_stale_preview_on_wrong_revision() {
        let (canonical, non_canonical) = distinct_ids();
        let preview = build_simple(canonical, non_canonical, summary_empty()).unwrap();
        let mut req = commit_from_preview(&preview, "actor-alice");
        // Change the revision — the recomputed token won't match.
        req.base_revision = rev(99);
        let err = MergePreviewBuilder::validate_commit(&req, &preview).unwrap_err();
        assert_eq!(err, MergePreviewError::StalePreview);
    }

    // ── 7. mark_stale: sets is_stale=true and commit is then rejected

    #[test]
    fn mark_stale_sets_is_stale_true() {
        let (canonical, non_canonical) = distinct_ids();
        let mut preview = build_simple(canonical, non_canonical, summary_empty()).unwrap();
        assert!(!preview.is_stale);
        MergePreviewBuilder::mark_stale(&mut preview);
        assert!(preview.is_stale);
    }

    #[test]
    fn validate_commit_rejects_stale_preview() {
        let (canonical, non_canonical) = distinct_ids();
        let mut preview = build_simple(canonical, non_canonical, summary_empty()).unwrap();
        let req = commit_from_preview(&preview, "actor-alice");
        MergePreviewBuilder::mark_stale(&mut preview);
        let err = MergePreviewBuilder::validate_commit(&req, &preview).unwrap_err();
        assert_eq!(err, MergePreviewError::StalePreview);
    }

    // ── 8. reversible always true

    #[test]
    fn reversible_is_always_true() {
        let (canonical, non_canonical) = distinct_ids();
        let preview = build_simple(canonical, non_canonical, summary_empty()).unwrap();
        assert!(preview.reversible, "all merges must be reversible");
    }

    // ── 9. preview_token is deterministic

    #[test]
    fn preview_token_is_deterministic() {
        let (canonical, non_canonical) = distinct_ids();
        // Build twice with same inputs — token must be identical.
        let p1 = MergePreviewBuilder::build(
            "p-det".to_string(),
            canonical.clone(),
            non_canonical.clone(),
            "user",
            "chat",
            0,
            "user",
            "chat",
            0,
            summary_empty(),
            rev(7),
        )
        .unwrap();
        let p2 = MergePreviewBuilder::build(
            "p-det".to_string(),
            canonical.clone(),
            non_canonical.clone(),
            "user",
            "chat",
            0,
            "user",
            "chat",
            0,
            summary_empty(),
            rev(7),
        )
        .unwrap();
        assert_eq!(p1.preview_token, p2.preview_token);
    }

    #[test]
    fn different_base_revision_produces_different_token() {
        let (canonical, non_canonical) = distinct_ids();
        let p1 = build_simple(canonical.clone(), non_canonical.clone(), summary_empty()).unwrap();
        let p2 = MergePreviewBuilder::build(
            "proposal-001".to_string(),
            canonical,
            non_canonical,
            "user",
            "chat",
            1,
            "user",
            "chat",
            1,
            summary_empty(),
            rev(99), // different revision
        )
        .unwrap();
        assert_ne!(p1.preview_token, p2.preview_token);
    }
}
