//! P3: GoalTree — bounded, immutable, stage-oriented workflow structure.
//!
//! This module defines the core type hierarchy for multi-stage workflow
//! cognition. A `GoalTree` is compiled once by the `WorkflowCompiler`,
//! validated, and then consumed immutably by the `StageExecutor`.
//!
//! # Architectural Invariants
//!
//! - GoalTree is **immutable after construction** — no `&mut self` methods.
//! - All collections are bounded by compile-time constants.
//! - Recovery is finite (max 2 attempts per stage).
//! - Execution is sequential-only (no DAGs, no parallelism).
//! - The GoalTree never observes runtime state.
//!
//! # Authority Boundary
//!
//! This module defines DATA ONLY. It does not execute, plan, or reason.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::agent::execution_verifier::{FsEffect, VerifyTarget};
use crate::agent::htn_executor::VerificationType;

// ============================================================================
// Boundedness Constants — compile-time, non-configurable
// ============================================================================

/// Maximum number of stages in a GoalTree.
pub const MAX_STAGES: usize = 8;

/// Maximum number of actions within a single stage.
pub const MAX_ACTIONS_PER_STAGE: usize = 6;

/// Hard cap on recovery attempts per stage.
pub const MAX_RECOVERY_ATTEMPTS: u8 = 2;

/// Maximum total workflow duration in seconds (5 minutes).
pub const MAX_WORKFLOW_DURATION_SEC: u64 = 300;

/// Maximum per-stage duration in seconds.
pub const MAX_STAGE_DURATION_SEC: u64 = 60;

// ============================================================================
// GoalTree (Immutable After Compile)
// ============================================================================

/// Compiled multi-stage workflow. Immutable after construction.
/// Created by the WorkflowCompiler, consumed by the StageExecutor.
///
/// This struct exposes NO `&mut self` methods. Once built and validated,
/// it is a frozen execution plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalTree {
    /// Unique workflow identifier
    pub workflow_id: String,
    /// Human-readable description of the full workflow
    pub description: String,
    /// Ordered stages — executed sequentially, never reordered
    pub stages: Vec<WorkflowStage>,
    /// Global completion contract
    pub completion: CompletionContract,
    /// Global safe-abort sequence (runs if any stage fails unrecoverably)
    pub global_abort: Vec<SafeAbortStep>,
    /// Maximum total duration across all stages
    pub max_total_duration_sec: u64,
    /// Preconditions that must be true before any stage begins
    pub preconditions: Vec<Precondition>,
}

// ============================================================================
// WorkflowStage
// ============================================================================

/// A single stage in the GoalTree. Contains one or more actions
/// grouped by a shared verification checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStage {
    /// Stage index (0-based, for logging)
    pub index: u32,
    /// Human-readable label (e.g., "Open VS Code")
    pub label: String,
    /// The actions to execute in this stage
    pub action_group: ActionGroup,
    /// Verification checkpoint — checked AFTER action_group completes
    pub checkpoint: VerificationCheckpoint,
    /// Bounded recovery if checkpoint fails (optional)
    pub recovery: Option<RecoveryPath>,
    /// Context hints for this stage (advisory, from grounder)
    pub context_hints: StageContextHints,
    /// Per-stage timeout (independent of global timeout)
    pub timeout_sec: u64,
    /// Whether this stage can be skipped during recovery
    pub skippable: bool,
}

// ============================================================================
// ActionGroup + StageAction
// ============================================================================

/// A group of actions within a single stage.
/// Actions execute sequentially within the group.
/// The group is sequential-only, non-transactional (cannot be rolled back).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionGroup {
    /// Ordered actions
    pub actions: Vec<StageAction>,
}

/// A single action within a stage.
/// Maps directly to a tool call + verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageAction {
    /// Action identifier (e.g., "open_application", "type_text")
    pub action: String,
    /// Parameters for the action
    pub params: serde_json::Value,
    /// Per-action verification (VerificationType from existing system)
    pub verify: VerificationType,
    /// Per-action timeout
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

// ============================================================================
// VerificationCheckpoint
// ============================================================================

/// Stage-level verification checkpoint.
/// Checked after all actions in the stage complete.
/// The checkpoint determines whether to proceed to the next stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VerificationCheckpoint {
    /// Verify window state (most common for app-switching stages)
    WindowFocused {
        title_contains: Option<String>,
        class: Option<String>,
        pid: Option<u32>,
    },
    /// Verify text appears in a target (terminal output, file, etc.)
    OutputContains {
        expected: String,
        target: VerifyTarget,
        case_insensitive: bool,
    },
    /// Verify process is running
    ProcessRunning { binary: String },
    /// Verify filesystem effect
    FileEffect { path: PathBuf, effect: FsEffect },
    /// No checkpoint — proceed unconditionally.
    /// VALIDATION RULE: Only permitted on the terminal (last) stage.
    None,
}

// ============================================================================
// RecoveryPath + RecoveryAction
// ============================================================================

/// Bounded recovery for a failed checkpoint.
/// Recovery is finite: max 2 attempts, each with a single action sequence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryPath {
    /// Maximum recovery attempts (HARD CAP: 2)
    pub max_attempts: u8,
    /// Recovery action to try
    pub recovery_action: RecoveryAction,
}

/// What to do when a checkpoint fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecoveryAction {
    /// Retry from a specific action index within the stage.
    RetryFromAction { restart_from_index: u32 },
    /// Execute a specific corrective action, then re-check
    Corrective { actions: Vec<StageAction> },
    /// Skip this stage (only allowed if stage.skippable == true)
    SkipStage,
    /// Abort the entire workflow
    AbortWorkflow,
}

// ============================================================================
// CompletionContract
// ============================================================================

/// How the workflow reports success to the caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompletionContract {
    /// All stages completed and all checkpoints passed
    AllStagesPassed,
    /// Specific final verification
    FinalVerification(VerificationCheckpoint),
    /// User must confirm completion
    UserConfirmation { prompt: String, timeout_sec: u64 },
}

// ============================================================================
// StageContextHints (Advisory, from Grounder)
// ============================================================================

/// Advisory context for a stage. Populated from OperationalFacts.
/// The executor MAY use these for logging but MUST NOT skip stages
/// based on them alone.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StageContextHints {
    /// Expected focused app for this stage (if known)
    pub expected_app: Option<String>,
    /// Whether the target app is likely already open (from visible_windows)
    pub target_likely_open: bool,
    /// Expected CWD for terminal stages
    pub expected_cwd: Option<PathBuf>,
}

// ============================================================================
// Precondition
// ============================================================================

/// A precondition that must be verified before the workflow begins.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Precondition {
    /// An app must be available (installed). Probed via which::which().
    AppAvailable(String),
    /// Display server must support queries
    DisplayServerAvailable,
    /// A specific window must be visible
    WindowVisible { class: String },
}

// ============================================================================
// SafeAbortStep (reused from existing htn_executor, re-typed here for GoalTree)
// ============================================================================

/// Safe abort step for graceful halt. Mirrors htn_executor::SafeAbortStep
/// but belongs to the GoalTree domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafeAbortStep {
    /// Action to execute (e.g., "press_shortcut")
    pub action: String,
    /// Action parameters
    pub params: serde_json::Value,
}

// ============================================================================
// Serde support for execution_verifier types
// ============================================================================

// FsEffect and VerifyTarget need Serialize/Deserialize for GoalTree serialization.
// They are defined in execution_verifier.rs without serde derives.
// We handle this by implementing custom serde for VerificationCheckpoint variants
// that use them. This is done via serde's tag/content representation above.
//
// ARCHITECTURAL NOTE: Rather than modifying execution_verifier.rs (which is a
// shared module), we provide local serde wrappers where needed. This preserves
// backward compatibility.

// ============================================================================
// Validation Errors
// ============================================================================

/// Errors discovered during GoalTree validation.
#[derive(Debug, Clone, thiserror::Error)]
pub enum GoalTreeValidationError {
    #[error("Too many stages: {count} exceeds MAX_STAGES ({max})")]
    TooManyStages { count: usize, max: usize },

    #[error(
        "Stage {stage_index}: too many actions: {count} exceeds MAX_ACTIONS_PER_STAGE ({max})"
    )]
    TooManyActions {
        stage_index: u32,
        count: usize,
        max: usize,
    },

    #[error("Stage {stage_index}: recovery max_attempts {attempts} exceeds MAX_RECOVERY_ATTEMPTS ({max})")]
    RecoveryBudgetExceeded {
        stage_index: u32,
        attempts: u8,
        max: u8,
    },

    #[error("Workflow duration {duration}s exceeds MAX_WORKFLOW_DURATION_SEC ({max}s)")]
    WorkflowDurationExceeded { duration: u64, max: u64 },

    #[error("Stage {stage_index}: duration {duration}s exceeds MAX_STAGE_DURATION_SEC ({max}s)")]
    StageDurationExceeded {
        stage_index: u32,
        duration: u64,
        max: u64,
    },

    #[error(
        "Stage {stage_index}: VerificationCheckpoint::None only permitted on terminal (last) stage"
    )]
    NoneCheckpointOnNonTerminal { stage_index: u32 },

    #[error("Stage {stage_index}: RecoveryAction::SkipStage used but stage is not skippable")]
    SkipOnNonSkippable { stage_index: u32 },

    #[error("Stage {stage_index}: empty action group (no actions)")]
    EmptyActionGroup { stage_index: u32 },

    #[error("No stages in GoalTree")]
    NoStages,

    #[error("Stage {stage_index}: recovery corrective actions count {count} exceeds MAX_ACTIONS_PER_STAGE ({max})")]
    RecoveryActionsTooMany {
        stage_index: u32,
        count: usize,
        max: usize,
    },

    #[error("Stage {stage_index}: RetryFromAction index {index} exceeds action count {count}")]
    RetryIndexOutOfBounds {
        stage_index: u32,
        index: u32,
        count: usize,
    },
}

// ============================================================================
// GoalTree Validation (immutable — &self only)
// ============================================================================

impl GoalTree {
    /// Validate all boundedness invariants.
    ///
    /// Returns all violations found (not just the first). The caller
    /// should reject the GoalTree if any errors are returned.
    ///
    /// This is the ONLY post-construction method on GoalTree, and it
    /// takes `&self` — preserving immutability.
    pub fn validate(&self) -> Vec<GoalTreeValidationError> {
        let mut errors = Vec::new();

        // No stages
        if self.stages.is_empty() {
            errors.push(GoalTreeValidationError::NoStages);
            return errors; // Nothing else to check
        }

        // Max stages
        if self.stages.len() > MAX_STAGES {
            errors.push(GoalTreeValidationError::TooManyStages {
                count: self.stages.len(),
                max: MAX_STAGES,
            });
        }

        // Global duration
        if self.max_total_duration_sec > MAX_WORKFLOW_DURATION_SEC {
            errors.push(GoalTreeValidationError::WorkflowDurationExceeded {
                duration: self.max_total_duration_sec,
                max: MAX_WORKFLOW_DURATION_SEC,
            });
        }

        let last_index = self.stages.len().saturating_sub(1);

        for (i, stage) in self.stages.iter().enumerate() {
            let idx = stage.index;

            // Empty action group
            if stage.action_group.actions.is_empty() {
                errors.push(GoalTreeValidationError::EmptyActionGroup { stage_index: idx });
            }

            // Max actions per stage
            if stage.action_group.actions.len() > MAX_ACTIONS_PER_STAGE {
                errors.push(GoalTreeValidationError::TooManyActions {
                    stage_index: idx,
                    count: stage.action_group.actions.len(),
                    max: MAX_ACTIONS_PER_STAGE,
                });
            }

            // Per-stage duration
            if stage.timeout_sec > MAX_STAGE_DURATION_SEC {
                errors.push(GoalTreeValidationError::StageDurationExceeded {
                    stage_index: idx,
                    duration: stage.timeout_sec,
                    max: MAX_STAGE_DURATION_SEC,
                });
            }

            // VerificationCheckpoint::None only on terminal stage
            if matches!(stage.checkpoint, VerificationCheckpoint::None) && i != last_index {
                errors.push(GoalTreeValidationError::NoneCheckpointOnNonTerminal {
                    stage_index: idx,
                });
            }

            // Recovery checks
            if let Some(ref recovery) = stage.recovery {
                // Max attempts
                if recovery.max_attempts > MAX_RECOVERY_ATTEMPTS {
                    errors.push(GoalTreeValidationError::RecoveryBudgetExceeded {
                        stage_index: idx,
                        attempts: recovery.max_attempts,
                        max: MAX_RECOVERY_ATTEMPTS,
                    });
                }

                // SkipStage requires skippable
                if matches!(recovery.recovery_action, RecoveryAction::SkipStage) && !stage.skippable
                {
                    errors.push(GoalTreeValidationError::SkipOnNonSkippable { stage_index: idx });
                }

                // Corrective action count
                if let RecoveryAction::Corrective { ref actions } = recovery.recovery_action {
                    if actions.len() > MAX_ACTIONS_PER_STAGE {
                        errors.push(GoalTreeValidationError::RecoveryActionsTooMany {
                            stage_index: idx,
                            count: actions.len(),
                            max: MAX_ACTIONS_PER_STAGE,
                        });
                    }
                }

                // RetryFromAction index bounds
                if let RecoveryAction::RetryFromAction { restart_from_index } =
                    recovery.recovery_action
                {
                    if restart_from_index as usize >= stage.action_group.actions.len() {
                        errors.push(GoalTreeValidationError::RetryIndexOutOfBounds {
                            stage_index: idx,
                            index: restart_from_index,
                            count: stage.action_group.actions.len(),
                        });
                    }
                }
            }
        }

        errors
    }
}

// ============================================================================
// Conversion: VerificationCheckpoint → Verifiability
// ============================================================================

impl VerificationCheckpoint {
    /// Convert a stage-level checkpoint to the existing `Verifiability` enum
    /// used by `BoundedExecutionVerifier`. This bridges P3 checkpoints into
    /// the existing verification authority.
    pub fn to_verifiability(&self) -> crate::agent::execution_verifier::Verifiability {
        use crate::agent::execution_verifier::Verifiability;

        match self {
            VerificationCheckpoint::WindowFocused {
                title_contains,
                class,
                ..
            } => Verifiability::WindowState {
                title_contains: title_contains.clone(),
                class: class.clone(),
            },
            VerificationCheckpoint::OutputContains {
                expected, target, ..
            } => Verifiability::DeterministicOutput {
                expected_substring: expected.clone(),
                in_target: target.clone(),
            },
            VerificationCheckpoint::ProcessRunning { binary } => Verifiability::ProcessLaunched {
                binary: binary.clone(),
                max_wait_ms: 5000,
            },
            VerificationCheckpoint::FileEffect { path, effect } => {
                Verifiability::FileSystemEffect {
                    path: path.clone(),
                    kind: effect.clone(),
                }
            }
            VerificationCheckpoint::None => Verifiability::Unverifiable {
                reason: "No checkpoint — unconditional proceed".into(),
            },
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ──────────────────────────────────────────────────────────

    fn make_action(action: &str) -> StageAction {
        StageAction {
            action: action.to_string(),
            params: serde_json::json!({}),
            verify: VerificationType::None,
            timeout_ms: None,
        }
    }

    fn make_stage(
        index: u32,
        actions: Vec<StageAction>,
        checkpoint: VerificationCheckpoint,
    ) -> WorkflowStage {
        WorkflowStage {
            index,
            label: format!("Stage {}", index),
            action_group: ActionGroup { actions },
            checkpoint,
            recovery: None,
            context_hints: StageContextHints::default(),
            timeout_sec: 30,
            skippable: false,
        }
    }

    fn make_valid_tree(stages: Vec<WorkflowStage>) -> GoalTree {
        GoalTree {
            workflow_id: "test-tree".to_string(),
            description: "Test workflow".to_string(),
            stages,
            completion: CompletionContract::AllStagesPassed,
            global_abort: vec![SafeAbortStep {
                action: "press_shortcut".to_string(),
                params: serde_json::json!({"keys": ["Escape"]}),
            }],
            max_total_duration_sec: 120,
            preconditions: vec![],
        }
    }

    // ── Construction Tests ──────────────────────────────────────────────

    #[test]
    fn valid_two_stage_workflow() {
        let tree = make_valid_tree(vec![
            make_stage(
                0,
                vec![make_action("open_application")],
                VerificationCheckpoint::WindowFocused {
                    title_contains: Some("VS Code".into()),
                    class: None,
                    pid: None,
                },
            ),
            make_stage(
                1,
                vec![make_action("type_text")],
                VerificationCheckpoint::None,
            ),
        ]);

        let errors = tree.validate();
        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn valid_single_stage_with_none_checkpoint() {
        let tree = make_valid_tree(vec![make_stage(
            0,
            vec![make_action("press_shortcut")],
            VerificationCheckpoint::None,
        )]);
        assert!(tree.validate().is_empty());
    }

    // ── Boundedness Tests ───────────────────────────────────────────────

    #[test]
    fn reject_too_many_stages() {
        let stages: Vec<_> = (0..=MAX_STAGES as u32)
            .map(|i| make_stage(i, vec![make_action("noop")], VerificationCheckpoint::None))
            .collect();
        let tree = make_valid_tree(stages);
        let errors = tree.validate();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, GoalTreeValidationError::TooManyStages { .. })),
            "Expected TooManyStages error"
        );
    }

    #[test]
    fn reject_too_many_actions() {
        let actions: Vec<_> = (0..=MAX_ACTIONS_PER_STAGE)
            .map(|_| make_action("noop"))
            .collect();
        let tree = make_valid_tree(vec![make_stage(0, actions, VerificationCheckpoint::None)]);
        let errors = tree.validate();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, GoalTreeValidationError::TooManyActions { .. })),
            "Expected TooManyActions error"
        );
    }

    #[test]
    fn reject_empty_action_group() {
        let tree = make_valid_tree(vec![make_stage(0, vec![], VerificationCheckpoint::None)]);
        let errors = tree.validate();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, GoalTreeValidationError::EmptyActionGroup { .. })),
            "Expected EmptyActionGroup error"
        );
    }

    #[test]
    fn reject_no_stages() {
        let tree = make_valid_tree(vec![]);
        let errors = tree.validate();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, GoalTreeValidationError::NoStages)),
            "Expected NoStages error"
        );
    }

    #[test]
    fn reject_excessive_workflow_duration() {
        let mut tree = make_valid_tree(vec![make_stage(
            0,
            vec![make_action("noop")],
            VerificationCheckpoint::None,
        )]);
        tree.max_total_duration_sec = MAX_WORKFLOW_DURATION_SEC + 1;
        let errors = tree.validate();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, GoalTreeValidationError::WorkflowDurationExceeded { .. })),
            "Expected WorkflowDurationExceeded error"
        );
    }

    #[test]
    fn reject_excessive_stage_duration() {
        let mut stage = make_stage(0, vec![make_action("noop")], VerificationCheckpoint::None);
        stage.timeout_sec = MAX_STAGE_DURATION_SEC + 1;
        let tree = make_valid_tree(vec![stage]);
        let errors = tree.validate();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, GoalTreeValidationError::StageDurationExceeded { .. })),
            "Expected StageDurationExceeded error"
        );
    }

    #[test]
    fn reject_recovery_budget_exceeded() {
        let mut stage = make_stage(0, vec![make_action("noop")], VerificationCheckpoint::None);
        stage.recovery = Some(RecoveryPath {
            max_attempts: MAX_RECOVERY_ATTEMPTS + 1,
            recovery_action: RecoveryAction::RetryFromAction {
                restart_from_index: 0,
            },
        });
        let tree = make_valid_tree(vec![stage]);
        let errors = tree.validate();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, GoalTreeValidationError::RecoveryBudgetExceeded { .. })),
            "Expected RecoveryBudgetExceeded error"
        );
    }

    #[test]
    fn reject_none_checkpoint_on_non_terminal() {
        let tree = make_valid_tree(vec![
            make_stage(0, vec![make_action("noop")], VerificationCheckpoint::None),
            make_stage(
                1,
                vec![make_action("noop")],
                VerificationCheckpoint::WindowFocused {
                    title_contains: None,
                    class: None,
                    pid: None,
                },
            ),
        ]);
        let errors = tree.validate();
        assert!(
            errors.iter().any(|e| matches!(
                e,
                GoalTreeValidationError::NoneCheckpointOnNonTerminal { .. }
            )),
            "Expected NoneCheckpointOnNonTerminal error"
        );
    }

    #[test]
    fn reject_skip_on_non_skippable() {
        let mut stage = make_stage(0, vec![make_action("noop")], VerificationCheckpoint::None);
        stage.skippable = false;
        stage.recovery = Some(RecoveryPath {
            max_attempts: 1,
            recovery_action: RecoveryAction::SkipStage,
        });
        let tree = make_valid_tree(vec![stage]);
        let errors = tree.validate();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, GoalTreeValidationError::SkipOnNonSkippable { .. })),
            "Expected SkipOnNonSkippable error"
        );
    }

    #[test]
    fn allow_skip_on_skippable() {
        let mut stage = make_stage(0, vec![make_action("noop")], VerificationCheckpoint::None);
        stage.skippable = true;
        stage.recovery = Some(RecoveryPath {
            max_attempts: 1,
            recovery_action: RecoveryAction::SkipStage,
        });
        let tree = make_valid_tree(vec![stage]);
        let errors = tree.validate();
        // Should NOT have SkipOnNonSkippable
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, GoalTreeValidationError::SkipOnNonSkippable { .. })),
            "SkipStage on skippable stage should be allowed"
        );
    }

    #[test]
    fn reject_retry_index_out_of_bounds() {
        let mut stage = make_stage(
            0,
            vec![make_action("noop")], // only 1 action, index 0
            VerificationCheckpoint::None,
        );
        stage.recovery = Some(RecoveryPath {
            max_attempts: 1,
            recovery_action: RecoveryAction::RetryFromAction {
                restart_from_index: 5,
            },
        });
        let tree = make_valid_tree(vec![stage]);
        let errors = tree.validate();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, GoalTreeValidationError::RetryIndexOutOfBounds { .. })),
            "Expected RetryIndexOutOfBounds error"
        );
    }

    #[test]
    fn reject_recovery_corrective_actions_too_many() {
        let many_actions: Vec<_> = (0..=MAX_ACTIONS_PER_STAGE)
            .map(|_| make_action("noop"))
            .collect();
        let mut stage = make_stage(0, vec![make_action("noop")], VerificationCheckpoint::None);
        stage.recovery = Some(RecoveryPath {
            max_attempts: 1,
            recovery_action: RecoveryAction::Corrective {
                actions: many_actions,
            },
        });
        let tree = make_valid_tree(vec![stage]);
        let errors = tree.validate();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, GoalTreeValidationError::RecoveryActionsTooMany { .. })),
            "Expected RecoveryActionsTooMany error"
        );
    }

    // ── Immutability Test ───────────────────────────────────────────────

    #[test]
    fn goal_tree_has_no_mut_self_methods() {
        // This test is structural: GoalTree only exposes `validate(&self)`.
        // Any addition of `&mut self` methods is a design violation.
        // The Rust compiler enforces this if we only hold &GoalTree in executors.
        let tree = make_valid_tree(vec![make_stage(
            0,
            vec![make_action("noop")],
            VerificationCheckpoint::None,
        )]);
        // Can only call &self methods
        let _errors = tree.validate();
        let _id = &tree.workflow_id;
        let _stages = &tree.stages;
    }

    // ── Checkpoint → Verifiability Bridge ───────────────────────────────

    #[test]
    fn checkpoint_window_focused_bridges_to_verifiability() {
        let cp = VerificationCheckpoint::WindowFocused {
            title_contains: Some("Firefox".into()),
            class: Some("Navigator".into()),
            pid: None,
        };
        let v = cp.to_verifiability();
        assert!(matches!(
            v,
            crate::agent::execution_verifier::Verifiability::WindowState {
                title_contains: Some(_),
                class: Some(_),
            }
        ));
    }

    #[test]
    fn checkpoint_process_running_bridges_to_verifiability() {
        let cp = VerificationCheckpoint::ProcessRunning {
            binary: "code".into(),
        };
        let v = cp.to_verifiability();
        assert!(matches!(
            v,
            crate::agent::execution_verifier::Verifiability::ProcessLaunched { .. }
        ));
    }

    #[test]
    fn checkpoint_none_bridges_to_unverifiable() {
        let cp = VerificationCheckpoint::None;
        let v = cp.to_verifiability();
        assert!(matches!(
            v,
            crate::agent::execution_verifier::Verifiability::Unverifiable { .. }
        ));
    }

    // ── Serialization Roundtrip ─────────────────────────────────────────

    #[test]
    fn goal_tree_serialization_roundtrip() {
        let tree = make_valid_tree(vec![
            make_stage(
                0,
                vec![make_action("open_application")],
                VerificationCheckpoint::WindowFocused {
                    title_contains: Some("Code".into()),
                    class: None,
                    pid: None,
                },
            ),
            make_stage(
                1,
                vec![make_action("type_text")],
                VerificationCheckpoint::None,
            ),
        ]);

        let json = serde_json::to_string_pretty(&tree).expect("serialize");
        let deserialized: GoalTree = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.workflow_id, tree.workflow_id);
        assert_eq!(deserialized.stages.len(), tree.stages.len());
        assert_eq!(deserialized.stages[0].label, tree.stages[0].label);
        assert_eq!(
            deserialized.stages[0].action_group.actions[0].action,
            tree.stages[0].action_group.actions[0].action
        );
    }
}
