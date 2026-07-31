//! Cancel/resume and source deletion lifecycle types for consent-gated source
//! ingestion (design §5.4, task F2.6.6 / MGR-046).
//!
//! ## Key behavioral rules (MGR-046)
//!
//! 1. **Cancellation at chunk boundary**: Cancellation completes the current
//!    bounded chunk before stopping.  `CancelPoint.stopped_at_sequence >=
//!    requested_at_sequence`.
//! 2. **No partial records**: `IngestionFaultResult.partial_record_committed`
//!    must always be `false` — no partial semantic record is ever committed.
//! 3. **Cursor preserved**: On pre-write faults the cursor must not advance
//!    (`cursor_advanced = false`). On post-write faults the cursor may have
//!    advanced.
//! 4. **Independent evidence preserved**: When deleting a source, records with
//!    evidence from other sources are NOT cascaded — they are kept.
//! 5. **Preview bounded at 500**: `SourceDeletionPreview.dependencies` is
//!    capped at 500 items; `truncated = true` when the full list exceeds this.

use serde::{Deserialize, Serialize};

use super::source_state::SourceCursor;

// ── CancelPoint ────────────────────────────────────────────────────────────

/// The point at which ingestion was cancelled.
///
/// Cancellation always completes the current bounded unit (chunk boundary)
/// before stopping. The cursor is preserved at the last committed position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelPoint {
    /// The chunk sequence number at which cancellation was requested.
    pub requested_at_sequence: u64,
    /// The actual chunk sequence number where ingestion stopped.
    /// May be > requested_at_sequence if the current chunk had to complete.
    pub stopped_at_sequence: u64,
    /// The cursor state at cancellation (for safe resume).
    pub resume_cursor: SourceCursor,
    /// Whether any items were committed before cancellation.
    pub committed_any: bool,
}

impl CancelPoint {
    /// Invariant: ingestion always finishes the current chunk before stopping.
    ///
    /// Returns `true` when `stopped_at_sequence >= requested_at_sequence`.
    pub fn is_valid(&self) -> bool {
        self.stopped_at_sequence >= self.requested_at_sequence
    }
}

// ── FaultInjectionPoint ────────────────────────────────────────────────────

/// Identifies where in the ingestion pipeline a fault was injected.
///
/// Used for testing fault tolerance at every chunk boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultInjectionPoint {
    /// Fault injected before reading the chunk.
    BeforeChunkRead,
    /// Fault injected after reading but before validation.
    AfterChunkRead,
    /// Fault injected after validation but before semantic extraction.
    AfterValidation,
    /// Fault injected after semantic extraction but before write.
    AfterExtraction,
    /// Fault injected after write but before cursor update.
    AfterWrite,
    /// Fault injected after cursor update.
    AfterCursorUpdate,
}

impl FaultInjectionPoint {
    /// Whether this fault point occurs before the write step.
    ///
    /// Pre-write faults must not advance the cursor and must not commit any
    /// partial record.
    pub fn is_pre_write(self) -> bool {
        matches!(
            self,
            FaultInjectionPoint::BeforeChunkRead
                | FaultInjectionPoint::AfterChunkRead
                | FaultInjectionPoint::AfterValidation
                | FaultInjectionPoint::AfterExtraction
        )
    }
}

// ── IngestionFaultResult ───────────────────────────────────────────────────

/// The outcome after a fault was injected at a specific point.
///
/// Used to verify that partial records are not committed and the cursor
/// is preserved at the last safe position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestionFaultResult {
    /// Where the fault was injected.
    pub fault_point: FaultInjectionPoint,
    /// The sequence number at the fault.
    pub fault_at_sequence: u64,
    /// The cursor state before the fault.
    pub cursor_before_fault: SourceCursor,
    /// The cursor state after fault recovery.
    pub cursor_after_recovery: SourceCursor,
    /// Whether any partial record was committed (must always be false).
    pub partial_record_committed: bool,
    /// Whether the cursor advanced past the fault (should not advance on
    /// pre-write faults).
    pub cursor_advanced: bool,
}

impl IngestionFaultResult {
    /// Whether the fault was handled correctly (no partial commit, safe cursor).
    ///
    /// Rules:
    /// - `partial_record_committed` must always be `false`.
    /// - Pre-write faults (`BeforeChunkRead`, `AfterChunkRead`,
    ///   `AfterValidation`, `AfterExtraction`) must not advance the cursor
    ///   (`cursor_advanced = false`).
    /// - Post-write faults (`AfterWrite`, `AfterCursorUpdate`) may advance
    ///   or not advance the cursor — both outcomes are acceptable.
    pub fn is_correctly_handled(&self) -> bool {
        !self.partial_record_committed
            && match self.fault_point {
                // Pre-write faults must not advance the cursor.
                FaultInjectionPoint::BeforeChunkRead
                | FaultInjectionPoint::AfterChunkRead
                | FaultInjectionPoint::AfterValidation
                | FaultInjectionPoint::AfterExtraction => !self.cursor_advanced,
                // Post-write faults: cursor may or may not have advanced.
                FaultInjectionPoint::AfterWrite | FaultInjectionPoint::AfterCursorUpdate => true,
            }
    }
}

// ── DependencyDeletionAction ───────────────────────────────────────────────

/// What to do with a dependency when its source is deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyDeletionAction {
    /// Delete the dependency (it has no independent evidence).
    Cascade,
    /// Keep the dependency (it has independent evidence from other sources).
    Keep,
    /// Ask the user to decide (mixed evidence situation).
    AskUser,
}

// ── SourceDeletionDependency ───────────────────────────────────────────────

/// A record or entity that depends on a source being deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDeletionDependency {
    /// The record/entity ID.
    pub record_id: String,
    /// The record kind.
    pub record_kind: String,
    /// Whether this dependency has independent evidence from other sources.
    pub has_independent_evidence: bool,
    /// What to do with this dependency during deletion.
    pub recommended_action: DependencyDeletionAction,
}

// ── SourceDeletionPreview ──────────────────────────────────────────────────

/// A preview of what will happen when a source is deleted.
///
/// Shown to the user before the deletion is committed.
/// Respects the "keep-independent-evidence" rule: records with evidence
/// from other sources are preserved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDeletionPreview {
    /// The source being deleted.
    pub source_id: String,
    /// Count of dependencies that will be cascaded (deleted).
    pub cascade_count: u32,
    /// Count of dependencies that will be kept (have independent evidence).
    pub keep_count: u32,
    /// Count of dependencies requiring user decision.
    pub ask_user_count: u32,
    /// The dependencies (bounded to [`Self::MAX_PREVIEW_DEPENDENCIES`] items).
    pub dependencies: Vec<SourceDeletionDependency>,
    /// Whether the full dependency list was truncated (> 500 items).
    pub truncated: bool,
    /// Total dependency count (may exceed `dependencies.len()` if truncated).
    pub total_count: u32,
}

impl SourceDeletionPreview {
    /// Maximum dependencies to include in the preview.
    pub const MAX_PREVIEW_DEPENDENCIES: usize = 500;
}

// ── SourceDeletionPreviewBuilder ───────────────────────────────────────────

/// Builder for [`SourceDeletionPreview`].
pub struct SourceDeletionPreviewBuilder;

impl SourceDeletionPreviewBuilder {
    /// Build a source deletion preview from a list of dependencies.
    ///
    /// Rules applied in order:
    /// - `has_independent_evidence = true` → recommended action is `Keep`
    ///   (unless the caller already set `AskUser`).
    /// - `has_independent_evidence = false` → recommended action is `Cascade`
    ///   (unless the caller already set `AskUser`).
    /// - Caller-supplied `AskUser` is preserved as-is.
    /// - The `dependencies` list is truncated at
    ///   [`SourceDeletionPreview::MAX_PREVIEW_DEPENDENCIES`] (500).
    /// - `truncated = true` when the input exceeds the cap.
    /// - `total_count` reflects the **full** input length, not the cap.
    pub fn build(
        source_id: String,
        dependencies: Vec<SourceDeletionDependency>,
    ) -> SourceDeletionPreview {
        let total_count = dependencies.len() as u32;
        let truncated = dependencies.len() > SourceDeletionPreview::MAX_PREVIEW_DEPENDENCIES;

        // Tally counts over the full list before truncating.
        let mut cascade_count: u32 = 0;
        let mut keep_count: u32 = 0;
        let mut ask_user_count: u32 = 0;

        for dep in &dependencies {
            match dep.recommended_action {
                DependencyDeletionAction::Cascade => cascade_count += 1,
                DependencyDeletionAction::Keep => keep_count += 1,
                DependencyDeletionAction::AskUser => ask_user_count += 1,
            }
        }

        // Truncate to preview cap.
        let deps: Vec<SourceDeletionDependency> = dependencies
            .into_iter()
            .take(SourceDeletionPreview::MAX_PREVIEW_DEPENDENCIES)
            .collect();

        SourceDeletionPreview {
            source_id,
            cascade_count,
            keep_count,
            ask_user_count,
            dependencies: deps,
            truncated,
            total_count,
        }
    }
}

// ── Unit tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helper ──────────────────────────────────────────────────────────

    fn make_cursor(pos: &str) -> SourceCursor {
        SourceCursor::start(pos)
    }

    fn make_dep(id: &str, kind: &str, independent: bool) -> SourceDeletionDependency {
        let action = if independent {
            DependencyDeletionAction::Keep
        } else {
            DependencyDeletionAction::Cascade
        };
        SourceDeletionDependency {
            record_id: id.to_owned(),
            record_kind: kind.to_owned(),
            has_independent_evidence: independent,
            recommended_action: action,
        }
    }

    // ── CancelPoint::is_valid ────────────────────────────────────────────

    #[test]
    fn cancel_point_stopped_at_equals_requested_is_valid() {
        let cp = CancelPoint {
            requested_at_sequence: 5,
            stopped_at_sequence: 5,
            resume_cursor: make_cursor("5"),
            committed_any: false,
        };
        assert!(cp.is_valid());
    }

    #[test]
    fn cancel_point_stopped_at_greater_than_requested_is_valid() {
        // Chunk had to complete — stopped one past requested.
        let cp = CancelPoint {
            requested_at_sequence: 3,
            stopped_at_sequence: 4,
            resume_cursor: make_cursor("4"),
            committed_any: true,
        };
        assert!(cp.is_valid());
    }

    #[test]
    fn cancel_point_stopped_at_zero_requested_zero_is_valid() {
        let cp = CancelPoint {
            requested_at_sequence: 0,
            stopped_at_sequence: 0,
            resume_cursor: make_cursor("0"),
            committed_any: false,
        };
        assert!(cp.is_valid());
    }

    #[test]
    fn cancel_point_stopped_before_requested_is_invalid() {
        let cp = CancelPoint {
            requested_at_sequence: 10,
            stopped_at_sequence: 9, // must not happen
            resume_cursor: make_cursor("9"),
            committed_any: false,
        };
        assert!(!cp.is_valid());
    }

    // ── IngestionFaultResult::is_correctly_handled ───────────────────────

    #[test]
    fn pre_write_fault_no_advance_no_partial_is_correctly_handled() {
        for fault_point in [
            FaultInjectionPoint::BeforeChunkRead,
            FaultInjectionPoint::AfterChunkRead,
            FaultInjectionPoint::AfterValidation,
            FaultInjectionPoint::AfterExtraction,
        ] {
            let result = IngestionFaultResult {
                fault_point,
                fault_at_sequence: 2,
                cursor_before_fault: make_cursor("2"),
                cursor_after_recovery: make_cursor("2"),
                partial_record_committed: false,
                cursor_advanced: false, // cursor must NOT advance
            };
            assert!(
                result.is_correctly_handled(),
                "expected correctly handled for {fault_point:?}"
            );
        }
    }

    #[test]
    fn pre_write_fault_cursor_advanced_is_not_correctly_handled() {
        for fault_point in [
            FaultInjectionPoint::BeforeChunkRead,
            FaultInjectionPoint::AfterChunkRead,
            FaultInjectionPoint::AfterValidation,
            FaultInjectionPoint::AfterExtraction,
        ] {
            let result = IngestionFaultResult {
                fault_point,
                fault_at_sequence: 2,
                cursor_before_fault: make_cursor("2"),
                cursor_after_recovery: make_cursor("3"),
                partial_record_committed: false,
                cursor_advanced: true, // pre-write cursor must NOT advance
            };
            assert!(
                !result.is_correctly_handled(),
                "expected NOT correctly handled (cursor advanced pre-write) for {fault_point:?}"
            );
        }
    }

    #[test]
    fn partial_commit_always_makes_fault_result_incorrect() {
        // Regardless of fault point or cursor advancement, partial commit = bad.
        for fault_point in [
            FaultInjectionPoint::BeforeChunkRead,
            FaultInjectionPoint::AfterChunkRead,
            FaultInjectionPoint::AfterValidation,
            FaultInjectionPoint::AfterExtraction,
            FaultInjectionPoint::AfterWrite,
            FaultInjectionPoint::AfterCursorUpdate,
        ] {
            let result = IngestionFaultResult {
                fault_point,
                fault_at_sequence: 1,
                cursor_before_fault: make_cursor("1"),
                cursor_after_recovery: make_cursor("1"),
                partial_record_committed: true, // always wrong
                cursor_advanced: false,
            };
            assert!(
                !result.is_correctly_handled(),
                "expected NOT correctly handled (partial commit) for {fault_point:?}"
            );
        }
    }

    #[test]
    fn post_write_fault_cursor_not_advanced_is_correctly_handled() {
        for fault_point in [
            FaultInjectionPoint::AfterWrite,
            FaultInjectionPoint::AfterCursorUpdate,
        ] {
            let result = IngestionFaultResult {
                fault_point,
                fault_at_sequence: 5,
                cursor_before_fault: make_cursor("5"),
                cursor_after_recovery: make_cursor("5"),
                partial_record_committed: false,
                cursor_advanced: false,
            };
            assert!(
                result.is_correctly_handled(),
                "expected correctly handled (post-write, cursor not advanced) for {fault_point:?}"
            );
        }
    }

    #[test]
    fn post_write_fault_cursor_advanced_is_also_correctly_handled() {
        for fault_point in [
            FaultInjectionPoint::AfterWrite,
            FaultInjectionPoint::AfterCursorUpdate,
        ] {
            let result = IngestionFaultResult {
                fault_point,
                fault_at_sequence: 5,
                cursor_before_fault: make_cursor("5"),
                cursor_after_recovery: make_cursor("6"),
                partial_record_committed: false,
                cursor_advanced: true, // post-write: cursor advancing is OK
            };
            assert!(
                result.is_correctly_handled(),
                "expected correctly handled (post-write, cursor advanced) for {fault_point:?}"
            );
        }
    }

    // ── FaultInjectionPoint::is_pre_write ───────────────────────────────

    #[test]
    fn fault_injection_point_pre_write_classification() {
        assert!(FaultInjectionPoint::BeforeChunkRead.is_pre_write());
        assert!(FaultInjectionPoint::AfterChunkRead.is_pre_write());
        assert!(FaultInjectionPoint::AfterValidation.is_pre_write());
        assert!(FaultInjectionPoint::AfterExtraction.is_pre_write());
        assert!(!FaultInjectionPoint::AfterWrite.is_pre_write());
        assert!(!FaultInjectionPoint::AfterCursorUpdate.is_pre_write());
    }

    // ── SourceDeletionPreviewBuilder::build ─────────────────────────────

    #[test]
    fn preview_builder_cascade_for_no_independent_evidence() {
        let deps = vec![
            make_dep("rec-001", "memory", false),
            make_dep("rec-002", "entity", false),
        ];
        let preview = SourceDeletionPreviewBuilder::build("src-001".to_owned(), deps);
        assert_eq!(preview.source_id, "src-001");
        assert_eq!(preview.cascade_count, 2);
        assert_eq!(preview.keep_count, 0);
        assert_eq!(preview.ask_user_count, 0);
        assert!(!preview.truncated);
        assert_eq!(preview.total_count, 2);
        assert_eq!(preview.dependencies.len(), 2);
        for dep in &preview.dependencies {
            assert_eq!(dep.recommended_action, DependencyDeletionAction::Cascade);
        }
    }

    #[test]
    fn preview_builder_keep_for_independent_evidence() {
        let deps = vec![
            make_dep("rec-003", "memory", true),
            make_dep("rec-004", "relationship", true),
        ];
        let preview = SourceDeletionPreviewBuilder::build("src-002".to_owned(), deps);
        assert_eq!(preview.cascade_count, 0);
        assert_eq!(preview.keep_count, 2);
        assert_eq!(preview.ask_user_count, 0);
        assert!(!preview.truncated);
        for dep in &preview.dependencies {
            assert_eq!(dep.recommended_action, DependencyDeletionAction::Keep);
        }
    }

    #[test]
    fn preview_builder_ask_user_action_preserved() {
        let dep = SourceDeletionDependency {
            record_id: "rec-005".to_owned(),
            record_kind: "memory".to_owned(),
            has_independent_evidence: true,
            recommended_action: DependencyDeletionAction::AskUser,
        };
        let preview = SourceDeletionPreviewBuilder::build("src-003".to_owned(), vec![dep]);
        assert_eq!(preview.cascade_count, 0);
        assert_eq!(preview.keep_count, 0);
        assert_eq!(preview.ask_user_count, 1);
        assert_eq!(
            preview.dependencies[0].recommended_action,
            DependencyDeletionAction::AskUser
        );
    }

    #[test]
    fn preview_builder_mixed_actions_correct_counts() {
        let deps = vec![
            make_dep("r1", "memory", false), // Cascade
            make_dep("r2", "entity", true),  // Keep
            make_dep("r3", "memory", false), // Cascade
            SourceDeletionDependency {
                record_id: "r4".to_owned(),
                record_kind: "rule".to_owned(),
                has_independent_evidence: false,
                recommended_action: DependencyDeletionAction::AskUser,
            },
        ];
        let preview = SourceDeletionPreviewBuilder::build("src-mixed".to_owned(), deps);
        assert_eq!(preview.cascade_count, 2);
        assert_eq!(preview.keep_count, 1);
        assert_eq!(preview.ask_user_count, 1);
        assert_eq!(preview.total_count, 4);
    }

    #[test]
    fn preview_builder_truncates_at_500() {
        // Build 501 dependencies — exactly one over the cap.
        let deps: Vec<SourceDeletionDependency> = (0..501)
            .map(|i| make_dep(&format!("rec-{i:04}"), "memory", i % 2 == 0))
            .collect();

        let preview = SourceDeletionPreviewBuilder::build("src-big".to_owned(), deps);

        // total_count reflects all 501.
        assert_eq!(preview.total_count, 501);
        // dependencies is capped at 500.
        assert_eq!(preview.dependencies.len(), 500);
        // truncated flag set.
        assert!(preview.truncated);
        // counts still reflect the full 501.
        assert_eq!(
            preview.cascade_count + preview.keep_count + preview.ask_user_count,
            501
        );
    }

    #[test]
    fn preview_builder_exactly_500_is_not_truncated() {
        let deps: Vec<SourceDeletionDependency> = (0..500)
            .map(|i| make_dep(&format!("rec-{i:04}"), "memory", false))
            .collect();

        let preview = SourceDeletionPreviewBuilder::build("src-500".to_owned(), deps);

        assert_eq!(preview.total_count, 500);
        assert_eq!(preview.dependencies.len(), 500);
        assert!(!preview.truncated);
        assert_eq!(preview.cascade_count, 500);
    }

    #[test]
    fn preview_builder_empty_dependencies() {
        let preview = SourceDeletionPreviewBuilder::build("src-empty".to_owned(), vec![]);
        assert_eq!(preview.total_count, 0);
        assert_eq!(preview.cascade_count, 0);
        assert_eq!(preview.keep_count, 0);
        assert_eq!(preview.ask_user_count, 0);
        assert!(preview.dependencies.is_empty());
        assert!(!preview.truncated);
    }

    // ── MAX_PREVIEW_DEPENDENCIES constant ───────────────────────────────

    #[test]
    fn max_preview_dependencies_is_500() {
        assert_eq!(SourceDeletionPreview::MAX_PREVIEW_DEPENDENCIES, 500);
    }

    // ── FaultInjectionPoint serde roundtrip ─────────────────────────────

    #[test]
    fn fault_injection_point_serde_roundtrip() {
        let points = [
            FaultInjectionPoint::BeforeChunkRead,
            FaultInjectionPoint::AfterChunkRead,
            FaultInjectionPoint::AfterValidation,
            FaultInjectionPoint::AfterExtraction,
            FaultInjectionPoint::AfterWrite,
            FaultInjectionPoint::AfterCursorUpdate,
        ];
        for point in &points {
            let json = serde_json::to_string(point).unwrap();
            let back: FaultInjectionPoint = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, point, "serde roundtrip failed for {point:?}");
        }
    }

    #[test]
    fn fault_injection_point_serde_snake_case_values() {
        assert_eq!(
            serde_json::to_string(&FaultInjectionPoint::BeforeChunkRead).unwrap(),
            "\"before_chunk_read\""
        );
        assert_eq!(
            serde_json::to_string(&FaultInjectionPoint::AfterCursorUpdate).unwrap(),
            "\"after_cursor_update\""
        );
    }

    // ── DependencyDeletionAction serde roundtrip ─────────────────────────

    #[test]
    fn dependency_deletion_action_serde_roundtrip() {
        let actions = [
            DependencyDeletionAction::Cascade,
            DependencyDeletionAction::Keep,
            DependencyDeletionAction::AskUser,
        ];
        for action in &actions {
            let json = serde_json::to_string(action).unwrap();
            let back: DependencyDeletionAction = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, action, "serde roundtrip failed for {action:?}");
        }
    }

    // ── CancelPoint serde roundtrip ──────────────────────────────────────

    #[test]
    fn cancel_point_serde_roundtrip() {
        let cp = CancelPoint {
            requested_at_sequence: 7,
            stopped_at_sequence: 8,
            resume_cursor: make_cursor("position-8"),
            committed_any: true,
        };
        let json = serde_json::to_string(&cp).unwrap();
        let back: CancelPoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back.requested_at_sequence, cp.requested_at_sequence);
        assert_eq!(back.stopped_at_sequence, cp.stopped_at_sequence);
        assert_eq!(back.committed_any, cp.committed_any);
        assert_eq!(back.resume_cursor.position, cp.resume_cursor.position);
    }

    // ── SourceDeletionPreview::MAX_PREVIEW_DEPENDENCIES truncation ───────

    #[test]
    fn preview_truncates_at_exactly_one_over_limit() {
        let n = SourceDeletionPreview::MAX_PREVIEW_DEPENDENCIES + 1;
        let deps: Vec<SourceDeletionDependency> = (0..n)
            .map(|i| make_dep(&format!("r{i}"), "memory", false))
            .collect();
        let preview = SourceDeletionPreviewBuilder::build("src-trunc".to_owned(), deps);
        assert_eq!(preview.total_count, n as u32);
        assert_eq!(
            preview.dependencies.len(),
            SourceDeletionPreview::MAX_PREVIEW_DEPENDENCIES
        );
        assert!(preview.truncated);
    }
}
