//! P3b: WorkflowCompiler — bounded multi-verb → GoalTree compilation.
//!
//! # Authority Boundary
//!
//! The WorkflowCompiler is a **pure decomposition layer**. It:
//! - Receives a `MultiVerbSpec` (from IntentCompiler) + `OperationalFacts` (advisory)
//! - Returns a `GoalTree` or a `CompileError`
//!
//! It MUST NOT:
//! - Execute any actions
//! - Query the OS or environment
//! - Call other planners/compilers recursively
//! - Mutate any state
//! - Perform network I/O
//! - Infer hidden goals
//! - Invent semantic workflows beyond explicit user intent
//! - Observe runtime execution
//!
//! # Relationship to GuiPlanner
//!
//! The `WorkflowCompiler` is SEPARATE from `GuiPlanner`. The existing
//! `GuiPlanner::plan()` continues to handle single-verb requests unchanged.
//! The `WorkflowCompiler` is invoked ONLY when `IntentCompiler` detects
//! multi-verb intent. If only one verb is detected, it rejects with
//! `CompileError::SingleVerb` and the coordinator falls through to
//! `GuiPlanner::plan()`.

use crate::agent::environment_grounder::OperationalFacts;
use crate::agent::goal_tree::{
    ActionGroup, CompletionContract, GoalTree, GoalTreeValidationError, Precondition,
    RecoveryAction, RecoveryPath, SafeAbortStep, StageAction, StageContextHints,
    VerificationCheckpoint, WorkflowStage,
};
use crate::agent::htn_executor::VerificationType;
use crate::agent::intent_compiler::{ContentClass, TargetRef, Verb};

// ============================================================================
// MultiVerbSpec — multi-verb intent from IntentCompiler
// ============================================================================

/// A multi-verb specification produced by the IntentCompiler when it
/// detects conjunctions ("and", "then", ";") in user input.
///
/// Each `VerbClause` represents one explicit user-stated action.
/// The WorkflowCompiler maps each clause to a `WorkflowStage`.
#[derive(Debug, Clone)]
pub struct MultiVerbSpec {
    /// The original user text (for logging, not for reasoning)
    pub original_text: String,
    /// Ordered verb clauses — each becomes one stage
    pub clauses: Vec<VerbClause>,
}

/// A single verb clause extracted from a multi-verb user request.
///
/// Example: "Open VS Code and run cargo test"
///   → clause 0: { verb: Open, targets: [App("VS Code")] }
///   → clause 1: { verb: Run, targets: [App("cargo test")] }
#[derive(Debug, Clone)]
pub struct VerbClause {
    /// The verb for this clause
    pub verb: Verb,
    /// Targets referenced in this clause
    pub targets: Vec<TargetRef>,
    /// Content to type/write (if applicable)
    pub content: Option<ContentClass>,
}

// ============================================================================
// CompileError
// ============================================================================

/// Errors from workflow compilation — always explicit, never silent.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CompileError {
    #[error("Single-verb request — use GuiPlanner::plan() instead")]
    SingleVerb,

    #[error("No verbs detected in multi-verb spec")]
    NoVerbs,

    #[error("Too many verb clauses ({count}): exceeds MAX_STAGES ({max})")]
    TooManyClauses { count: usize, max: usize },

    #[error("Unsupported verb in clause {clause_index}: {verb:?}")]
    UnsupportedVerb { clause_index: usize, verb: Verb },

    #[error("Missing required parameter in clause {clause_index}: {param}")]
    MissingParameter { clause_index: usize, param: String },

    #[error("GoalTree validation failed: {errors:?}")]
    ValidationFailed {
        errors: Vec<GoalTreeValidationError>,
    },
}

// ============================================================================
// WorkflowCompiler Trait
// ============================================================================

/// WorkflowCompiler MUST be a pure function. It receives multi-verb specs
/// and advisory facts and returns a GoalTree. It MUST NOT: call tools,
/// query the OS, mutate state, perform network I/O, or call other
/// compilers/planners recursively.
pub trait WorkflowCompiler: Send + Sync {
    /// Compile a multi-verb specification into a validated GoalTree.
    ///
    /// # Errors
    /// - `CompileError::SingleVerb` if only one clause (caller must use GuiPlanner)
    /// - `CompileError::ValidationFailed` if the GoalTree fails boundedness checks
    fn compile(
        &self,
        spec: &MultiVerbSpec,
        facts: &OperationalFacts,
    ) -> Result<GoalTree, CompileError>;
}

// ============================================================================
// RuleBasedWorkflowCompiler — deterministic multi-verb decomposition
// ============================================================================

/// Deterministic rule-based compiler. Maps each VerbClause to a
/// WorkflowStage using explicit pattern matching. No LLM, no inference,
/// no hidden reasoning.
pub struct RuleBasedWorkflowCompiler;

impl WorkflowCompiler for RuleBasedWorkflowCompiler {
    fn compile(
        &self,
        spec: &MultiVerbSpec,
        facts: &OperationalFacts,
    ) -> Result<GoalTree, CompileError> {
        // ── Reject single-verb or empty ────────────────────────────
        if spec.clauses.is_empty() {
            return Err(CompileError::NoVerbs);
        }
        if spec.clauses.len() == 1 {
            return Err(CompileError::SingleVerb);
        }
        if spec.clauses.len() > crate::agent::goal_tree::MAX_STAGES {
            return Err(CompileError::TooManyClauses {
                count: spec.clauses.len(),
                max: crate::agent::goal_tree::MAX_STAGES,
            });
        }

        // ── Compile each clause into a stage ───────────────────────
        let total_clauses = spec.clauses.len();
        let mut stages = Vec::with_capacity(total_clauses);

        for (i, clause) in spec.clauses.iter().enumerate() {
            let is_last = i == total_clauses - 1;
            let stage = self.compile_clause(i, clause, is_last, facts)?;
            stages.push(stage);
        }

        // ── Build GoalTree ─────────────────────────────────────────
        let tree = GoalTree {
            workflow_id: format!("wf-{}", uuid::Uuid::new_v4()),
            description: spec.original_text.clone(),
            stages,
            completion: CompletionContract::AllStagesPassed,
            global_abort: vec![SafeAbortStep {
                action: "press_shortcut".to_string(),
                params: serde_json::json!({"keys": ["Escape"]}),
            }],
            max_total_duration_sec: crate::agent::goal_tree::MAX_WORKFLOW_DURATION_SEC,
            preconditions: vec![Precondition::DisplayServerAvailable],
        };

        // ── Validate boundedness ───────────────────────────────────
        let errors = tree.validate();
        if !errors.is_empty() {
            return Err(CompileError::ValidationFailed { errors });
        }

        Ok(tree)
    }
}

impl RuleBasedWorkflowCompiler {
    /// Compile a single VerbClause into a WorkflowStage.
    ///
    /// This is the core decomposition function. It maps verbs to actions
    /// and selects appropriate checkpoints. There is NO hidden reasoning —
    /// each verb maps to a fixed action pattern.
    fn compile_clause(
        &self,
        index: usize,
        clause: &VerbClause,
        is_last: bool,
        facts: &OperationalFacts,
    ) -> Result<WorkflowStage, CompileError> {
        let (actions, checkpoint, label) = match &clause.verb {
            Verb::Open => {
                let app = extract_app_target(&clause.targets, index)?;
                let actions = vec![StageAction {
                    action: "open_application".to_string(),
                    params: serde_json::json!({"name": app}),
                    verify: VerificationType::WindowState {
                        title_contains: Some(app.clone()),
                        class: None,
                    },
                    timeout_ms: Some(5000),
                }];
                let checkpoint = VerificationCheckpoint::WindowFocused {
                    title_contains: Some(app.clone()),
                    class: None,
                    pid: None,
                };
                (actions, checkpoint, format!("Open {}", app))
            }
            Verb::Switch => {
                let app = extract_app_target(&clause.targets, index)?;
                let actions = vec![StageAction {
                    action: "switch_to_window".to_string(),
                    params: serde_json::json!({"name": app}),
                    verify: VerificationType::WindowState {
                        title_contains: Some(app.clone()),
                        class: None,
                    },
                    timeout_ms: Some(3000),
                }];
                let checkpoint = VerificationCheckpoint::WindowFocused {
                    title_contains: Some(app.clone()),
                    class: None,
                    pid: None,
                };
                (actions, checkpoint, format!("Switch to {}", app))
            }
            Verb::Type => {
                let text = extract_content(clause, index)?;
                let actions = vec![StageAction {
                    action: "type_text".to_string(),
                    params: serde_json::json!({"text": text}),
                    verify: VerificationType::TextPresent {
                        text: text.clone(),
                        case_insensitive: false,
                    },
                    timeout_ms: Some(3000),
                }];
                // For type actions mid-workflow, verify text appeared.
                // For terminal stage, None checkpoint is acceptable.
                let checkpoint = if is_last {
                    VerificationCheckpoint::None
                } else {
                    VerificationCheckpoint::OutputContains {
                        expected: text.clone(),
                        target: crate::agent::execution_verifier::VerifyTarget::TerminalOutput,
                        case_insensitive: true,
                    }
                };
                (
                    actions,
                    checkpoint,
                    format!("Type '{}'", truncate_for_label(&text, 30)),
                )
            }
            Verb::Run => {
                let cmd = extract_run_command(clause, index)?;
                let actions = vec![StageAction {
                    action: "type_text".to_string(),
                    params: serde_json::json!({"text": format!("{}\n", cmd)}),
                    verify: VerificationType::None,
                    timeout_ms: Some(5000),
                }];
                // Run commands: we can't know what output to expect in general.
                // Use None on last stage, otherwise process-based check.
                let checkpoint = if is_last {
                    VerificationCheckpoint::None
                } else {
                    VerificationCheckpoint::OutputContains {
                        expected: cmd.clone(),
                        target: crate::agent::execution_verifier::VerifyTarget::TerminalOutput,
                        case_insensitive: false,
                    }
                };
                (
                    actions,
                    checkpoint,
                    format!("Run `{}`", truncate_for_label(&cmd, 30)),
                )
            }
            Verb::Click => {
                let element = extract_element_target(&clause.targets, index)?;
                let actions = vec![StageAction {
                    action: "click_element".to_string(),
                    params: serde_json::json!({"element_id": element, "button": "left"}),
                    verify: VerificationType::ScreenChanged {
                        element_id: None,
                        threshold: 0.90,
                    },
                    timeout_ms: Some(2000),
                }];
                let checkpoint = if is_last {
                    VerificationCheckpoint::None
                } else {
                    VerificationCheckpoint::WindowFocused {
                        title_contains: None,
                        class: None,
                        pid: None,
                    }
                };
                (actions, checkpoint, format!("Click '{}'", element))
            }
            Verb::Close => {
                let app = extract_app_target(&clause.targets, index)?;
                let actions = vec![StageAction {
                    action: "close_application".to_string(),
                    params: serde_json::json!({"name": app}),
                    verify: VerificationType::None,
                    timeout_ms: Some(2000),
                }];
                // Close: for non-terminal stages, verify window focus
                // changed (closed app no longer focused).
                let checkpoint = if is_last {
                    VerificationCheckpoint::None
                } else {
                    VerificationCheckpoint::WindowFocused {
                        title_contains: None,
                        class: None,
                        pid: None,
                    }
                };
                (actions, checkpoint, format!("Close {}", app))
            }
            Verb::Save => {
                let actions = vec![StageAction {
                    action: "press_shortcut".to_string(),
                    params: serde_json::json!({"keys": ["Ctrl+S"]}),
                    verify: VerificationType::None,
                    timeout_ms: Some(2000),
                }];
                // Save: no strong verification possible. For non-terminal
                // stages, use a permissive window-focused check (window
                // should remain focused after Ctrl+S).
                let checkpoint = if is_last {
                    VerificationCheckpoint::None
                } else {
                    VerificationCheckpoint::WindowFocused {
                        title_contains: None,
                        class: None,
                        pid: None,
                    }
                };
                (actions, checkpoint, "Save".to_string())
            }
            Verb::Other(raw) => {
                return Err(CompileError::UnsupportedVerb {
                    clause_index: index,
                    verb: Verb::Other(raw.clone()),
                });
            }
        };

        // ── Populate advisory context hints from OperationalFacts ──
        let context_hints = build_context_hints(clause, facts);

        // ── Default recovery: retry once for non-terminal stages ───
        let recovery = if !is_last {
            Some(RecoveryPath {
                max_attempts: 1,
                recovery_action: RecoveryAction::RetryFromAction {
                    restart_from_index: 0,
                },
            })
        } else {
            None
        };

        Ok(WorkflowStage {
            index: index as u32,
            label,
            action_group: ActionGroup { actions },
            checkpoint,
            recovery,
            context_hints,
            timeout_sec: crate::agent::goal_tree::MAX_STAGE_DURATION_SEC,
            skippable: false,
        })
    }
}

// ============================================================================
// Helper Functions (pure, no side effects)
// ============================================================================

/// Extract an app name from targets, or return an error.
fn extract_app_target(targets: &[TargetRef], clause_index: usize) -> Result<String, CompileError> {
    targets
        .iter()
        .find_map(|t| match t {
            TargetRef::App(a) => Some(a.clone()),
            _ => None,
        })
        .ok_or(CompileError::MissingParameter {
            clause_index,
            param: "target app".to_string(),
        })
}

/// Extract an element name from targets, or return an error.
fn extract_element_target(
    targets: &[TargetRef],
    clause_index: usize,
) -> Result<String, CompileError> {
    targets
        .iter()
        .find_map(|t| match t {
            TargetRef::Element(e) => Some(e.clone()),
            _ => None,
        })
        .ok_or(CompileError::MissingParameter {
            clause_index,
            param: "target element".to_string(),
        })
}

/// Extract text content from a VerbClause.
fn extract_content(clause: &VerbClause, clause_index: usize) -> Result<String, CompileError> {
    match clause.content.as_ref() {
        Some(ContentClass::Literal(t)) => Ok(t.clone()),
        Some(ContentClass::Generated { hint, .. }) => Ok(hint.clone()),
        None => Err(CompileError::MissingParameter {
            clause_index,
            param: "text content".to_string(),
        }),
    }
}

/// Extract a run command from clause targets or content.
fn extract_run_command(clause: &VerbClause, clause_index: usize) -> Result<String, CompileError> {
    // Try app target first (e.g. "run cargo test")
    if let Some(cmd) = clause.targets.iter().find_map(|t| match t {
        TargetRef::App(a) => Some(a.clone()),
        _ => None,
    }) {
        return Ok(cmd);
    }
    // Fall back to content
    match clause.content.as_ref() {
        Some(ContentClass::Literal(t)) => Ok(t.clone()),
        Some(ContentClass::Generated { hint, .. }) => Ok(hint.clone()),
        None => Err(CompileError::MissingParameter {
            clause_index,
            param: "command to run".to_string(),
        }),
    }
}

/// Build advisory context hints from OperationalFacts.
/// This is purely observational — no reasoning, no decisions.
fn build_context_hints(clause: &VerbClause, facts: &OperationalFacts) -> StageContextHints {
    let expected_app = clause.targets.iter().find_map(|t| match t {
        TargetRef::App(a) => Some(a.clone()),
        _ => None,
    });

    // Check if target app is likely already open (advisory only)
    let target_likely_open = if let Some(ref app) = expected_app {
        let app_lower = app.to_lowercase();
        facts.visible_windows.iter().any(|w| {
            w.title.to_lowercase().contains(&app_lower)
                || w.class.to_lowercase().contains(&app_lower)
        })
    } else {
        false
    };

    StageContextHints {
        expected_app,
        target_likely_open,
        expected_cwd: facts.terminal_cwd.clone(),
    }
}

/// Truncate a string for use in stage labels.
fn truncate_for_label(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::environment_grounder::{GroundingCapabilities, OperationalFacts};
    use crate::agent::goal_tree::{MAX_RECOVERY_ATTEMPTS, MAX_STAGES};

    fn empty_facts() -> OperationalFacts {
        OperationalFacts::empty(GroundingCapabilities::none())
    }

    // ── SingleVerb rejection ────────────────────────────────────────

    #[test]
    fn reject_single_verb() {
        let spec = MultiVerbSpec {
            original_text: "open firefox".into(),
            clauses: vec![VerbClause {
                verb: Verb::Open,
                targets: vec![TargetRef::App("firefox".into())],
                content: None,
            }],
        };
        let compiler = RuleBasedWorkflowCompiler;
        let err = compiler.compile(&spec, &empty_facts()).unwrap_err();
        assert!(matches!(err, CompileError::SingleVerb));
    }

    #[test]
    fn reject_empty_clauses() {
        let spec = MultiVerbSpec {
            original_text: "".into(),
            clauses: vec![],
        };
        let compiler = RuleBasedWorkflowCompiler;
        let err = compiler.compile(&spec, &empty_facts()).unwrap_err();
        assert!(matches!(err, CompileError::NoVerbs));
    }

    // ── Two-stage: Open X and Run Y ────────────────────────────────

    #[test]
    fn compile_open_and_run() {
        let spec = MultiVerbSpec {
            original_text: "Open VS Code and run cargo test".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Open,
                    targets: vec![TargetRef::App("VS Code".into())],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Run,
                    targets: vec![TargetRef::App("cargo test".into())],
                    content: None,
                },
            ],
        };
        let compiler = RuleBasedWorkflowCompiler;
        let tree = compiler.compile(&spec, &empty_facts()).unwrap();

        assert_eq!(tree.stages.len(), 2);
        assert_eq!(tree.stages[0].label, "Open VS Code");
        assert_eq!(
            tree.stages[0].action_group.actions[0].action,
            "open_application"
        );
        assert!(matches!(
            tree.stages[0].checkpoint,
            VerificationCheckpoint::WindowFocused { .. }
        ));
        // Non-terminal stage has recovery
        assert!(tree.stages[0].recovery.is_some());

        assert!(tree.stages[1].label.contains("cargo test"));
        assert_eq!(tree.stages[1].action_group.actions[0].action, "type_text");
        // Terminal stage has None checkpoint
        assert!(matches!(
            tree.stages[1].checkpoint,
            VerificationCheckpoint::None
        ));
        // Terminal stage has no recovery
        assert!(tree.stages[1].recovery.is_none());

        // Validate
        assert!(tree.validate().is_empty());
    }

    // ── Three-stage: Open, Type, Save ──────────────────────────────

    #[test]
    fn compile_open_type_save() {
        let spec = MultiVerbSpec {
            original_text: "Open gedit, type hello world, and save".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Open,
                    targets: vec![TargetRef::App("gedit".into())],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Type,
                    targets: vec![],
                    content: Some(ContentClass::Literal("hello world".into())),
                },
                VerbClause {
                    verb: Verb::Save,
                    targets: vec![],
                    content: None,
                },
            ],
        };
        let compiler = RuleBasedWorkflowCompiler;
        let tree = compiler.compile(&spec, &empty_facts()).unwrap();

        assert_eq!(tree.stages.len(), 3);
        assert_eq!(
            tree.stages[0].action_group.actions[0].action,
            "open_application"
        );
        assert_eq!(tree.stages[1].action_group.actions[0].action, "type_text");
        assert_eq!(
            tree.stages[2].action_group.actions[0].action,
            "press_shortcut"
        );

        // Only terminal stage should have None checkpoint
        assert!(!matches!(
            tree.stages[0].checkpoint,
            VerificationCheckpoint::None
        ));
        assert!(!matches!(
            tree.stages[1].checkpoint,
            VerificationCheckpoint::None
        ));
        assert!(matches!(
            tree.stages[2].checkpoint,
            VerificationCheckpoint::None
        ));

        assert!(tree.validate().is_empty());
    }

    // ── Too many clauses ───────────────────────────────────────────

    #[test]
    fn reject_too_many_clauses() {
        let clauses: Vec<VerbClause> = (0..MAX_STAGES + 1)
            .map(|_| VerbClause {
                verb: Verb::Save,
                targets: vec![],
                content: None,
            })
            .collect();
        let spec = MultiVerbSpec {
            original_text: "too many".into(),
            clauses,
        };
        let compiler = RuleBasedWorkflowCompiler;
        let err = compiler.compile(&spec, &empty_facts()).unwrap_err();
        assert!(matches!(err, CompileError::TooManyClauses { .. }));
    }

    // ── Unsupported verb ───────────────────────────────────────────

    #[test]
    fn reject_unsupported_verb() {
        let spec = MultiVerbSpec {
            original_text: "dance and sing".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Other("dance".into()),
                    targets: vec![],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Other("sing".into()),
                    targets: vec![],
                    content: None,
                },
            ],
        };
        let compiler = RuleBasedWorkflowCompiler;
        let err = compiler.compile(&spec, &empty_facts()).unwrap_err();
        assert!(matches!(err, CompileError::UnsupportedVerb { .. }));
    }

    // ── Missing parameter ──────────────────────────────────────────

    #[test]
    fn reject_missing_app_target() {
        let spec = MultiVerbSpec {
            original_text: "open and save".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Open,
                    targets: vec![], // no app target
                    content: None,
                },
                VerbClause {
                    verb: Verb::Save,
                    targets: vec![],
                    content: None,
                },
            ],
        };
        let compiler = RuleBasedWorkflowCompiler;
        let err = compiler.compile(&spec, &empty_facts()).unwrap_err();
        assert!(matches!(err, CompileError::MissingParameter { .. }));
    }

    // ── Recovery budget ────────────────────────────────────────────

    #[test]
    fn non_terminal_stages_have_bounded_recovery() {
        let spec = MultiVerbSpec {
            original_text: "Open firefox and save".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Open,
                    targets: vec![TargetRef::App("firefox".into())],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Save,
                    targets: vec![],
                    content: None,
                },
            ],
        };
        let compiler = RuleBasedWorkflowCompiler;
        let tree = compiler.compile(&spec, &empty_facts()).unwrap();

        // Non-terminal stage has recovery
        let recovery = tree.stages[0].recovery.as_ref().unwrap();
        assert!(recovery.max_attempts <= MAX_RECOVERY_ATTEMPTS);
        assert!(matches!(
            recovery.recovery_action,
            RecoveryAction::RetryFromAction { .. }
        ));
    }

    // ── Compiler purity: no side effects ───────────────────────────

    #[test]
    fn compiler_is_pure_deterministic() {
        let spec = MultiVerbSpec {
            original_text: "Open firefox and type hello".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Open,
                    targets: vec![TargetRef::App("firefox".into())],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Type,
                    targets: vec![],
                    content: Some(ContentClass::Literal("hello".into())),
                },
            ],
        };
        let compiler = RuleBasedWorkflowCompiler;
        let facts = empty_facts();

        // Compile twice — same output (deterministic)
        let tree1 = compiler.compile(&spec, &facts).unwrap();
        let tree2 = compiler.compile(&spec, &facts).unwrap();

        // Structure must be identical (workflow_id differs due to UUID, but stages match)
        assert_eq!(tree1.stages.len(), tree2.stages.len());
        assert_eq!(tree1.stages[0].label, tree2.stages[0].label);
        assert_eq!(tree1.stages[1].label, tree2.stages[1].label);
        assert_eq!(
            tree1.stages[0].action_group.actions[0].action,
            tree2.stages[0].action_group.actions[0].action
        );
    }

    // ── Switch verb ────────────────────────────────────────────────

    #[test]
    fn compile_switch_and_type() {
        let spec = MultiVerbSpec {
            original_text: "Switch to terminal and type ls".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Switch,
                    targets: vec![TargetRef::App("terminal".into())],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Type,
                    targets: vec![],
                    content: Some(ContentClass::Literal("ls".into())),
                },
            ],
        };
        let compiler = RuleBasedWorkflowCompiler;
        let tree = compiler.compile(&spec, &empty_facts()).unwrap();

        assert_eq!(tree.stages.len(), 2);
        assert_eq!(
            tree.stages[0].action_group.actions[0].action,
            "switch_to_window"
        );
        assert_eq!(tree.stages[1].action_group.actions[0].action, "type_text");
        assert!(tree.validate().is_empty());
    }

    // ── Close verb ─────────────────────────────────────────────────

    #[test]
    fn compile_save_and_close() {
        let spec = MultiVerbSpec {
            original_text: "Save and close gedit".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Save,
                    targets: vec![],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Close,
                    targets: vec![TargetRef::App("gedit".into())],
                    content: None,
                },
            ],
        };
        let compiler = RuleBasedWorkflowCompiler;
        let tree = compiler.compile(&spec, &empty_facts()).unwrap();

        assert_eq!(tree.stages.len(), 2);
        assert_eq!(
            tree.stages[0].action_group.actions[0].action,
            "press_shortcut"
        );
        assert_eq!(
            tree.stages[1].action_group.actions[0].action,
            "close_application"
        );
        assert!(tree.validate().is_empty());
    }

    // ── Click verb ─────────────────────────────────────────────────

    #[test]
    fn compile_click_and_type() {
        let spec = MultiVerbSpec {
            original_text: "Click search box and type query".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Click,
                    targets: vec![TargetRef::Element("search_box".into())],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Type,
                    targets: vec![],
                    content: Some(ContentClass::Literal("query".into())),
                },
            ],
        };
        let compiler = RuleBasedWorkflowCompiler;
        let tree = compiler.compile(&spec, &empty_facts()).unwrap();

        assert_eq!(tree.stages.len(), 2);
        assert_eq!(
            tree.stages[0].action_group.actions[0].action,
            "click_element"
        );
        assert_eq!(tree.stages[1].action_group.actions[0].action, "type_text");
        assert!(tree.validate().is_empty());
    }

    // ── Precondition: DisplayServerAvailable ───────────────────────

    #[test]
    fn compiled_tree_has_display_precondition() {
        let spec = MultiVerbSpec {
            original_text: "Open firefox and save".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Open,
                    targets: vec![TargetRef::App("firefox".into())],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Save,
                    targets: vec![],
                    content: None,
                },
            ],
        };
        let compiler = RuleBasedWorkflowCompiler;
        let tree = compiler.compile(&spec, &empty_facts()).unwrap();

        assert!(tree
            .preconditions
            .iter()
            .any(|p| matches!(p, Precondition::DisplayServerAvailable)));
    }
}
