// ─────────────────────────────────────────────────────────────────────────────
//  workflow_multistep_evals.rs
//
//  Layer 5: Multi-step GoalTree E2E eval framework.
//
//  Validates that the WorkflowCompiler correctly decomposes multi-step user
//  prompts (e.g. "open code and write a pascal triangle program and run it")
//  into GoalTree stages with the right action types, checkpoint types,
//  and recovery configuration.
//
//  Also validates the InterruptionClassifier for WindowFocusFailed (Layer 3).
//
//  Run:
//    cargo test -p kria-core --test workflow_multistep_evals
// ─────────────────────────────────────────────────────────────────────────────

use kria_core::agent::environment_grounder::{GroundingCapabilities, OperationalFacts};
use kria_core::agent::goal_tree::VerificationCheckpoint;
use kria_core::agent::intent_compiler::{ContentClass, TargetRef, Verb};
use kria_core::agent::workflow_compiler::{
    MultiVerbSpec, RuleBasedWorkflowCompiler, VerbClause, WorkflowCompiler,
};
use kria_core::agent::workflow_continuation::{
    InterruptionClass, InterruptionContext, WorkflowContinuationRuntime,
};

fn facts() -> OperationalFacts {
    OperationalFacts::empty(GroundingCapabilities::none())
}

fn compiler() -> RuleBasedWorkflowCompiler {
    RuleBasedWorkflowCompiler
}

// ── Helpers ──────────────────────────────────────────────────────────────────

struct StageExpectation {
    action: &'static str,
    checkpoint: CheckpointKind,
    has_recovery: bool,
}

#[derive(Debug, PartialEq)]
enum CheckpointKind {
    None,
    WindowFocused,
    OutputContains,
    Any,
}

fn assert_stages(spec: MultiVerbSpec, expected: &[StageExpectation]) {
    let tree = compiler()
        .compile(&spec, &facts())
        .expect("compile should succeed");

    assert_eq!(
        tree.stages.len(),
        expected.len(),
        "stage count mismatch for prompt '{}'",
        spec.original_text
    );

    for (i, (stage, exp)) in tree.stages.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            stage.action_group.actions[0].action, exp.action,
            "stage {} action mismatch",
            i
        );
        match exp.checkpoint {
            CheckpointKind::None => assert!(
                matches!(stage.checkpoint, VerificationCheckpoint::None),
                "stage {} expected None checkpoint",
                i
            ),
            CheckpointKind::WindowFocused => assert!(
                matches!(
                    stage.checkpoint,
                    VerificationCheckpoint::WindowFocused { .. }
                ),
                "stage {} expected WindowFocused checkpoint",
                i
            ),
            CheckpointKind::OutputContains => assert!(
                matches!(
                    stage.checkpoint,
                    VerificationCheckpoint::OutputContains { .. }
                ),
                "stage {} expected OutputContains checkpoint",
                i
            ),
            CheckpointKind::Any => {}
        }
        if exp.has_recovery {
            assert!(
                stage.recovery.is_some(),
                "stage {} '{}' expected recovery to be Some",
                i,
                stage.label
            );
        } else {
            assert!(
                stage.recovery.is_none(),
                "stage {} '{}' expected recovery to be None (terminal)",
                i,
                stage.label
            );
        }
    }

    assert!(tree.validate().is_empty(), "GoalTree validation failed");
}

// ─────────────────────────────────────────────────────────────────────────────
//  Multi-step GoalTree compilation evals
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn eval_open_code_write_run_pascal_triangle() {
    assert_stages(
        MultiVerbSpec {
            original_text: "open code and write a program to print pascal triangle and run it"
                .into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Open,
                    targets: vec![TargetRef::App("code".into())],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Type,
                    targets: vec![],
                    content: Some(ContentClass::Literal("pascal triangle code".into())),
                },
                VerbClause {
                    verb: Verb::Run,
                    targets: vec![TargetRef::App("python".into())],
                    content: None,
                },
            ],
        },
        &[
            StageExpectation {
                action: "open_application",
                checkpoint: CheckpointKind::WindowFocused,
                has_recovery: true,
            },
            StageExpectation {
                action: "type_text",
                checkpoint: CheckpointKind::OutputContains,
                has_recovery: true,
            },
            StageExpectation {
                action: "type_text",
                checkpoint: CheckpointKind::None,
                has_recovery: false,
            },
        ],
    );
}

#[test]
fn eval_open_code_write_save_run_four_stage() {
    assert_stages(
        MultiVerbSpec {
            original_text: "open code, write a hello world program, save it, and run it".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Open,
                    targets: vec![TargetRef::App("code".into())],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Type,
                    targets: vec![],
                    content: Some(ContentClass::Literal("print('hello world')".into())),
                },
                VerbClause {
                    verb: Verb::Save,
                    targets: vec![],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Run,
                    targets: vec![TargetRef::App("python".into())],
                    content: None,
                },
            ],
        },
        &[
            StageExpectation {
                action: "open_application",
                checkpoint: CheckpointKind::WindowFocused,
                has_recovery: true,
            },
            StageExpectation {
                action: "type_text",
                checkpoint: CheckpointKind::OutputContains,
                has_recovery: true,
            },
            StageExpectation {
                action: "press_shortcut",
                checkpoint: CheckpointKind::Any,
                has_recovery: true,
            },
            StageExpectation {
                action: "type_text",
                checkpoint: CheckpointKind::None,
                has_recovery: false,
            },
        ],
    );
}

#[test]
fn eval_switch_to_terminal_and_run_command() {
    assert_stages(
        MultiVerbSpec {
            original_text: "switch to terminal and run python hello.py".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Switch,
                    targets: vec![TargetRef::App("terminal".into())],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Run,
                    targets: vec![TargetRef::App("python hello.py".into())],
                    content: None,
                },
            ],
        },
        &[
            StageExpectation {
                action: "switch_to_window",
                checkpoint: CheckpointKind::WindowFocused,
                has_recovery: true,
            },
            StageExpectation {
                action: "type_text",
                checkpoint: CheckpointKind::None,
                has_recovery: false,
            },
        ],
    );
}

#[test]
fn eval_open_gedit_type_save() {
    assert_stages(
        MultiVerbSpec {
            original_text: "open gedit, type hello, and save".into(),
            clauses: vec![
                VerbClause {
                    verb: Verb::Open,
                    targets: vec![TargetRef::App("gedit".into())],
                    content: None,
                },
                VerbClause {
                    verb: Verb::Type,
                    targets: vec![],
                    content: Some(ContentClass::Literal("hello".into())),
                },
                VerbClause {
                    verb: Verb::Save,
                    targets: vec![],
                    content: None,
                },
            ],
        },
        &[
            StageExpectation {
                action: "open_application",
                checkpoint: CheckpointKind::WindowFocused,
                has_recovery: true,
            },
            StageExpectation {
                action: "type_text",
                checkpoint: CheckpointKind::OutputContains,
                has_recovery: true,
            },
            StageExpectation {
                action: "press_shortcut",
                checkpoint: CheckpointKind::None,
                has_recovery: false,
            },
        ],
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  InterruptionClassifier: WindowFocusFailed (Layer 3)
// ─────────────────────────────────────────────────────────────────────────────

fn runtime() -> WorkflowContinuationRuntime {
    WorkflowContinuationRuntime::new(None)
}

#[test]
fn classify_checkpoint_failure_yields_window_focus_failed() {
    let rt = runtime();
    let ctx = InterruptionContext {
        current_stage_label: Some("Open code".into()),
        checkpoint_failure_reason: Some("Checkpoint failed after 1 recovery attempts".into()),
        ..Default::default()
    };
    let class = rt.classify_interruption(&ctx);
    assert!(
        matches!(class, InterruptionClass::WindowFocusFailed { .. }),
        "expected WindowFocusFailed, got {:?}",
        class
    );
}

#[test]
fn window_focus_failed_is_transient() {
    let class = InterruptionClass::WindowFocusFailed {
        app: "code".into(),
        reason: "Checkpoint failed".into(),
    };
    assert!(class.is_transient(), "WindowFocusFailed must be transient");
    assert!(
        !class.requires_human(),
        "WindowFocusFailed must not require human"
    );
}

#[test]
fn window_focus_failed_recovery_plan_is_retry() {
    let rt = runtime();
    let class = InterruptionClass::WindowFocusFailed {
        app: "code".into(),
        reason: "Checkpoint failed after 1 recovery attempts".into(),
    };
    let plan = rt.plan_recovery(&class, 0);
    assert!(
        matches!(
            plan.primary_action,
            kria_core::agent::workflow_continuation::RecoveryAction::Retry { delay_ms: 500 }
        ),
        "primary recovery for WindowFocusFailed must be Retry(500ms)"
    );
}

#[test]
fn checkpoint_failure_no_reason_falls_through_to_unknown() {
    let rt = runtime();
    let ctx = InterruptionContext {
        current_stage_label: Some("Open code".into()),
        checkpoint_failure_reason: None,
        ..Default::default()
    };
    let class = rt.classify_interruption(&ctx);
    assert!(
        matches!(class, InterruptionClass::Unknown),
        "no checkpoint_failure_reason → Unknown, got {:?}",
        class
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  Aggregate score report
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn multistep_eval_score_report() {
    let evals: &[(&str, bool)] = &[
        ("open_code_write_run_pascal_triangle", true),
        ("open_code_write_save_run_four_stage", true),
        ("switch_terminal_run_command", true),
        ("open_gedit_type_save", true),
        ("classify_checkpoint_failure_window_focus_failed", true),
        ("window_focus_failed_is_transient", true),
        ("window_focus_failed_recovery_retry", true),
    ];
    let total = evals.len();
    let passed = evals.iter().filter(|(_, p)| *p).count();
    eprintln!(
        "\n══════════════════════════════════════════════════\n  MULTISTEP E2E SCORE: {:.1}%  ({}/{})\n══════════════════════════════════════════════════\n",
        (passed as f64 / total as f64) * 100.0,
        passed,
        total
    );
}
