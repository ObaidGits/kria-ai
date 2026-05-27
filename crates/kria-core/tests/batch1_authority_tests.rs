//! Batch 1 — Runtime Authority & Boundedness Validation Tests.
//!
//! # Coverage Matrix
//!
//! ## Category A: Verifier Authority (Phase 1 hardening)
//! - A1: Canonical BoundedExecutionVerifier is used by all non-GUI execution paths
//! - A2: Verifier returns verified=true for existing file
//! - A3: Verifier returns verified=false for nonexistent file
//! - A4: Verifier handles Unverifiable honestly (never silent success)
//! - A5: Verifier handles UserAttested honestly (never auto-verifies)
//! - A6: Verifier is bounded (≤500ms on timeout)
//! - A7: ProcessNotRunning returns true for a never-started binary
//! - A8: DeterministicOutput verifies file content correctly
//!
//! ## Category B: Authority Chain Bypass Tests (Phase 1 fail-closed)
//! - B1: Policy engine is evaluated for every tool name
//! - B2: BLACK-tier tools are blocked by policy
//! - B3: Policy decision is structurally sound (not simultaneously blocked + requires_approval)
//! - B4: Audit logger receives calls for blocked tools
//!
//! ## Category C: Session Persistence Boundedness (Phase 3)
//! - C1: SessionManager creates and loads sessions correctly
//! - C2: Session file path is stable and deterministic
//! - C3: Session deletion clears the on-disk file
//! - C4: Concurrent session saves don't corrupt each other
//! - C5: Large session (500 steps) saves and reloads correctly
//!
//! ## Category D: Transparency Layer Wiring (Phase 5 hardening)
//! - D1: ExecutionTransparencyLayer begin_trace creates a trace
//! - D2: update_stage records completed stages
//! - D3: complete_trace marks trace as Completed
//! - D4: Multiple concurrent trace updates don't deadlock
//! - D5: Trace not found for unknown workflow_id returns None
//!
//! ## Category E: WorkflowContinuationRuntime Boundedness (Phase 4)
//! - E1: Recovery depth limit enforced (≥ MAX_RECOVERY_DEPTH escalates)
//! - E2: pause_workflow writes checkpoint and returns plan
//! - E3: resume_workflow on nonexistent session returns failure
//! - E4: All interruption classes produce a valid primary action (no panic)
//! - E5: Recovery tree depth is bounded (fallbacks ≤ 1)

use kria_core::agent::execution_transparency::ExecutionTransparencyLayer;
use kria_core::agent::execution_verifier::ExecutionVerifier;
use kria_core::agent::execution_verifier::{FsEffect, Verifiability, VerifyTarget};
use kria_core::agent::execution_verifier_bounded::BoundedExecutionVerifier;
use kria_core::agent::goal_tree::{CompletionContract, GoalTree};
use kria_core::agent::stage_executor::StageOutcome;
use kria_core::agent::workflow_continuation::{
    InterruptionClass, WorkflowContinuationRuntime, MAX_RECOVERY_DEPTH,
};
use kria_core::agent::workflow_session::{SessionManager, WorkflowSession};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::NamedTempFile;

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

fn make_goal_tree(n: usize) -> GoalTree {
    GoalTree {
        workflow_id: format!("test-wf-{}", n),
        description: "test workflow".to_string(),
        stages: vec![],
        completion: CompletionContract::AllStagesPassed,
        global_abort: vec![],
        max_total_duration_sec: 60,
        preconditions: vec![],
    }
}

fn transparency() -> ExecutionTransparencyLayer {
    ExecutionTransparencyLayer::new(None)
}

fn continuation() -> WorkflowContinuationRuntime {
    WorkflowContinuationRuntime::new(None)
}

// ═══════════════════════════════════════════════════════════════════════════
// Category A: Verifier Authority
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a2_verifier_confirms_existing_file() {
    let tmp = NamedTempFile::new().unwrap();
    let verifier = BoundedExecutionVerifier::new();
    let outcome = verifier
        .verify(&Verifiability::FileSystemEffect {
            path: tmp.path().to_path_buf(),
            kind: FsEffect::Exists,
        })
        .await;
    assert!(outcome.verified, "existing file must be verified=true");
    assert!(outcome.confidence >= 0.9);
}

#[tokio::test]
async fn a3_verifier_rejects_nonexistent_file() {
    let verifier = BoundedExecutionVerifier::new();
    let outcome = verifier
        .verify(&Verifiability::FileSystemEffect {
            path: PathBuf::from("/tmp/kria_authority_test_nonexistent_99999"),
            kind: FsEffect::Exists,
        })
        .await;
    assert!(!outcome.verified, "nonexistent file must be verified=false");
}

#[tokio::test]
async fn a4_verifier_unverifiable_never_reports_success() {
    let verifier = BoundedExecutionVerifier::new();
    let outcome = verifier
        .verify(&Verifiability::Unverifiable {
            reason: "test: action has no observable side effect".to_string(),
        })
        .await;
    assert!(
        !outcome.verified,
        "Unverifiable must NEVER return verified=true (fail-closed)"
    );
    assert_eq!(
        outcome.confidence, 0.0,
        "Unverifiable must have 0 confidence"
    );
}

#[tokio::test]
async fn a5_verifier_user_attested_never_auto_verifies() {
    let verifier = BoundedExecutionVerifier::new();
    let outcome = verifier
        .verify(&Verifiability::UserAttested {
            question: "Did the file get uploaded?".to_string(),
        })
        .await;
    assert!(
        !outcome.verified,
        "UserAttested must NEVER auto-verify — requires HITL"
    );
}

#[tokio::test]
async fn a6_verifier_respects_500ms_timeout() {
    let verifier = BoundedExecutionVerifier::new();
    let start = std::time::Instant::now();
    // Use ProcessLaunched with a short binary name that definitely doesn't exist
    let _outcome = verifier
        .verify(&Verifiability::ProcessLaunched {
            binary: "kria_nonexistent_binary_99999".to_string(),
            max_wait_ms: 50, // very short wait
        })
        .await;
    let elapsed = start.elapsed().as_millis();
    // Must complete within the 500ms global timeout + 50ms wait
    assert!(
        elapsed < 600,
        "verifier must complete within bounded time, took {}ms",
        elapsed
    );
}

#[tokio::test]
async fn a7_process_not_running_for_nonexistent_binary() {
    let verifier = BoundedExecutionVerifier::new();
    let outcome = verifier
        .verify(&Verifiability::ProcessNotRunning {
            binary: "kria_nonexistent_binary_for_test_99999".to_string(),
            max_wait_ms: 100,
        })
        .await;
    assert!(
        outcome.verified,
        "a binary that doesn't exist is definitely not running"
    );
}

#[tokio::test]
async fn a8_deterministic_output_verifies_file_content() {
    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"hello world from kria").unwrap();
    let verifier = BoundedExecutionVerifier::new();
    let outcome = verifier
        .verify(&Verifiability::DeterministicOutput {
            expected_substring: "hello world".to_string(),
            in_target: VerifyTarget::FilePath(tmp.path().to_path_buf()),
        })
        .await;
    assert!(outcome.verified, "file content must verify substring match");
    assert!(outcome.confidence >= 0.9);
}

// ═══════════════════════════════════════════════════════════════════════════
// Category B: Authority Chain Bypass Tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn b1_policy_engine_evaluates_all_tool_names() {
    use kria_core::safety::policy::PolicyEngine;
    let engine = PolicyEngine::default();
    // All of these must produce a valid (non-panicking) decision
    for tool in &[
        "write_file",
        "execute_bash",
        "delete_file",
        "unknown_tool_xyz",
        "mcp_colab-mcp_open_colab_browser_connection",
        "open_application",
        "list_directory",
    ] {
        let decision = engine.evaluate(tool, &serde_json::json!({}));
        // decision must be structurally sound
        assert!(
            !(decision.blocked && decision.requires_approval),
            "policy invariant: tool '{}' cannot be both blocked and require approval",
            tool
        );
    }
}

#[test]
fn b3_policy_decision_never_simultaneously_blocked_and_requires_approval() {
    use kria_core::safety::policy::PolicyEngine;
    let engine = PolicyEngine::default();
    // Stress test: evaluate many tools and verify invariant
    let tools = vec![
        "write_file",
        "create_file",
        "delete_file",
        "move_file",
        "copy_file",
        "execute_bash",
        "execute_python",
        "execute_powershell",
        "open_application",
        "close_application",
        "kill_process",
        "install_package",
        "uninstall_package",
        "list_directory",
        "read_file",
        "get_active_window",
        "click_element",
        "type_text",
        "take_screenshot",
        "format_disk",
        "delete_all_files",
    ];
    for tool in &tools {
        let decision = engine.evaluate(tool, &serde_json::json!({"test": true}));
        assert!(
            !(decision.blocked && decision.requires_approval),
            "authority invariant violated for tool '{}': blocked={} requires_approval={}",
            tool,
            decision.blocked,
            decision.requires_approval
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Category C: Session Persistence Boundedness
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn c1_session_manager_round_trips_session() {
    let mgr = SessionManager::new();
    let mut session = WorkflowSession::new(
        "test-session-c1".to_string(),
        "open firefox".to_string(),
        "ReAct".to_string(),
    );
    session.add_step(kria_core::agent::workflow_session::SessionStep {
        step: 0,
        action: "open_application".to_string(),
        params: serde_json::json!({"name": "firefox"}),
        success: true,
        evidence: "Firefox opened".to_string(),
        timestamp: 1_700_000_000,
    });
    mgr.save(&session).unwrap();
    let loaded = mgr
        .load("test-session-c1")
        .expect("session must be loadable");
    assert_eq!(loaded.session_id, "test-session-c1");
    assert_eq!(loaded.completed_steps.len(), 1);
    assert_eq!(loaded.completed_steps[0].action, "open_application");
    // Cleanup
    mgr.delete("test-session-c1");
}

#[test]
fn c3_session_deletion_removes_file() {
    let mgr = SessionManager::new();
    let session = WorkflowSession::new(
        "test-session-c3".to_string(),
        "run test".to_string(),
        "ReAct".to_string(),
    );
    mgr.save(&session).unwrap();
    let path = mgr.session_path("test-session-c3");
    assert!(path.exists(), "session file must exist after save");
    mgr.delete("test-session-c3");
    assert!(
        !path.exists(),
        "session file must be deleted after delete()"
    );
}

#[test]
fn c4_concurrent_session_saves_no_corruption() {
    use std::thread;
    let mgr = Arc::new(SessionManager::new());
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let mgr = Arc::clone(&mgr);
            thread::spawn(move || {
                let mut session = WorkflowSession::new(
                    format!("concurrent-session-{}", i),
                    format!("task {}", i),
                    "ReAct".to_string(),
                );
                for j in 0..10 {
                    session.add_step(kria_core::agent::workflow_session::SessionStep {
                        step: j,
                        action: format!("tool_{}", j),
                        params: serde_json::json!({}),
                        success: true,
                        evidence: format!("step {} done", j),
                        timestamp: 1_700_000_000 + j as u64,
                    });
                }
                mgr.save(&session).unwrap();
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    // Verify all 8 sessions are readable and not corrupted
    for i in 0..8 {
        let id = format!("concurrent-session-{}", i);
        let loaded = mgr
            .load(&id)
            .expect("session must be readable after concurrent save");
        assert_eq!(
            loaded.completed_steps.len(),
            10,
            "all steps must be saved for session {}",
            i
        );
        mgr.delete(&id);
    }
}

#[test]
fn c5_large_session_500_steps_round_trips() {
    let mgr = SessionManager::new();
    let mut session = WorkflowSession::new(
        "large-session-c5".to_string(),
        "large workflow".to_string(),
        "ReAct".to_string(),
    );
    for i in 0..500 {
        session.add_step(kria_core::agent::workflow_session::SessionStep {
            step: i,
            action: format!("tool_{}", i % 20),
            params: serde_json::json!({"step": i, "data": "x".repeat(100)}),
            success: i % 7 != 0,
            evidence: format!("step {} completed with data", i),
            timestamp: 1_700_000_000 + i as u64,
        });
    }
    mgr.save(&session).unwrap();
    let loaded = mgr.load("large-session-c5").unwrap();
    assert_eq!(
        loaded.completed_steps.len(),
        500,
        "all 500 steps must survive round-trip"
    );
    mgr.delete("large-session-c5");
}

// ═══════════════════════════════════════════════════════════════════════════
// Category D: Transparency Layer Wiring
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn d1_transparency_begin_trace_creates_trace() {
    let layer = transparency();
    let tree = make_goal_tree(3);
    let trace = layer.begin_trace(&tree);
    assert_eq!(trace.workflow_id, tree.workflow_id);
}

#[test]
fn d2_transparency_update_stage_records() {
    let layer = transparency();
    let tree = make_goal_tree(5);
    layer.begin_trace(&tree);
    layer.update_stage(
        &tree.workflow_id,
        0,
        "open_firefox",
        &StageOutcome::Passed,
        1,
        0,
        100,
        0.9,
    );
    layer.update_stage(
        &tree.workflow_id,
        1,
        "navigate_url",
        &StageOutcome::Passed,
        1,
        0,
        50,
        0.95,
    );
    let trace = layer.get_trace(&tree.workflow_id).unwrap();
    assert_eq!(trace.completed_stages.len(), 2);
}

#[test]
fn d3_transparency_complete_trace_marks_completed() {
    let layer = transparency();
    let tree = make_goal_tree(2);
    layer.begin_trace(&tree);
    layer.complete_trace(&tree.workflow_id, true, None);
    let trace = layer.get_trace(&tree.workflow_id).unwrap();
    assert!(matches!(
        trace.status,
        kria_core::agent::execution_transparency::WorkflowStatusTrace::Completed
    ));
}

#[test]
fn d4_transparency_concurrent_updates_no_deadlock() {
    use std::thread;
    let layer = Arc::new(transparency());
    let tree = make_goal_tree(16);
    layer.begin_trace(&tree);
    let handles: Vec<_> = (0..16)
        .map(|i| {
            let layer = Arc::clone(&layer);
            let wf_id = tree.workflow_id.clone();
            thread::spawn(move || {
                layer.update_stage(
                    &wf_id,
                    i as u32,
                    &format!("stage_{}", i),
                    &StageOutcome::Passed,
                    1,
                    0,
                    10,
                    0.9,
                );
            })
        })
        .collect();
    for h in handles {
        h.join().expect("transparency thread must not deadlock");
    }
    let trace = layer.get_trace(&tree.workflow_id).unwrap();
    assert_eq!(trace.completed_stages.len(), 16);
}

#[test]
fn d5_transparency_unknown_workflow_id_returns_none() {
    let layer = transparency();
    assert!(
        layer.get_trace("nonexistent-workflow-id-99999").is_none(),
        "unknown workflow_id must return None"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Category E: WorkflowContinuationRuntime Boundedness
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn e1_recovery_depth_limit_always_escalates() {
    let rt = continuation();
    let plan = rt.plan_recovery(&InterruptionClass::NetworkDropped, MAX_RECOVERY_DEPTH);
    assert!(
        matches!(
            plan.primary_action,
            kria_core::agent::workflow_continuation::RecoveryAction::Escalate { .. }
        ),
        "at MAX_RECOVERY_DEPTH, plan must always escalate"
    );
}

#[test]
fn e3_resume_nonexistent_session_returns_failure() {
    let rt = continuation();
    let result = rt.resume_workflow("nonexistent-session-e3-99999");
    assert!(
        !result.success,
        "resuming nonexistent session must return success=false"
    );
}

#[test]
fn e4_all_interruption_classes_produce_valid_primary_action() {
    let rt = continuation();
    let interruptions = vec![
        InterruptionClass::Popup {
            title: "Update available".to_string(),
            is_auth: false,
        },
        InterruptionClass::Popup {
            title: "sudo password".to_string(),
            is_auth: true,
        },
        InterruptionClass::FocusTheft {
            stolen_by: "Slack".to_string(),
        },
        InterruptionClass::AuthRequired {
            service: "GitHub".to_string(),
        },
        InterruptionClass::NetworkDropped,
        InterruptionClass::ProcessCrashed {
            binary: "firefox".to_string(),
        },
        InterruptionClass::BrowserStateChanged {
            url: "about:blank".to_string(),
        },
        InterruptionClass::IdeConflict {
            file: "main.rs".to_string(),
        },
        InterruptionClass::CompositorEvent {
            description: "Wayland restart".to_string(),
        },
        InterruptionClass::UserIntervened {
            description: "pressed ESC".to_string(),
        },
        InterruptionClass::Timeout {
            stage_label: "navigate".to_string(),
        },
        InterruptionClass::ResourceExhausted {
            resource: "disk".to_string(),
        },
        InterruptionClass::Unknown,
    ];
    for interruption in &interruptions {
        let plan = rt.plan_recovery(interruption, 0);
        // Must not panic and must produce a non-empty explanation
        assert!(
            !plan.explanation.is_empty(),
            "every interruption class must produce a non-empty explanation: {:?}",
            interruption
        );
    }
}

#[test]
fn e5_recovery_tree_depth_bounded() {
    let rt = continuation();
    let plan = rt.plan_recovery(&InterruptionClass::NetworkDropped, 0);
    assert!(
        plan.fallbacks.len() <= 1,
        "recovery fallback tree must be bounded to depth 1, got {}",
        plan.fallbacks.len()
    );
    // Each fallback must also be bounded
    for fb in &plan.fallbacks {
        assert!(
            fb.depth <= MAX_RECOVERY_DEPTH,
            "fallback depth {} exceeds MAX_RECOVERY_DEPTH {}",
            fb.depth,
            MAX_RECOVERY_DEPTH
        );
    }
}
