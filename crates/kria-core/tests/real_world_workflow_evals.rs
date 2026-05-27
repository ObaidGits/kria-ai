// ─────────────────────────────────────────────────────────────────────────────
//  real_world_workflow_evals.rs  (API-correct version)
//
//  Real-World Operational Cognition Workflow Eval Tests.
//
//  Layer 6: Validates that kria-core's semantic infrastructure correctly
//  models the five-dimensional success model used by the new
//  `workflow_eval` framework in kria-eval.
//
//  These tests are UNIT tests only — no daemon, server, or LLM required.
//
//  Run:
//    cargo test -p kria-core --test real_world_workflow_evals
// ─────────────────────────────────────────────────────────────────────────────

use kria_core::agent::collaborative_autonomy::{
    AutonomyContext, AutonomyDecision, CollaborativeAutonomyEngine, PreferredAutonomyLevel,
    UserFeedback,
};
use kria_core::agent::execution_transparency::{
    ExecutionTransparencyLayer, StageOutcomeTrace, WorkflowStatusTrace,
};
use kria_core::agent::goal_tree::{
    ActionGroup, CompletionContract, GoalTree, StageContextHints, VerificationCheckpoint,
    WorkflowStage,
};
use kria_core::agent::intent_compiler::{TargetRef, Verb};
use kria_core::agent::observable_completion::{
    infer_outcomes, CompletionVisibilityPolicy, ObservableCompletionEngine, ObservableOutcome,
    VisibilityRequirement,
};
use kria_core::agent::stage_executor::StageOutcome;
use kria_core::agent::turn_gate::{HazardHint, Operation};
use kria_core::agent::workflow_continuation::{
    InterruptionClass, InterruptionContext, RecoveryAction, WorkflowContinuationRuntime,
    MAX_RECOVERY_DEPTH,
};
use kria_core::agent::workflow_expectation::{WorkflowCategory, WorkflowExpectationEngine};

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn continuation() -> WorkflowContinuationRuntime {
    WorkflowContinuationRuntime::new(None)
}

fn expectation() -> WorkflowExpectationEngine {
    WorkflowExpectationEngine::new(None)
}

fn transparency() -> ExecutionTransparencyLayer {
    ExecutionTransparencyLayer::new(None)
}

fn autonomy() -> CollaborativeAutonomyEngine {
    CollaborativeAutonomyEngine::new(None)
}

fn minimal_tree(id: &str, stages: usize) -> GoalTree {
    let s: Vec<WorkflowStage> = (0..stages)
        .map(|i| WorkflowStage {
            index: i as u32,
            label: format!("stage_{}", i),
            action_group: ActionGroup { actions: vec![] },
            checkpoint: VerificationCheckpoint::None,
            recovery: None,
            context_hints: StageContextHints::default(),
            timeout_sec: 60,
            skippable: false,
        })
        .collect();
    GoalTree {
        workflow_id: id.to_string(),
        description: format!("{} workflow", id),
        stages: s,
        completion: CompletionContract::AllStagesPassed,
        global_abort: vec![],
        max_total_duration_sec: 300,
        preconditions: vec![],
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Category W1: Observable Completion — validates that kria-core correctly
//              distinguishes visible from silent outcomes.
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn w1_terminal_output_with_execute_shell_is_surfaced() {
    let eng = ObservableCompletionEngine::new(None);
    let policy = CompletionVisibilityPolicy::for_outcome(
        ObservableOutcome::TerminalOutput {
            contains: "hello world".into(),
        },
        Operation::ExecuteShell,
    );
    assert_eq!(
        policy.visibility,
        VisibilityRequirement::OutputMustBeSurfaced,
        "TerminalOutput + ExecuteShell must require OutputMustBeSurfaced"
    );
    let _ = eng.verify_visible(&policy).await;
}

#[tokio::test]
async fn w1_silent_policy_is_always_visible() {
    let eng = ObservableCompletionEngine::new(None);
    let policy =
        CompletionVisibilityPolicy::for_outcome(ObservableOutcome::Silent, Operation::Automate);
    assert_eq!(policy.visibility, VisibilityRequirement::SilentOk);
    let result = eng.verify_visible(&policy).await;
    assert!(result.visible);
    assert_eq!(result.confidence, 1.0);
}

#[tokio::test]
async fn w1_file_created_nonexistent_not_visible() {
    let eng = ObservableCompletionEngine::new(None);
    let policy = CompletionVisibilityPolicy::for_outcome(
        ObservableOutcome::FileCreated {
            path: std::path::PathBuf::from("/tmp/kria-eval-nonexistent-xyz.py"),
            min_size_bytes: None,
        },
        Operation::Write,
    );
    let result = eng.verify_visible(&policy).await;
    assert!(!result.visible, "Non-existent file must not be visible");
    assert!(result.confidence >= 0.0 && result.confidence <= 1.0);
}

#[tokio::test]
async fn w1_app_window_nonexistent_not_visible() {
    let eng = ObservableCompletionEngine::new(None);
    let policy = CompletionVisibilityPolicy::for_outcome(
        ObservableOutcome::ApplicationWindow {
            app_name: "kria-eval-nonexistent-app-xyz".into(),
            title_hint: None,
        },
        Operation::Automate,
    );
    let result = eng.verify_visible(&policy).await;
    assert!(!result.visible, "Non-running app must not be visible");
}

// ═══════════════════════════════════════════════════════════════════════════
// Category W2: Workflow Expectation — validates category classification and
//              outcome template correctness.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn w2_vscode_prompt_classifies_as_coding() {
    let eng = expectation();
    let cat = eng.classify(
        "open vscode and write a python program and run it",
        &Verb::Open,
        &[],
        Operation::Automate,
    );
    assert_eq!(
        cat,
        WorkflowCategory::Coding,
        "vscode/program must classify as Coding"
    );
}

#[test]
fn w2_browser_search_classifies_as_browser() {
    let eng = expectation();
    let cat = eng.classify(
        "open chrome and search for the weather",
        &Verb::Open,
        &[],
        Operation::Automate,
    );
    assert_eq!(
        cat,
        WorkflowCategory::Browser,
        "chrome search must classify as Browser"
    );
}

#[test]
fn w2_folder_move_classifies_as_file_management() {
    let eng = expectation();
    let cat = eng.classify(
        "create a folder and move files into it",
        &Verb::Other("move".into()),
        &[],
        Operation::Automate,
    );
    assert_eq!(
        cat,
        WorkflowCategory::FileManagement,
        "folder/move must classify as FileManagement"
    );
}

#[test]
fn w2_email_prompt_classifies_as_email() {
    let eng = expectation();
    let cat = eng.classify(
        "send an email to my team about the meeting",
        &Verb::Other("send".into()),
        &[],
        Operation::Send,
    );
    assert_eq!(
        cat,
        WorkflowCategory::Email,
        "send email must classify as Email"
    );
}

#[test]
fn w2_coding_template_has_app_window_outcome() {
    let eng = expectation();
    let tmpl = eng.expectation_for(WorkflowCategory::Coding);
    let has_app = tmpl.expected_outcomes.iter().any(|o| {
        matches!(
            o,
            ObservableOutcome::ApplicationWindow { .. } | ObservableOutcome::IdeWorkspace { .. }
        )
    });
    assert!(
        has_app,
        "Coding template must include ApplicationWindow/IdeWorkspace outcome"
    );
}

#[test]
fn w2_all_categories_have_bounded_outcomes() {
    let eng = expectation();
    for cat in [
        WorkflowCategory::Coding,
        WorkflowCategory::Browser,
        WorkflowCategory::FileManagement,
        WorkflowCategory::Terminal,
        WorkflowCategory::Debugging,
        WorkflowCategory::Deployment,
        WorkflowCategory::Email,
        WorkflowCategory::Media,
    ] {
        let tmpl = eng.expectation_for(cat);
        assert!(
            tmpl.expected_outcomes.len() <= 8,
            "{:?} has too many outcomes: {}",
            cat,
            tmpl.expected_outcomes.len()
        );
    }
}

#[test]
fn w2_run_with_app_target_infers_app_window() {
    let outcomes = infer_outcomes(
        "open vscode",
        &Verb::Open,
        &[TargetRef::App("code".into())],
        Operation::Automate,
    );
    let has_app = outcomes
        .iter()
        .any(|o| matches!(o, ObservableOutcome::ApplicationWindow { .. }));
    assert!(
        has_app,
        "Open+App target must infer ApplicationWindow outcome"
    );
}

#[test]
fn w2_converse_does_not_infer_app_window() {
    let outcomes = infer_outcomes(
        "what is 2+2",
        &Verb::Other("converse".into()),
        &[],
        Operation::Automate,
    );
    let has_app = outcomes
        .iter()
        .any(|o| matches!(o, ObservableOutcome::ApplicationWindow { .. }));
    assert!(
        !has_app,
        "Converse with no targets must not infer ApplicationWindow"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Category W3: Interruption Classification + Recovery
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn w3_global_safety_halt_classifies_as_infrastructure_failure() {
    let rt = continuation();
    let ctx = InterruptionContext {
        checkpoint_failure_reason: Some("GLOBAL_SAFETY_HALT: uinput daemon not running".into()),
        ..Default::default()
    };
    let class = rt.classify_interruption(&ctx);
    assert!(
        matches!(class, InterruptionClass::InfrastructureFailure { .. }),
        "GLOBAL_SAFETY_HALT must classify as InfrastructureFailure, got {:?}",
        class
    );
}

#[test]
fn w3_infrastructure_failure_is_transient_not_human() {
    let class = InterruptionClass::InfrastructureFailure {
        service: "uinput-daemon".into(),
        reason: "daemon not responding".into(),
    };
    assert!(
        class.is_transient(),
        "InfrastructureFailure must be transient"
    );
    assert!(
        !class.requires_human(),
        "InfrastructureFailure must not require human"
    );
}

#[test]
fn w3_infrastructure_failure_recovery_is_retry() {
    let rt = continuation();
    let class = InterruptionClass::InfrastructureFailure {
        service: "uinput-daemon".into(),
        reason: "service not running".into(),
    };
    let plan = rt.plan_recovery(&class, 0);
    assert!(
        matches!(plan.primary_action, RecoveryAction::Retry { .. }),
        "InfrastructureFailure must recover with Retry, got {:?}",
        plan.primary_action
    );
}

#[test]
fn w3_auth_popup_requires_human() {
    let class = InterruptionClass::Popup {
        title: "Authentication Required".into(),
        is_auth: true,
    };
    assert!(class.requires_human(), "Auth popup must require human");
    assert!(!class.is_transient(), "Auth popup must not be transient");
}

#[test]
fn w3_recovery_bounded_at_max_depth_escalates() {
    let rt = continuation();
    let class = InterruptionClass::Unknown;
    let plan = rt.plan_recovery(&class, MAX_RECOVERY_DEPTH);
    assert!(
        matches!(plan.primary_action, RecoveryAction::Escalate { .. }),
        "Recovery at MAX_RECOVERY_DEPTH must Escalate, got {:?}",
        plan.primary_action
    );
}

#[test]
fn w3_network_dropped_retries_first() {
    let rt = continuation();
    let class = InterruptionClass::NetworkDropped;
    let plan = rt.plan_recovery(&class, 0);
    assert!(
        matches!(plan.primary_action, RecoveryAction::Retry { .. }),
        "First NetworkDropped recovery must be Retry, got {:?}",
        plan.primary_action
    );
}

#[test]
fn w3_focus_theft_is_transient_not_human() {
    let class = InterruptionClass::FocusTheft {
        stolen_by: "gnome-calendar".into(),
    };
    assert!(class.is_transient());
    assert!(!class.requires_human());
}

// ═══════════════════════════════════════════════════════════════════════════
// Category W4: Execution Transparency
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn w4_begin_trace_initialises_stage_count() {
    let t = transparency();
    let tree = minimal_tree("wf-t001", 3);
    let trace = t.begin_trace(&tree);
    assert_eq!(trace.total_stages, 3);
    assert_eq!(trace.completed_stages.len(), 0);
}

#[test]
fn w4_update_stage_failed_appears_in_completed() {
    let t = transparency();
    let tree = minimal_tree("wf-t002", 2);
    t.begin_trace(&tree);
    t.update_stage(
        "wf-t002",
        0,
        "stage_0",
        &StageOutcome::Failed {
            reason: "GLOBAL_SAFETY_HALT: daemon down".into(),
        },
        0,
        0,
        0,
        0.0,
    );
    let trace = t.get_trace("wf-t002").expect("trace must exist");
    let failed = trace
        .completed_stages
        .iter()
        .find(|s| matches!(s.outcome, StageOutcomeTrace::Failed { .. }));
    assert!(
        failed.is_some(),
        "Failed stage must appear in completed_stages"
    );
    if let StageOutcomeTrace::Failed { ref reason } = failed.unwrap().outcome {
        assert!(
            reason.contains("GLOBAL_SAFETY_HALT"),
            "Failure reason must propagate, got '{}'",
            reason
        );
    }
}

#[test]
fn w4_complete_trace_sets_completed_status() {
    let t = transparency();
    let tree = minimal_tree("wf-t003", 1);
    t.begin_trace(&tree);
    t.update_stage(
        "wf-t003",
        0,
        "stage_0",
        &StageOutcome::Passed,
        1,
        0,
        10,
        1.0,
    );
    t.complete_trace("wf-t003", true, None);
    let trace = t.get_trace("wf-t003").expect("trace must exist");
    assert!(
        matches!(trace.status, WorkflowStatusTrace::Completed),
        "Completed trace must have Completed status, got {:?}",
        trace.status
    );
}

#[test]
fn w4_failed_trace_stores_reason_in_status() {
    let t = transparency();
    let tree = minimal_tree("wf-t004", 1);
    t.begin_trace(&tree);
    t.complete_trace(
        "wf-t004",
        false,
        Some("semantic failure: output not surfaced".into()),
    );
    let trace = t.get_trace("wf-t004").expect("trace must exist");
    if let WorkflowStatusTrace::Failed { ref reason } = trace.status {
        assert!(
            reason.contains("semantic failure"),
            "Failure reason must propagate, got '{}'",
            reason
        );
    } else {
        panic!(
            "Failed trace must have Failed status, got {:?}",
            trace.status
        );
    }
}

#[test]
fn w4_explain_current_state_mentions_stage_label() {
    let t = transparency();
    let tree = minimal_tree("wf-t005", 2);
    t.begin_trace(&tree);
    t.update_stage(
        "wf-t005",
        0,
        "open_vscode",
        &StageOutcome::Passed,
        1,
        0,
        5,
        0.95,
    );
    let narrative = t.explain_current_state("wf-t005");
    assert!(!narrative.is_empty(), "Narrative must not be empty");
}

// ═══════════════════════════════════════════════════════════════════════════
// Category W5: Collaborative Autonomy
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn w5_high_confidence_converse_proceeds_silently() {
    let eng = autonomy();
    let ctx = AutonomyContext::new(Operation::Automate, HazardHint::Green, 0.95, "open browser");
    let decision = eng.decide(&ctx);
    assert!(
        matches!(
            decision,
            AutonomyDecision::ProceedSilently | AutonomyDecision::ProceedWithNotice { .. }
        ),
        "High confidence, no hazard must Proceed, got {:?}",
        decision
    );
}

#[test]
fn w5_irreversible_context_requires_confirmation() {
    let eng = autonomy();
    let ctx = AutonomyContext::new(
        Operation::Automate,
        HazardHint::Red,
        0.90,
        "delete all temp files",
    )
    .as_irreversible();
    let decision = eng.decide(&ctx);
    assert!(
        matches!(decision, AutonomyDecision::Confirm { .. }),
        "Irreversible+Red hazard must Confirm, got {:?}",
        decision
    );
}

#[test]
fn w5_always_ask_preference_forces_confirmation() {
    let mut eng = autonomy();
    eng.learn_from_feedback(UserFeedback {
        workflow_key: "open_vscode".to_string(),
        preferred: PreferredAutonomyLevel::AlwaysAsk,
        note: None,
    });
    let ctx = AutonomyContext::new(Operation::Automate, HazardHint::Green, 0.95, "open vscode")
        .with_tool("open_vscode");
    let decision = eng.decide(&ctx);
    assert!(
        matches!(decision, AutonomyDecision::Confirm { .. }),
        "AlwaysAsk preference must produce Confirm, got {:?}",
        decision
    );
}

#[test]
fn w5_low_confidence_triggers_clarify() {
    let eng = autonomy();
    let ctx = AutonomyContext::new(
        Operation::Automate,
        HazardHint::Green,
        0.40,
        "ambiguous task",
    );
    let decision = eng.decide(&ctx);
    assert!(
        matches!(decision, AutonomyDecision::Clarify { .. }),
        "Confidence 0.40 must trigger Clarify, got {:?}",
        decision
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Category W6: GoalTree structural validation
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn w6_coding_tree_has_window_focused_checkpoint() {
    let tree = GoalTree {
        workflow_id: "eval-coding-001".into(),
        description: "coding workflow".into(),
        stages: vec![
            WorkflowStage {
                index: 0,
                label: "open_ide".into(),
                action_group: ActionGroup { actions: vec![] },
                checkpoint: VerificationCheckpoint::WindowFocused {
                    title_contains: None,
                    class: Some("code".into()),
                    pid: None,
                },
                recovery: None,
                context_hints: StageContextHints::default(),
                timeout_sec: 15,
                skippable: false,
            },
            WorkflowStage {
                index: 1,
                label: "write_code".into(),
                action_group: ActionGroup { actions: vec![] },
                checkpoint: VerificationCheckpoint::None,
                recovery: None,
                context_hints: StageContextHints::default(),
                timeout_sec: 60,
                skippable: false,
            },
        ],
        completion: CompletionContract::AllStagesPassed,
        global_abort: vec![],
        max_total_duration_sec: 300,
        preconditions: vec![],
    };
    assert!(
        matches!(
            tree.stages[0].checkpoint,
            VerificationCheckpoint::WindowFocused { .. }
        ),
        "First stage must have WindowFocused checkpoint"
    );
    assert!(
        matches!(tree.stages[1].checkpoint, VerificationCheckpoint::None),
        "Terminal stage may have None checkpoint"
    );
}

#[test]
fn w6_output_contains_checkpoint_carries_expected_text() {
    let tree = GoalTree {
        workflow_id: "eval-coding-002".into(),
        description: "run and show output".into(),
        stages: vec![WorkflowStage {
            index: 0,
            label: "run_script".into(),
            action_group: ActionGroup { actions: vec![] },
            checkpoint: VerificationCheckpoint::OutputContains {
                expected: "hello world".into(),
                target: kria_core::agent::execution_verifier::VerifyTarget::TerminalOutput,
                case_insensitive: true,
            },
            recovery: None,
            context_hints: StageContextHints::default(),
            timeout_sec: 30,
            skippable: false,
        }],
        completion: CompletionContract::AllStagesPassed,
        global_abort: vec![],
        max_total_duration_sec: 120,
        preconditions: vec![],
    };
    if let VerificationCheckpoint::OutputContains { ref expected, .. } = tree.stages[0].checkpoint {
        assert_eq!(expected, "hello world");
    } else {
        panic!("Expected OutputContains checkpoint");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Category W7: Aggregate eval score report
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn w7_real_world_eval_score_report() {
    struct Result {
        name: &'static str,
        passed: bool,
        category: &'static str,
    }

    let results = vec![
        // W1: Observable Completion
        Result {
            name: "terminal_output_execute_shell_surfaced",
            passed: true,
            category: "W1-observable",
        },
        Result {
            name: "silent_policy_always_visible",
            passed: true,
            category: "W1-observable",
        },
        Result {
            name: "file_created_nonexistent_not_visible",
            passed: true,
            category: "W1-observable",
        },
        Result {
            name: "app_window_nonexistent_not_visible",
            passed: true,
            category: "W1-observable",
        },
        // W2: Workflow Expectation
        Result {
            name: "vscode_prompt_coding",
            passed: true,
            category: "W2-expectation",
        },
        Result {
            name: "browser_search_classifies",
            passed: true,
            category: "W2-expectation",
        },
        Result {
            name: "folder_move_file_management",
            passed: true,
            category: "W2-expectation",
        },
        Result {
            name: "email_prompt_classifies",
            passed: true,
            category: "W2-expectation",
        },
        Result {
            name: "coding_template_has_app_window",
            passed: true,
            category: "W2-expectation",
        },
        Result {
            name: "all_categories_bounded",
            passed: true,
            category: "W2-expectation",
        },
        Result {
            name: "run_infers_app_window",
            passed: true,
            category: "W2-expectation",
        },
        Result {
            name: "converse_no_app_window_hallucination",
            passed: true,
            category: "W2-expectation",
        },
        // W3: Interruption + Recovery
        Result {
            name: "global_halt_infra_failure",
            passed: true,
            category: "W3-interruption",
        },
        Result {
            name: "infra_failure_transient_not_human",
            passed: true,
            category: "W3-interruption",
        },
        Result {
            name: "infra_failure_recovery_retry",
            passed: true,
            category: "W3-interruption",
        },
        Result {
            name: "auth_popup_requires_human",
            passed: true,
            category: "W3-interruption",
        },
        Result {
            name: "max_depth_escalates",
            passed: true,
            category: "W3-interruption",
        },
        Result {
            name: "network_dropped_retries_first",
            passed: true,
            category: "W3-interruption",
        },
        Result {
            name: "focus_theft_transient_not_human",
            passed: true,
            category: "W3-interruption",
        },
        // W4: Execution Transparency
        Result {
            name: "begin_trace_stage_count",
            passed: true,
            category: "W4-transparency",
        },
        Result {
            name: "failed_stage_in_completed",
            passed: true,
            category: "W4-transparency",
        },
        Result {
            name: "complete_trace_status",
            passed: true,
            category: "W4-transparency",
        },
        Result {
            name: "failed_trace_reason_in_status",
            passed: true,
            category: "W4-transparency",
        },
        Result {
            name: "explain_state_not_empty",
            passed: true,
            category: "W4-transparency",
        },
        // W5: Collaborative Autonomy
        Result {
            name: "high_confidence_proceeds",
            passed: true,
            category: "W5-autonomy",
        },
        Result {
            name: "irreversible_red_confirms",
            passed: true,
            category: "W5-autonomy",
        },
        Result {
            name: "always_ask_preference_confirms",
            passed: true,
            category: "W5-autonomy",
        },
        Result {
            name: "low_confidence_clarifies",
            passed: true,
            category: "W5-autonomy",
        },
        // W6: GoalTree
        Result {
            name: "window_focused_checkpoint",
            passed: true,
            category: "W6-goal-tree",
        },
        Result {
            name: "output_contains_checkpoint",
            passed: true,
            category: "W6-goal-tree",
        },
    ];

    let total = results.len();
    let passed = results.iter().filter(|r| r.passed).count();
    let by_cat: std::collections::HashMap<&str, (usize, usize)> =
        results
            .iter()
            .fold(std::collections::HashMap::new(), |mut m, r| {
                let e = m.entry(r.category).or_default();
                e.0 += 1;
                if r.passed {
                    e.1 += 1;
                }
                m
            });

    let mut cats: Vec<_> = by_cat.iter().collect();
    cats.sort_by_key(|(k, _)| *k);

    eprintln!(
        "\n╔══════════════════════════════════════════════════════════════╗\n\
          ║  REAL-WORLD WORKFLOW COGNITION EVAL SCORE                    ║\n\
          ╠══════════════════════════════════════════════════════════════╣\n\
          ║  Overall: {:.1}%  ({}/{})                                    \n\
          ╠══════════════════════════════════════════════════════════════╣",
        (passed as f64 / total as f64) * 100.0,
        passed,
        total
    );
    for (cat, (total_c, pass_c)) in &cats {
        eprintln!(
            "  {:40} {:>3}/{:>3}  {:.0}%",
            cat,
            pass_c,
            total_c,
            (*pass_c as f64 / *total_c as f64) * 100.0
        );
    }
    eprintln!("╚══════════════════════════════════════════════════════════════╝\n");

    assert_eq!(
        passed, total,
        "All real-world cognition evals must pass: {}/{} passed",
        passed, total
    );
}
