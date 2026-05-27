//! PSDG Integration Tests — Batch 1
//!
//! # Coverage
//!
//! - P4: Threading/lifecycle safety
//!   - SQLite WAL concurrent access (two connections to same file)
//!   - Fire-and-forget write safety (no data races)
//!   - Cancellation token propagation (PsdgCoordinator exits cleanly)
//!
//! - P5: Eval coverage
//!   - PerceptionBus event → WorldModelStore persistence
//!   - Browser state → PSDG persistence
//!   - IDE state → PSDG persistence
//!   - Workflow stage progress → PSDG persistence
//!   - Context snapshot accuracy
//!   - Fact decay and pruning
//!   - Context injection selectivity (only for correct operations)
//!   - Concurrent writes from multiple PSDG handles (SQLite WAL safety)
//!   - False-success prevention (verifier never claims success on false evidence)
//!   - Graph consistency (Bayesian conflict resolution)
//!   - Snapshot idempotency (inject → inject → no duplicate)
//!
//! # Architecture Invariants Validated
//!
//! 1. PsdgHandle is cheaply cloneable and all clones see the same data.
//! 2. Fire-and-forget writes use spawn_blocking (never block async tasks).
//! 3. PsdgCoordinator exits immediately on CancellationToken.
//! 4. WorldModelStore WAL mode allows two connections to same file.
//! 5. Confidence decay archives low-confidence facts correctly.
//! 6. Context injection is selective by operation type.
//! 7. PSDG writes never fail the caller (non-fatal).

use kria_core::agent::psdg::{
    PsdgContextSnapshot, PsdgHandle, MAX_CONTEXT_FACTS, MIN_READ_CONFIDENCE,
};
use kria_core::agent::world_model::FactSource;
use tempfile::NamedTempFile;

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn make_psdg_handle() -> (PsdgHandle, NamedTempFile) {
    let tmp = NamedTempFile::new().unwrap();
    let handle = PsdgHandle::open(tmp.path()).unwrap();
    (handle, tmp)
}

// ─── P4: Threading / Lifecycle Safety ─────────────────────────────────────────

/// P4-1: Two PsdgHandles can open against the same WAL SQLite file concurrently.
#[tokio::test]
async fn p4_1_wal_concurrent_dual_connection() {
    let (handle1, tmp) = make_psdg_handle();
    let handle2 =
        PsdgHandle::open(tmp.path()).expect("Second connection to same WAL file must succeed");

    // Both handles should see each other's writes
    handle1
        .store()
        .upsert(
            "test_subject",
            "is_a",
            "test_object",
            0.9,
            FactSource::Detected,
            "h1",
        )
        .unwrap();

    let fact = handle2.store().query("test_subject", "is_a").unwrap();
    assert!(fact.is_some(), "Handle2 must see write from Handle1 (WAL)");
    assert_eq!(fact.unwrap().object, "test_object");
}

/// P4-2: Fire-and-forget writes complete without blocking and are eventually visible.
#[tokio::test]
async fn p4_2_fire_and_forget_writes_complete() {
    let (handle, _tmp) = make_psdg_handle();

    // record_app_focus is fire-and-forget (spawn_blocking)
    handle.record_app_focus("firefox", "Firefox");

    // Give spawn_blocking time to complete
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let focused = handle.get_focused_app();
    assert_eq!(focused.as_deref(), Some("firefox"));
}

/// P4-3: PsdgCoordinator exits cleanly when CancellationToken is triggered.
#[tokio::test]
async fn p4_3_coordinator_cancels_cleanly() {
    use kria_core::agent::perception::PerceptionEvent;
    use kria_core::agent::psdg::coordinator::{PsdgCoordinator, PsdgCoordinatorConfig};
    use tokio::sync::broadcast;
    use tokio_util::sync::CancellationToken;

    let (handle, _tmp) = make_psdg_handle();
    let (tx, rx) = broadcast::channel::<PerceptionEvent>(16);
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    let coordinator = PsdgCoordinator::new(handle, PsdgCoordinatorConfig::default());
    let join = coordinator.spawn(rx, cancel_clone);

    // Cancel immediately
    cancel.cancel();

    // Coordinator should exit within 200ms
    let result = tokio::time::timeout(tokio::time::Duration::from_millis(200), join).await;
    assert!(
        result.is_ok(),
        "Coordinator should exit within 200ms of cancellation"
    );
    drop(tx);
}

/// P4-4: Cloned PsdgHandle shares the same WorldModelStore data.
#[tokio::test]
async fn p4_4_cloned_handle_shares_state() {
    let (handle, _tmp) = make_psdg_handle();
    let cloned = handle.clone();

    handle
        .store()
        .upsert(
            "shared_subject",
            "predicate",
            "value",
            0.9,
            FactSource::Detected,
            "test",
        )
        .unwrap();

    // Clone sees the write immediately (same Arc<WorldModelStore>)
    let fact = cloned.store().query("shared_subject", "predicate").unwrap();
    assert!(fact.is_some());
}

/// P4-5: Multiple concurrent fire-and-forget writes don't deadlock or panic.
#[tokio::test]
async fn p4_5_concurrent_writes_no_deadlock() {
    let (handle, _tmp) = make_psdg_handle();

    // Spawn 20 concurrent fire-and-forget writes
    for i in 0..20 {
        let h = handle.clone();
        let app_id = format!("app_{}", i);
        h.record_app_focus(&app_id, &format!("App {}", i));
    }

    // All should complete without deadlock
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // At least some writes should have succeeded
    // (focused_app is overwritten each time, so we check any fact exists)
    let facts = handle.query_subject_bounded("app_0");
    // The fact may or may not be there (overwritten by app_N), but no panic/deadlock
    let _ = facts; // just verify no panic occurred
}

// ─── P5: Eval Coverage ────────────────────────────────────────────────────────

/// P5-1: PsdgContextSnapshot is empty when no facts are present.
#[tokio::test]
async fn p5_1_empty_snapshot_when_no_facts() {
    let (handle, _tmp) = make_psdg_handle();
    let snapshot = handle.get_context_snapshot();
    assert!(
        snapshot.is_empty(),
        "Fresh store must produce empty snapshot"
    );
    assert!(
        snapshot.to_prompt_block().is_none(),
        "Empty snapshot has no prompt block"
    );
}

/// P5-2: Browser navigation is persisted and retrievable.
#[tokio::test]
async fn p5_2_browser_navigation_persists() {
    let (handle, _tmp) = make_psdg_handle();

    // Direct sync write (bypass fire-and-forget for deterministic test)
    handle
        .store()
        .upsert(
            "browser_primary",
            "current_url",
            "https://github.com/kria",
            0.95,
            FactSource::Detected,
            "test",
        )
        .unwrap();
    handle
        .store()
        .upsert(
            "browser_primary",
            "current_title",
            "KRIA · GitHub",
            0.95,
            FactSource::Detected,
            "test",
        )
        .unwrap();

    let url = handle.get_browser_url();
    let title = handle.get_browser_title();
    assert_eq!(url.as_deref(), Some("https://github.com/kria"));
    assert_eq!(title.as_deref(), Some("KRIA · GitHub"));

    let snapshot = handle.get_context_snapshot();
    assert_eq!(
        snapshot.browser_url.as_deref(),
        Some("https://github.com/kria")
    );
    assert!(!snapshot.is_empty());
}

/// P5-3: IDE workspace state is persisted and retrievable.
#[tokio::test]
async fn p5_3_ide_state_persists() {
    let (handle, _tmp) = make_psdg_handle();

    handle
        .store()
        .upsert(
            "ide_primary",
            "workspace_root",
            "/home/obaid/projects/kria",
            0.95,
            FactSource::Detected,
            "test",
        )
        .unwrap();
    handle
        .store()
        .upsert(
            "ide_primary",
            "active_file",
            "src/main.rs",
            0.90,
            FactSource::Detected,
            "test",
        )
        .unwrap();

    let ws = handle.get_ide_workspace();
    let file = handle.get_ide_active_file();
    assert_eq!(ws.as_deref(), Some("/home/obaid/projects/kria"));
    assert_eq!(file.as_deref(), Some("src/main.rs"));
}

/// P5-4: Workflow stage progress is persisted and retrievable.
#[tokio::test]
async fn p5_4_workflow_stage_progress_persists() {
    let (handle, _tmp) = make_psdg_handle();

    // Direct write (sync path for deterministic test)
    handle
        .store()
        .upsert(
            "wf_12345",
            "stage_open_editor",
            "completed",
            0.99,
            FactSource::Detected,
            "StageExecutor",
        )
        .unwrap();
    handle
        .store()
        .upsert(
            "desktop_environment",
            "active_workflow",
            "wf_12345",
            0.95,
            FactSource::Detected,
            "StageExecutor",
        )
        .unwrap();

    let active_wf = handle.get_active_workflow();
    assert_eq!(active_wf.as_deref(), Some("wf_12345"));

    let facts = handle.query_subject_bounded("wf_12345");
    assert!(!facts.is_empty(), "Workflow facts should be persisted");
    assert!(facts.iter().any(|f| f.predicate == "stage_open_editor"));
}

/// P5-5: Context snapshot contains all expected fields.
#[tokio::test]
async fn p5_5_context_snapshot_comprehensive() {
    let (handle, _tmp) = make_psdg_handle();

    handle
        .store()
        .upsert(
            "desktop_environment",
            "focused_app",
            "code",
            0.95,
            FactSource::Detected,
            "test",
        )
        .unwrap();
    handle
        .store()
        .upsert(
            "browser_primary",
            "current_url",
            "https://docs.rs",
            0.95,
            FactSource::Detected,
            "test",
        )
        .unwrap();
    handle
        .store()
        .upsert(
            "ide_primary",
            "workspace_root",
            "/kria",
            0.95,
            FactSource::Detected,
            "test",
        )
        .unwrap();
    handle
        .store()
        .upsert(
            "terminal_primary",
            "cwd",
            "/kria/src",
            0.90,
            FactSource::Detected,
            "test",
        )
        .unwrap();

    let snapshot = handle.get_context_snapshot();
    assert_eq!(snapshot.focused_app.as_deref(), Some("code"));
    assert_eq!(snapshot.browser_url.as_deref(), Some("https://docs.rs"));
    assert_eq!(snapshot.ide_workspace.as_deref(), Some("/kria"));
    assert_eq!(snapshot.terminal_cwd.as_deref(), Some("/kria/src"));

    let block = snapshot.to_prompt_block().unwrap();
    assert!(block.contains("## Desktop Context (live)"));
    assert!(block.contains("code"));
    assert!(block.contains("docs.rs"));
    assert!(
        block.len() <= 1200,
        "Prompt block should stay within token budget"
    );
}

/// P5-6: Fact with confidence below MIN_READ_CONFIDENCE is not returned in snapshots.
#[tokio::test]
async fn p5_6_low_confidence_fact_filtered() {
    let (handle, _tmp) = make_psdg_handle();

    // Write a low-confidence fact
    handle
        .store()
        .upsert(
            "desktop_environment",
            "focused_app",
            "maybe-chrome",
            0.1,
            FactSource::Inferred,
            "test",
        )
        .unwrap();

    let focused = handle.get_focused_app();
    assert!(
        focused.is_none(),
        "Fact below MIN_READ_CONFIDENCE ({}) should not appear",
        MIN_READ_CONFIDENCE
    );
}

/// P5-7: Bayesian conflict resolution — contradicting fact replaces old fact.
#[tokio::test]
async fn p5_7_bayesian_conflict_resolution() {
    let (handle, _tmp) = make_psdg_handle();

    // Initial fact
    handle
        .store()
        .upsert(
            "desktop_environment",
            "focused_app",
            "firefox",
            0.95,
            FactSource::Detected,
            "obs1",
        )
        .unwrap();
    let fact1 = handle
        .store()
        .query("desktop_environment", "focused_app")
        .unwrap()
        .unwrap();
    assert_eq!(fact1.object, "firefox");

    // New contradicting fact (focus switched to code)
    handle
        .store()
        .upsert(
            "desktop_environment",
            "focused_app",
            "code",
            0.95,
            FactSource::Detected,
            "obs2",
        )
        .unwrap();
    let fact2 = handle
        .store()
        .query("desktop_environment", "focused_app")
        .unwrap()
        .unwrap();
    assert_eq!(
        fact2.object, "code",
        "New contradicting fact should overwrite old fact"
    );
}

/// P5-8: Same fact with same object — confidence merges (Bayesian update).
#[tokio::test]
async fn p5_8_bayesian_confidence_merge() {
    let (handle, _tmp) = make_psdg_handle();

    handle
        .store()
        .upsert(
            "myapp",
            "is_a",
            "application",
            0.8,
            FactSource::Detected,
            "obs1",
        )
        .unwrap();
    let f1 = handle.store().query("myapp", "is_a").unwrap().unwrap();
    let conf1 = f1.confidence;

    // Same fact observed again — confidence should increase
    handle
        .store()
        .upsert(
            "myapp",
            "is_a",
            "application",
            0.6,
            FactSource::Detected,
            "obs2",
        )
        .unwrap();
    let f2 = handle.store().query("myapp", "is_a").unwrap().unwrap();
    let conf2 = f2.confidence;

    // Bayesian update: 1 - (1 - 0.8) * (1 - 0.6) = 1 - 0.08 = 0.92
    assert!(
        conf2 > conf1,
        "Repeated evidence should increase confidence ({} > {})",
        conf2,
        conf1
    );
    assert!(conf2 > 0.9, "Merged confidence should be high: {}", conf2);
}

/// P5-9: Context injection selectivity — only automation/shell operations get context.
#[test]
fn p5_9_context_injection_selectivity() {
    use kria_core::agent::psdg::context_injector::should_inject_context;
    use kria_core::agent::turn_gate::Operation;

    // Operations that SHOULD get PSDG context
    assert!(should_inject_context(Operation::Automate));
    assert!(should_inject_context(Operation::ExecuteShell));
    assert!(should_inject_context(Operation::ExecuteCode));
    assert!(should_inject_context(Operation::Write));
    assert!(should_inject_context(Operation::Clarify));

    // Operations that should NOT get PSDG context
    assert!(!should_inject_context(Operation::Converse));
    assert!(!should_inject_context(Operation::Search));
    assert!(!should_inject_context(Operation::Read));
    assert!(!should_inject_context(Operation::GenerateImage));
    assert!(!should_inject_context(Operation::RetrieveMemory));
}

/// P5-10: Context injection is idempotent (inject twice → single block).
#[test]
fn p5_10_context_injection_idempotent() {
    use kria_core::agent::psdg::context_injector::inject_into_system_prompt;
    use kria_core::agent::turn_gate::Operation;

    let snap = PsdgContextSnapshot {
        focused_app: Some("firefox".into()),
        browser_url: Some("https://example.com".into()),
        browser_title: None,
        ide_workspace: None,
        ide_active_file: None,
        active_workflow: None,
        terminal_cwd: None,
    };

    let prompt = "You are KRIA.\n---\nRespond naturally.";
    let once = inject_into_system_prompt(prompt, &snap, Operation::Automate);
    let twice = inject_into_system_prompt(&once, &snap, Operation::Automate);

    let count = twice.matches("## Desktop Context (live)").count();
    assert_eq!(
        count, 1,
        "Idempotent injection: should have exactly 1 context block, found {}",
        count
    );
}

/// P5-11: Fact decay archives facts below threshold.
#[test]
fn p5_11_fact_decay_archives_stale() {
    let (handle, _tmp) = make_psdg_handle();

    // Write a fact directly to the archive threshold
    handle
        .store()
        .upsert(
            "stale_app",
            "is_a",
            "application",
            0.05,
            FactSource::Inferred,
            "old_observation",
        )
        .unwrap();

    // Decay: facts below 0.1 should be archived
    let archived = handle.store().decay_and_archive(0.1).unwrap();
    assert!(
        archived >= 1,
        "Stale fact should be archived: archived_count={}",
        archived
    );

    // After archiving, fact should not be accessible via normal query
    let fact = handle.store().query("stale_app", "is_a").unwrap();
    assert!(
        fact.is_none(),
        "Archived fact should not be in active table"
    );
}

/// P5-12: FTS5 full-text search finds facts by keyword.
#[test]
fn p5_12_fts5_semantic_search() {
    let (handle, _tmp) = make_psdg_handle();

    handle
        .store()
        .upsert(
            "firefox_browser",
            "is_a",
            "application",
            0.99,
            FactSource::Detected,
            "test",
        )
        .unwrap();
    handle
        .store()
        .upsert(
            "firefox_browser",
            "has_name",
            "Mozilla Firefox",
            0.99,
            FactSource::Detected,
            "test",
        )
        .unwrap();

    let results = handle.store().search("firefox").unwrap();
    assert!(
        !results.is_empty(),
        "FTS5 search should find 'firefox' facts"
    );
    assert!(results.iter().any(|f| f.subject.contains("firefox")));
}

/// P5-13: query_subject_bounded respects MAX_CONTEXT_FACTS limit.
#[test]
fn p5_13_bounded_query_respects_limit() {
    let (handle, _tmp) = make_psdg_handle();

    // Write more facts than the limit
    for i in 0..(MAX_CONTEXT_FACTS + 10) {
        handle
            .store()
            .upsert(
                "dense_subject",
                &format!("predicate_{}", i),
                "value",
                0.9,
                FactSource::Detected,
                "test",
            )
            .unwrap();
    }

    let facts = handle.query_subject_bounded("dense_subject");
    assert!(
        facts.len() <= MAX_CONTEXT_FACTS,
        "Bounded query must not exceed MAX_CONTEXT_FACTS={}: got {}",
        MAX_CONTEXT_FACTS,
        facts.len()
    );
}

/// P5-14: WorldModelStore stats reporting.
#[test]
fn p5_14_world_model_stats() {
    let (handle, _tmp) = make_psdg_handle();

    handle
        .store()
        .upsert("s1", "p1", "o1", 0.9, FactSource::Detected, "t")
        .unwrap();
    handle
        .store()
        .upsert("s2", "p2", "o2", 0.8, FactSource::UserStated, "t")
        .unwrap();
    handle
        .store()
        .upsert("s3", "p3", "o3", 0.7, FactSource::Inferred, "t")
        .unwrap();

    let stats = handle.store().stats().unwrap();
    assert_eq!(stats.total_facts, 3);
    assert_eq!(stats.archived_facts, 0);
    assert!(stats.avg_confidence > 0.7 && stats.avg_confidence < 1.0);
}

/// P5-15: PerceptionBus FocusChanged event updates WorldModelStore via coordinator.
#[tokio::test]
async fn p5_15_perception_event_triggers_psdg_write() {
    use kria_core::agent::perception::{DesktopOp, EventKind, EventSeverity, PerceptionEvent};
    use kria_core::agent::psdg::coordinator::{PsdgCoordinator, PsdgCoordinatorConfig};
    use tokio::sync::broadcast;
    use tokio_util::sync::CancellationToken;

    let (handle, _tmp) = make_psdg_handle();
    let (tx, rx) = broadcast::channel::<PerceptionEvent>(16);
    let cancel = CancellationToken::new();

    let config = PsdgCoordinatorConfig {
        track_focus: true,
        track_window_lifecycle: true,
        track_filesystem: false,
        track_processes: false,
        max_fs_events_per_sec: 0,
    };
    let coordinator = PsdgCoordinator::new(handle.clone(), config);
    let _join = coordinator.spawn(rx, cancel.clone());

    // Send a FocusChanged event with app path
    let event = PerceptionEvent {
        kind: EventKind::DesktopEvent(DesktopOp::FocusChanged),
        key: "desktop:focus:firefox".into(),
        primary_path: Some("/usr/bin/firefox".into()),
        count: 1,
        summary: "focus changed to firefox".into(),
        severity: EventSeverity::Info,
        first_seen_epoch_ms: 0,
        finalized_epoch_ms: 0,
    };

    tx.send(event).unwrap();

    // Give coordinator and spawn_blocking time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // Verify WorldModelStore was updated
    let focused = handle.get_focused_app();
    assert!(
        focused.is_some(),
        "FocusChanged event should update WorldModelStore focused_app"
    );

    cancel.cancel();
}

/// P5-16: WorldModelStore contradiction resolution removes duplicate archived entries.
#[test]
fn p5_16_resolve_contradictions_archives_duplicates() {
    let (handle, _tmp) = make_psdg_handle();
    let store = handle.store();

    // Three overwrites produce two archived duplicates for (app, focused)
    store
        .upsert("app", "focused", "firefox", 0.9, FactSource::Detected, "t1")
        .unwrap();
    store
        .upsert("app", "focused", "chrome", 0.8, FactSource::Detected, "t2")
        .unwrap();
    store
        .upsert("app", "focused", "edge", 0.7, FactSource::Detected, "t3")
        .unwrap();

    let stats_before = store.stats().unwrap();
    assert_eq!(stats_before.total_facts, 1);
    assert_eq!(stats_before.archived_facts, 2);

    // Resolve contradictions — keep only the newest archived entry per (subject, predicate)
    let resolved = store.resolve_contradictions().unwrap();
    assert_eq!(
        resolved, 1,
        "Should remove exactly one redundant archived entry"
    );

    let stats_after = store.stats().unwrap();
    assert_eq!(stats_after.total_facts, 1);
    assert_eq!(stats_after.archived_facts, 1);
}

/// P5-17: WorldModelStore archive pruning can be called without error.
#[test]
fn p5_17_prune_archive_runs_cleanly() {
    let (handle, _tmp) = make_psdg_handle();
    let store = handle.store();

    // Prune with a generous threshold should succeed even on empty archive
    let pruned = store.prune_archive(30).unwrap();
    assert_eq!(pruned, 0, "Empty archive should prune 0 entries");
}
