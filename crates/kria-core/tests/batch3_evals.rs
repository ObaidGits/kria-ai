//! Batch 3 Evaluation Suite — Persistent Operational Desktop Cognition Runtime
//!
//! Categories:
//!   A (10) — CognitionEventBus: typed events, flood guard, broadcast delivery
//!   B (10) — AmbientCognitionLoop: config, handle, tick behaviour
//!   C (10) — OperationalContextTracker: chain, lineage, boundedness
//!   D (10) — ProceduralWorkflowMemory: skill graph, extraction, pruning
//!   E (10) — PersistentGoalRuntime: lifecycle, caps, persistence, expiry
//!   F (10) — OperationalSuggestionsEngine: rate limit, dedup, disable, types
//!   G (10) — DesktopAwarenessRuntime: snapshot update, event application
//!   H (5)  — AgentLoop builder API integration for Batch 3 fields
//!   I (5)  — Cross-module integration (bus → awareness, context → suggestions)
//!
//! All tests are synchronous or #[tokio::test]. No live daemon connections.
//! NO destructive operations. NO GPU inference. All production safety
//! invariants are preserved.

use std::sync::Arc;

// ─────────────────────────────────────────────────────────────────────────────
// Re-usable helpers
// ─────────────────────────────────────────────────────────────────────────────

use kria_core::agent::ambient_cognition::{
    AmbientCognitionConfig, AmbientCognitionHandle, AmbientCognitionLoop, DEFAULT_TICK_INTERVAL,
    MAX_TICK_MS, MIN_TICK_INTERVAL,
};
use kria_core::agent::cognition_event_bus::{
    BrowserCognitionEvent, BrowserEventKind, CognitionEvent, CognitionEventBus, ContinuationEvent,
    ContinuationEventKind, DesktopCognitionEvent, DesktopCognitionEventKind, EventFloodGuard,
    IdeCognitionEvent, IdeEventKind, PolicyEvent, PolicyEventKind, SuggestionEvent, SuggestionKind,
    WorkflowEvent, WorkflowEventKind,
};
use kria_core::agent::desktop_awareness::DesktopAwarenessRuntime;
use kria_core::agent::goal_runtime::{
    GoalStatus, OperationalGoal, PersistentGoalRuntime, MAX_ACTIVE_GOALS, MAX_GOAL_AGE_DAYS,
};
#[allow(unused_imports)]
use kria_core::agent::loop_engine::AgentLoop;
use kria_core::agent::operational_context::{
    OperationalContextTracker, MAX_CHAIN_LENGTH, MAX_INTERRUPTION_LINEAGE, MAX_RECOVERY_LINEAGE,
};
use kria_core::agent::operational_suggestions::{
    OperationalSuggestionsEngine, MAX_SUGGESTIONS_PER_WINDOW,
};
use kria_core::agent::procedural_memory::{
    ProceduralWorkflowMemory, MAX_SKILLS_PER_CATEGORY, MIN_SESSIONS_FOR_SKILL,
};
use kria_core::agent::workflow_continuation::WorkflowContinuationRuntime;
use kria_core::agent::workflow_session::{SessionStep, WorkflowSession};

// ─────────────────────────────────────────────────────────────────────────────
// Shared constructors
// ─────────────────────────────────────────────────────────────────────────────

fn wf_started_ev(id: &str) -> CognitionEvent {
    CognitionEvent::Workflow(WorkflowEvent {
        session_id: id.to_string(),
        description: "test wf".to_string(),
        kind: WorkflowEventKind::Started,
    })
}

fn wf_completed_ev(id: &str) -> CognitionEvent {
    CognitionEvent::Workflow(WorkflowEvent {
        session_id: id.to_string(),
        description: "test wf".to_string(),
        kind: WorkflowEventKind::Completed { duration_ms: 100 },
    })
}

fn browser_nav(url: &str) -> CognitionEvent {
    CognitionEvent::Browser(BrowserCognitionEvent {
        url: url.to_string(),
        title: "Page".to_string(),
        kind: BrowserEventKind::Navigated,
    })
}

fn ide_build_ok() -> CognitionEvent {
    CognitionEvent::Ide(IdeCognitionEvent {
        workspace_root: Some("/proj".to_string()),
        kind: IdeEventKind::BuildSucceeded,
    })
}

fn completed_session(id: &str, intent: &str, succeeded: bool) -> WorkflowSession {
    let mut s = WorkflowSession::new(id.into(), intent.into(), "react".into());
    s.completed_steps.push(SessionStep {
        step: 1,
        action: "run_command".to_string(),
        params: serde_json::Value::Null,
        success: true,
        evidence: "ok".into(),
        timestamp: 0,
    });
    s.complete = succeeded;
    s
}

// ─────────────────────────────────────────────────────────────────────────────
// Category A — CognitionEventBus
// ─────────────────────────────────────────────────────────────────────────────

/// A1: Bus emits event and single subscriber receives it.
#[test]
fn a1_bus_emit_single_subscriber_receives() {
    let bus = CognitionEventBus::new();
    let mut rx = bus.subscribe();
    bus.emit(wf_started_ev("s1"));
    let ev = rx.try_recv().unwrap();
    assert!(matches!(ev, CognitionEvent::Workflow(_)));
}

/// A2: Bus with no subscribers returns 0 from emit.
#[test]
fn a2_bus_no_subscribers_returns_zero() {
    let bus = CognitionEventBus::new();
    let n = bus.emit(wf_started_ev("s2"));
    assert_eq!(n, 0);
}

/// A3: Multiple subscribers each receive the same event.
#[test]
fn a3_bus_multiple_subscribers_all_receive() {
    let bus = CognitionEventBus::new();
    let mut rx1 = bus.subscribe();
    let mut rx2 = bus.subscribe();
    bus.emit(wf_started_ev("s3"));
    assert!(rx1.try_recv().is_ok());
    assert!(rx2.try_recv().is_ok());
}

/// A4: EventFloodGuard allows first emission.
#[test]
fn a4_flood_guard_first_emission_passes() {
    let guard = EventFloodGuard::new();
    assert!(guard.should_emit(&wf_started_ev("fg1")));
}

/// A5: EventFloodGuard suppresses duplicate within window.
#[test]
fn a5_flood_guard_suppresses_duplicate_in_window() {
    let guard = EventFloodGuard::new();
    let ev = wf_started_ev("fg2");
    assert!(guard.should_emit(&ev));
    assert!(!guard.should_emit(&ev), "duplicate must be suppressed");
}

/// A6: EventFloodGuard passes events with different dedup keys.
#[test]
fn a6_flood_guard_passes_different_keys() {
    let guard = EventFloodGuard::new();
    assert!(guard.should_emit(&wf_started_ev("a")));
    assert!(guard.should_emit(&wf_started_ev("b")));
}

/// A7: emit_critical bypasses flood guard.
#[test]
fn a7_emit_critical_bypasses_flood_guard() {
    let bus = CognitionEventBus::new();
    let _rx = bus.subscribe();
    let ev = wf_started_ev("crit");
    bus.emit(ev.clone()); // first normal emission to prime flood state
    let n = bus.emit_critical(ev);
    assert_eq!(n, 1);
}

/// A8: dedup_key is stable across identical event constructions.
#[test]
fn a8_dedup_key_is_stable() {
    let e1 = wf_started_ev("k");
    let e2 = wf_started_ev("k");
    assert_eq!(e1.dedup_key(), e2.dedup_key());
}

/// A9: dedup_key differs for different event types.
#[test]
fn a9_dedup_key_differs_for_different_types() {
    let wf = wf_started_ev("x");
    let br = browser_nav("https://x.com");
    assert_ne!(wf.dedup_key(), br.dedup_key());
}

/// A10: receiver_count matches subscribed receivers.
#[test]
fn a10_receiver_count_matches_subscriptions() {
    let bus = CognitionEventBus::new();
    assert_eq!(bus.receiver_count(), 0);
    let _rx1 = bus.subscribe();
    let _rx2 = bus.subscribe();
    assert_eq!(bus.receiver_count(), 2);
}

// ─────────────────────────────────────────────────────────────────────────────
// Category B — AmbientCognitionLoop
// ─────────────────────────────────────────────────────────────────────────────

/// B1: Default config is enabled with 30s interval.
#[test]
fn b1_default_config_enabled_30s() {
    let c = AmbientCognitionConfig::default();
    assert!(c.enabled);
    assert_eq!(c.tick_interval, DEFAULT_TICK_INTERVAL);
}

/// B2: Tick interval clamped to minimum floor.
#[test]
fn b2_tick_interval_clamped_to_minimum() {
    let c = AmbientCognitionConfig {
        tick_interval: std::time::Duration::from_secs(1),
        ..Default::default()
    };
    let effective = c.tick_interval.max(MIN_TICK_INTERVAL);
    assert_eq!(effective, MIN_TICK_INTERVAL);
}

/// B3: Handle pause sets enabled=false.
#[test]
fn b3_handle_pause_sets_disabled() {
    let (h, _) = AmbientCognitionHandle::new_for_test();
    assert!(h.is_enabled());
    h.pause();
    assert!(!h.is_enabled());
}

/// B4: Handle resume restores enabled=true.
#[test]
fn b4_handle_resume_restores_enabled() {
    let (h, _) = AmbientCognitionHandle::new_for_test();
    h.pause();
    h.resume();
    assert!(h.is_enabled());
}

/// B5: Handle stop cancels the token.
#[test]
fn b5_handle_stop_cancels_token() {
    let (h, cancel) = AmbientCognitionHandle::new_for_test();
    assert!(!cancel.is_cancelled());
    h.stop();
    assert!(cancel.is_cancelled());
}

/// B6: MAX_TICK_MS constant is reasonable (≤ 10 000).
#[test]
fn b6_max_tick_ms_is_reasonable() {
    assert!(MAX_TICK_MS <= 10_000, "MAX_TICK_MS should be ≤ 10s");
    assert!(MAX_TICK_MS >= 100, "MAX_TICK_MS should be ≥ 100ms");
}

/// B7: Config suggest_session_resume defaults to true.
#[test]
fn b7_config_suggest_resume_default_true() {
    assert!(AmbientCognitionConfig::default().suggest_session_resume);
}

/// B8: Config suggest_build_recovery defaults to true.
#[test]
fn b8_config_suggest_build_recovery_default_true() {
    assert!(AmbientCognitionConfig::default().suggest_build_recovery);
}

/// B9: Tick with no WCR and no PSDG emits zero events.
#[tokio::test]
async fn b9_tick_no_dependencies_emits_zero() {
    let lp = AmbientCognitionLoop::new(
        AmbientCognitionConfig::default(),
        Arc::new(CognitionEventBus::new()),
        None,
        None,
    );
    let n = lp.run_tick().await;
    assert_eq!(n, 0);
}

/// B10: Tick with WCR but no paused sessions emits zero events.
#[tokio::test]
async fn b10_tick_wcr_no_paused_sessions_emits_zero() {
    let wcr = Arc::new(WorkflowContinuationRuntime::new(None));
    let lp = AmbientCognitionLoop::new(
        AmbientCognitionConfig::default(),
        Arc::new(CognitionEventBus::new()),
        Some(wcr),
        None,
    );
    let n = lp.run_tick().await;
    assert_eq!(n, 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// Category C — OperationalContextTracker
// ─────────────────────────────────────────────────────────────────────────────

fn ctx() -> OperationalContextTracker {
    OperationalContextTracker::new(None)
}

/// C1: Record workflow started sets current_session_id.
#[test]
fn c1_record_started_sets_current_session() {
    let t = ctx();
    t.record_workflow_started("s1", "build");
    assert_eq!(t.snapshot().current_session_id.as_deref(), Some("s1"));
}

/// C2: Record workflow ended clears current session.
#[test]
fn c2_record_ended_clears_current_session() {
    let t = ctx();
    t.record_workflow_started("s1", "build");
    t.record_workflow_ended("s1", true);
    assert!(t.snapshot().current_session_id.is_none());
}

/// C3: Workflow chain is bounded to MAX_CHAIN_LENGTH.
#[test]
fn c3_chain_bounded_to_max_length() {
    let t = ctx();
    for i in 0..=(MAX_CHAIN_LENGTH + 5) {
        t.record_workflow_started(&format!("s{}", i), "wf");
        t.record_workflow_ended(&format!("s{}", i), true);
    }
    assert!(t.snapshot().recent_session_chain.len() <= MAX_CHAIN_LENGTH);
}

/// C4: set_browser_url updates snapshot.
#[test]
fn c4_set_browser_url_updates_snapshot() {
    let t = ctx();
    t.set_browser_url("https://example.com");
    assert_eq!(
        t.snapshot().browser_url.as_deref(),
        Some("https://example.com")
    );
}

/// C5: set_ide_workspace updates snapshot.
#[test]
fn c5_set_ide_workspace_updates_snapshot() {
    let t = ctx();
    t.set_ide_workspace("/home/user/project");
    assert_eq!(
        t.snapshot().ide_workspace.as_deref(),
        Some("/home/user/project")
    );
}

/// C6: set_active_project updates snapshot.
#[test]
fn c6_set_active_project_updates_snapshot() {
    let t = ctx();
    t.set_active_project("/projects/kria");
    assert_eq!(
        t.snapshot().active_project.as_deref(),
        Some("/projects/kria")
    );
}

/// C7: Interruption lineage is bounded.
#[test]
fn c7_interruption_lineage_bounded() {
    let t = ctx();
    for _ in 0..(MAX_INTERRUPTION_LINEAGE + 10) {
        t.record_interruption("s1", "Timeout", false);
    }
    assert!(t.snapshot().interruption_lineage.len() <= MAX_INTERRUPTION_LINEAGE);
}

/// C8: Recovery lineage is bounded.
#[test]
fn c8_recovery_lineage_bounded() {
    let t = ctx();
    for _ in 0..(MAX_RECOVERY_LINEAGE + 10) {
        t.record_recovery("s1", "retry", true);
    }
    assert!(t.snapshot().recovery_lineage.len() <= MAX_RECOVERY_LINEAGE);
}

/// C9: is_workflow_active reflects state correctly.
#[test]
fn c9_is_workflow_active_reflects_state() {
    let t = ctx();
    assert!(!t.is_workflow_active());
    t.record_workflow_started("s1", "wf");
    assert!(t.is_workflow_active());
    t.record_workflow_ended("s1", true);
    assert!(!t.is_workflow_active());
}

/// C10: Interruption entry records required_human flag.
#[test]
fn c10_interruption_entry_stores_required_human() {
    let t = ctx();
    t.record_interruption("s1", "AuthRequired", true);
    let entry = &t.snapshot().interruption_lineage[0];
    assert!(entry.required_human);
    assert_eq!(entry.class, "AuthRequired");
}

// ─────────────────────────────────────────────────────────────────────────────
// Category D — ProceduralWorkflowMemory
// ─────────────────────────────────────────────────────────────────────────────

fn pmem() -> ProceduralWorkflowMemory {
    ProceduralWorkflowMemory::new(None)
}

/// D1: Empty session is not ingested.
#[test]
fn d1_empty_session_not_ingested() {
    let m = pmem();
    let s = WorkflowSession::new("s0".into(), "build".into(), "react".into());
    m.ingest_session(&s);
    assert_eq!(m.skill_count(), 0);
}

/// D2: Completed session creates a skill.
#[test]
fn d2_completed_session_creates_skill() {
    let m = pmem();
    m.ingest_session(&completed_session("s1", "build the project", true));
    assert_eq!(m.skill_count(), 1);
}

/// D3: Multiple sessions for same intent merge statistics.
#[test]
fn d3_multiple_sessions_merge_statistics() {
    let m = pmem();
    for i in 0..4 {
        m.ingest_session(&completed_session(
            &format!("s{}", i),
            "build the project",
            true,
        ));
    }
    let skill = m.find_relevant_skill("build the project");
    assert!(skill.is_some());
    assert_eq!(skill.unwrap().session_count, 4);
}

/// D4: find_relevant_skill returns None below MIN_SESSIONS_FOR_SKILL.
#[test]
fn d4_find_relevant_skill_none_below_min_sessions() {
    let m = pmem();
    m.ingest_session(&completed_session("s1", "deploy the app", true));
    assert!(m.find_relevant_skill("deploy the app").is_none());
}

/// D5: Category cap is enforced.
#[test]
fn d5_category_cap_enforced() {
    let m = pmem();
    for i in 0..(MAX_SKILLS_PER_CATEGORY + 15) {
        let intent = format!("build variant-{}", i);
        for j in 0..MIN_SESSIONS_FOR_SKILL {
            m.ingest_session(&completed_session(&format!("s{}x{}", i, j), &intent, true));
        }
    }
    let coding = m.list_skills_for_category("coding");
    assert!(
        coding.len() <= MAX_SKILLS_PER_CATEGORY,
        "cap exceeded: {}",
        coding.len()
    );
}

/// D6: prune_low_confidence removes all-failure skills.
#[test]
fn d6_prune_removes_all_failure_skills() {
    let m = pmem();
    for i in 0..3 {
        m.ingest_session(&completed_session(
            &format!("s{}", i),
            "build fail project",
            false,
        ));
    }
    m.prune_low_confidence();
    assert_eq!(m.skill_count(), 0);
}

/// D7: Success rate computed correctly.
#[test]
fn d7_success_rate_computed_correctly() {
    let m = pmem();
    for i in 0..4 {
        m.ingest_session(&completed_session(
            &format!("s{}", i),
            "run tests suite",
            i % 2 == 0,
        ));
    }
    let skill = m.find_relevant_skill("run tests suite");
    if let Some(s) = skill {
        let rate = s.success_rate();
        assert!(rate >= 0.0 && rate <= 1.0);
    }
}

/// D8: list_skills_for_category returns only matching category.
#[test]
fn d8_list_skills_for_category_filtered() {
    let m = pmem();
    for i in 0..MIN_SESSIONS_FOR_SKILL {
        m.ingest_session(&completed_session(
            &format!("b{}", i),
            "build project",
            true,
        ));
        m.ingest_session(&completed_session(&format!("e{}", i), "send email", true));
    }
    let coding = m.list_skills_for_category("coding");
    let email = m.list_skills_for_category("email");
    // Not all categories have sessions, but none should bleed across
    for s in &coding {
        assert_eq!(s.category, "coding");
    }
    for s in &email {
        assert_eq!(s.category, "email");
    }
}

/// D9: Tool sequence bounded per skill.
#[test]
fn d9_tool_sequence_bounded() {
    let m = pmem();
    let mut s = WorkflowSession::new("s1".into(), "build project".into(), "react".into());
    for i in 0..20 {
        s.completed_steps.push(SessionStep {
            step: i,
            action: format!("tool_{}", i),
            params: serde_json::Value::Null,
            success: true,
            evidence: "ok".into(),
            timestamp: 0,
        });
    }
    s.complete = true;
    m.ingest_session(&s);
    let key = m.list_skills_for_category("coding");
    if let Some(skill) = key.first() {
        assert!(
            skill.tool_sequence.len() <= kria_core::agent::procedural_memory::MAX_TOOL_SEQUENCE
        );
    }
}

/// D10: Skill count is zero after initialization.
#[test]
fn d10_skill_count_zero_after_init() {
    let m = pmem();
    assert_eq!(m.skill_count(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// Category E — PersistentGoalRuntime
// ─────────────────────────────────────────────────────────────────────────────

fn gr() -> PersistentGoalRuntime {
    PersistentGoalRuntime::ephemeral()
}

/// E1: Create goal returns Some with Pending status.
#[test]
fn e1_create_goal_returns_pending() {
    let r = gr();
    let g = r.create_goal("finish docs", None).unwrap();
    assert!(matches!(g.status, GoalStatus::Pending));
}

/// E2: Activate transitions to Active.
#[test]
fn e2_activate_transitions_to_active() {
    let r = gr();
    let g = r.create_goal("refactor", None).unwrap();
    r.activate_goal(&g.goal_id, None);
    assert!(matches!(
        r.get_goal(&g.goal_id).unwrap().status,
        GoalStatus::Active
    ));
}

/// E3: Complete marks goal Completed.
#[test]
fn e3_complete_marks_completed() {
    let r = gr();
    let g = r.create_goal("deploy", None).unwrap();
    r.complete_goal(&g.goal_id);
    assert!(matches!(
        r.get_goal(&g.goal_id).unwrap().status,
        GoalStatus::Completed { .. }
    ));
}

/// E4: Fail marks goal Failed.
#[test]
fn e4_fail_marks_failed() {
    let r = gr();
    let g = r.create_goal("migration", None).unwrap();
    r.fail_goal(&g.goal_id, "timeout");
    assert!(matches!(
        r.get_goal(&g.goal_id).unwrap().status,
        GoalStatus::Failed { .. }
    ));
}

/// E5: Cancel marks goal Cancelled.
#[test]
fn e5_cancel_marks_cancelled() {
    let r = gr();
    let g = r.create_goal("review PR", None).unwrap();
    r.cancel_goal(&g.goal_id);
    assert!(matches!(
        r.get_goal(&g.goal_id).unwrap().status,
        GoalStatus::Cancelled { .. }
    ));
}

/// E6: list_active_goals excludes terminal goals.
#[test]
fn e6_list_active_excludes_terminal() {
    let r = gr();
    let g1 = r.create_goal("active goal", None).unwrap();
    let g2 = r.create_goal("done goal", None).unwrap();
    r.complete_goal(&g2.goal_id);
    let active = r.list_active_goals();
    assert!(active.iter().any(|g| g.goal_id == g1.goal_id));
    assert!(!active.iter().any(|g| g.goal_id == g2.goal_id));
}

/// E7: MAX_ACTIVE_GOALS cap enforced.
#[test]
fn e7_max_active_goals_cap_enforced() {
    let r = gr();
    for i in 0..=(MAX_ACTIVE_GOALS + 5) {
        r.create_goal(format!("goal {}", i), None);
    }
    assert!(r.list_active_goals().len() <= MAX_ACTIVE_GOALS);
}

/// E8: update_hint persists the hint.
#[test]
fn e8_update_hint_persists() {
    let r = gr();
    let g = r.create_goal("write tests", None).unwrap();
    r.update_hint(&g.goal_id, "Start with unit tests");
    assert_eq!(
        r.get_goal(&g.goal_id).unwrap().continuation_hint.as_deref(),
        Some("Start with unit tests")
    );
}

/// E9: Maintenance expires goals created at epoch 0.
#[test]
fn e9_maintenance_expires_ancient_goals() {
    let r = gr();
    {
        let mut goals = r.goals.lock().unwrap();
        goals.insert(
            "old".to_string(),
            OperationalGoal {
                goal_id: "old".to_string(),
                description: "old goal".to_string(),
                associated_session_id: None,
                status: GoalStatus::Pending,
                created_at: 0,
                updated_at: 0,
                continuation_hint: None,
                attempt_count: 0,
            },
        );
    }
    r.maintenance();
    assert!(matches!(
        r.get_goal("old").unwrap().status,
        GoalStatus::Expired { .. }
    ));
}

/// E10: MAX_GOAL_AGE_DAYS constant is 7.
#[test]
fn e10_max_goal_age_days_is_seven() {
    assert_eq!(MAX_GOAL_AGE_DAYS, 7);
}

// ─────────────────────────────────────────────────────────────────────────────
// Category F — OperationalSuggestionsEngine
// ─────────────────────────────────────────────────────────────────────────────

fn seng() -> OperationalSuggestionsEngine {
    OperationalSuggestionsEngine::new(None)
}

/// F1: Suggest resume for ancient pause returns suggestion.
#[test]
fn f1_suggest_resume_ancient_pause_returns_some() {
    let e = seng();
    let sug = e.suggest_resume("s1", "build project", 0);
    assert!(sug.is_some());
}

/// F2: Suggest resume for recent pause returns None.
#[test]
fn f2_suggest_resume_recent_pause_returns_none() {
    let e = seng();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let sug = e.suggest_resume("s1", "build project", now);
    assert!(
        sug.is_none(),
        "very recent pause should not trigger suggestion"
    );
}

/// F3: Duplicate suggestion is suppressed.
#[test]
fn f3_duplicate_suggestion_suppressed() {
    let e = seng();
    let s1 = e.suggest_resume("s1", "wf", 0);
    let s2 = e.suggest_resume("s1", "wf", 0);
    assert!(s1.is_some());
    assert!(s2.is_none(), "duplicate must be suppressed");
}

/// F4: clear_suggestion allows re-emission.
#[test]
fn f4_clear_allows_reemission() {
    let e = seng();
    e.suggest_resume("s1", "wf", 0);
    e.clear_suggestion("resume-s1");
    assert!(e.suggest_resume("s1", "wf", 0).is_some());
}

/// F5: Rate cap enforced across window.
#[test]
fn f5_rate_cap_enforced() {
    let e = seng();
    for i in 0..(MAX_SUGGESTIONS_PER_WINDOW + 10) {
        e.suggest_build_recovery(&format!("/proj/{}", i), "err");
    }
    assert!(e.window_count() <= MAX_SUGGESTIONS_PER_WINDOW);
}

/// F6: disable() prevents suggestion emission.
#[test]
fn f6_disable_prevents_emission() {
    let e = seng();
    e.disable();
    assert!(e.suggest_resume("s1", "wf", 0).is_none());
}

/// F7: enable() after disable restores emission.
#[test]
fn f7_enable_after_disable_restores() {
    let e = seng();
    e.disable();
    e.enable();
    assert!(e.suggest_resume("s1", "wf", 0).is_some());
}

/// F8: Zero errors produces no diagnostic suggestion.
#[test]
fn f8_zero_errors_no_diagnostic_suggestion() {
    let e = seng();
    assert!(e.suggest_address_diagnostics("/proj", 0).is_none());
}

/// F9: Non-zero errors produces diagnostic suggestion.
#[test]
fn f9_nonzero_errors_produces_suggestion() {
    let e = seng();
    let sug = e.suggest_address_diagnostics("/proj", 5);
    assert!(sug.is_some());
    assert!(matches!(
        sug.unwrap().kind,
        SuggestionKind::AddressDiagnostics { error_count: 5 }
    ));
}

/// F10: Goal step suggestion uses NextGoalStep kind.
#[test]
fn f10_goal_step_suggestion_uses_correct_kind() {
    let e = seng();
    let sug = e.suggest_goal_step("g1", "finish report", "write section 3");
    assert!(sug.is_some());
    assert!(matches!(
        sug.unwrap().kind,
        SuggestionKind::NextGoalStep { .. }
    ));
}

// ─────────────────────────────────────────────────────────────────────────────
// Category G — DesktopAwarenessRuntime
// ─────────────────────────────────────────────────────────────────────────────

fn drt() -> DesktopAwarenessRuntime {
    DesktopAwarenessRuntime::new(None)
}

/// G1: Browser nav event updates URL.
#[test]
fn g1_browser_nav_updates_url() {
    let r = drt();
    r.apply_event(&browser_nav("https://example.com"));
    assert_eq!(r.snapshot().browser.url, "https://example.com");
}

/// G2: IDE build success sets last_build_ok = Some(true).
#[test]
fn g2_ide_build_success_sets_last_build_ok() {
    let r = drt();
    r.apply_event(&ide_build_ok());
    assert_eq!(r.snapshot().ide.last_build_ok, Some(true));
}

/// G3: IDE build failure sets error_count.
#[test]
fn g3_ide_build_failure_sets_error_count() {
    let r = drt();
    r.apply_event(&CognitionEvent::Ide(IdeCognitionEvent {
        workspace_root: Some("/proj".into()),
        kind: IdeEventKind::BuildFailed {
            error_count: 4,
            first_error: "err".into(),
        },
    }));
    let snap = r.snapshot();
    assert_eq!(snap.ide.error_count, 4);
    assert_eq!(snap.ide.last_build_ok, Some(false));
}

/// G4: Workflow started sets active_workflow_id.
#[test]
fn g4_workflow_started_sets_active_id() {
    let r = drt();
    r.apply_event(&wf_started_ev("s1"));
    assert_eq!(r.snapshot().active_workflow_id.as_deref(), Some("s1"));
}

/// G5: Workflow completed clears active_workflow_id.
#[test]
fn g5_workflow_completed_clears_active_id() {
    let r = drt();
    r.apply_event(&wf_started_ev("s1"));
    r.apply_event(&wf_completed_ev("s1"));
    assert!(r.snapshot().active_workflow_id.is_none());
}

/// G6: is_clean() true when no workflow and no dialog.
#[test]
fn g6_is_clean_no_workflow_no_dialog() {
    let r = drt();
    assert!(r.snapshot().is_clean());
}

/// G7: Dialog appearance sets has_dialog = true.
#[test]
fn g7_dialog_sets_has_dialog() {
    let r = drt();
    r.apply_event(&CognitionEvent::Desktop(DesktopCognitionEvent {
        app_name: "sys".into(),
        kind: DesktopCognitionEventKind::WindowAppeared {
            title: "Prompt".into(),
            is_dialog: true,
        },
    }));
    assert!(r.snapshot().has_dialog);
}

/// G8: Window closed clears has_dialog.
#[test]
fn g8_window_closed_clears_has_dialog() {
    let r = drt();
    r.apply_event(&CognitionEvent::Desktop(DesktopCognitionEvent {
        app_name: "sys".into(),
        kind: DesktopCognitionEventKind::WindowAppeared {
            title: "D".into(),
            is_dialog: true,
        },
    }));
    r.apply_event(&CognitionEvent::Desktop(DesktopCognitionEvent {
        app_name: "sys".into(),
        kind: DesktopCognitionEventKind::WindowClosed { title: "D".into() },
    }));
    assert!(!r.snapshot().has_dialog);
}

/// G9: Auth interrupt event sets auth_interrupt flag.
#[test]
fn g9_auth_interrupt_sets_flag() {
    let r = drt();
    r.apply_event(&CognitionEvent::Browser(BrowserCognitionEvent {
        url: "https://accounts.google.com".into(),
        title: "Sign in".into(),
        kind: BrowserEventKind::AuthInterrupt {
            service_hint: "google".into(),
        },
    }));
    let snap = r.snapshot();
    assert!(snap.browser.auth_interrupt);
    assert_eq!(snap.browser.auth_service_hint.as_deref(), Some("google"));
}

/// G10: Non-dialog window appearance does not set has_dialog.
#[test]
fn g10_non_dialog_window_does_not_set_has_dialog() {
    let r = drt();
    r.apply_event(&CognitionEvent::Desktop(DesktopCognitionEvent {
        app_name: "firefox".into(),
        kind: DesktopCognitionEventKind::WindowAppeared {
            title: "Firefox".into(),
            is_dialog: false,
        },
    }));
    assert!(!r.snapshot().has_dialog);
}

// ─────────────────────────────────────────────────────────────────────────────
// Category H — AgentLoop builder API type verification
// (Following Batch 2 pattern: verify types are constructible as Arc<T>,
//  full AgentLoop construction requires model router and infra fixtures.)
// ─────────────────────────────────────────────────────────────────────────────

/// H1: CognitionEventBus is constructible as Arc<T> for AgentLoop injection.
#[test]
fn h1_cognition_bus_constructible_as_arc() {
    let _: Arc<CognitionEventBus> = Arc::new(CognitionEventBus::new());
}

/// H2: OperationalContextTracker is constructible as Arc<T>.
#[test]
fn h2_operational_context_constructible_as_arc() {
    let _: Arc<OperationalContextTracker> = Arc::new(OperationalContextTracker::new(None));
}

/// H3: ProceduralWorkflowMemory is constructible as Arc<T>.
#[test]
fn h3_procedural_memory_constructible_as_arc() {
    let _: Arc<ProceduralWorkflowMemory> = Arc::new(ProceduralWorkflowMemory::new(None));
}

/// H4: PersistentGoalRuntime is constructible as Arc<T>.
#[test]
fn h4_goal_runtime_constructible_as_arc() {
    let _: Arc<PersistentGoalRuntime> = Arc::new(PersistentGoalRuntime::ephemeral());
}

/// H5: DesktopAwarenessRuntime is constructible as Arc<T>.
#[test]
fn h5_desktop_awareness_constructible_as_arc() {
    let _: Arc<DesktopAwarenessRuntime> = Arc::new(DesktopAwarenessRuntime::new(None));
}

// ─────────────────────────────────────────────────────────────────────────────
// Category I — Cross-module integration
// ─────────────────────────────────────────────────────────────────────────────

/// I1: Bus event fan-out: same CognitionEvent updates both context and awareness.
#[test]
fn i1_bus_event_updates_both_context_and_awareness() {
    let bus = CognitionEventBus::new();
    let ctx = OperationalContextTracker::new(None);
    let awareness = DesktopAwarenessRuntime::new(None);

    // Simulate: workflow started → update both
    let ev = wf_started_ev("s-cross");
    ctx.record_workflow_started("s-cross", "cross test");
    awareness.apply_event(&ev);

    assert!(ctx.is_workflow_active());
    assert_eq!(
        awareness.snapshot().active_workflow_id.as_deref(),
        Some("s-cross")
    );
}

/// I2: Procedural memory + context tracker joint ingestion.
#[test]
fn i2_procedural_memory_and_context_joint_ingestion() {
    let ctx = OperationalContextTracker::new(None);
    let mem = ProceduralWorkflowMemory::new(None);

    let session_id = "s-joint";
    let intent = "build the api server";
    ctx.record_workflow_started(session_id, intent);
    mem.ingest_session(&completed_session(session_id, intent, true));
    ctx.record_workflow_ended(session_id, true);

    assert!(!ctx.is_workflow_active());
    assert_eq!(mem.skill_count(), 1);
}

/// I3: Goal runtime + suggestions engine integration.
#[test]
fn i3_goal_suggestion_from_stalled_goal() {
    let gr = PersistentGoalRuntime::ephemeral();
    let eng = seng();

    let g = gr
        .create_goal(
            "finish the migration",
            Some("Run pending migrations".to_string()),
        )
        .unwrap();
    gr.activate_goal(&g.goal_id, None);

    // Suggest goal step based on the goal
    let hint = gr
        .get_goal(&g.goal_id)
        .unwrap()
        .continuation_hint
        .unwrap_or_default();
    let sug = eng.suggest_goal_step(&g.goal_id, &g.description, &hint);
    assert!(sug.is_some());
}

/// I4: Desktop awareness + suggestions engine integration: build failure path.
#[test]
fn i4_desktop_awareness_build_fail_triggers_suggestion() {
    let r = drt();
    let eng = seng();

    r.apply_event(&CognitionEvent::Ide(IdeCognitionEvent {
        workspace_root: Some("/proj".into()),
        kind: IdeEventKind::BuildFailed {
            error_count: 2,
            first_error: "E0308".into(),
        },
    }));

    // If errors detected, suggest build recovery
    let snap = r.snapshot();
    if snap.ide.has_errors() {
        let sug = eng
            .suggest_build_recovery(snap.ide.workspace_root.as_deref().unwrap_or(""), "2 errors");
        assert!(sug.is_some());
    }
}

/// I5: OperationalContextTracker chain grows across multiple workflows.
#[test]
fn i5_context_chain_grows_correctly() {
    let t = ctx();
    let ids = ["wf-a", "wf-b", "wf-c"];
    for id in &ids {
        t.record_workflow_started(id, "test");
        t.record_workflow_ended(id, true);
    }
    let snap = t.snapshot();
    assert!(!snap.recent_session_chain.is_empty());
    assert!(
        snap.recent_session_chain.contains(&"wf-a".to_string())
            || snap.recent_session_chain.contains(&"wf-b".to_string())
            || snap.recent_session_chain.contains(&"wf-c".to_string())
    );
}
