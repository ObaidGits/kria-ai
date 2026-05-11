//! Live collision stress harness for GPU lease preemption.
//!
//! This test is intentionally `#[ignore]` because it requires a live local
//! runtime stack (llama-server + ComfyUI + NVIDIA GPU) and is meant to be run
//! manually while monitoring VRAM.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::llm_available;
use kria_core::agent::loop_engine::{
    PromptLabToolSelectionStrategy, StreamEvent, TurnExecutionProfile,
};
use kria_core::agent::turn_gate::{Operation, TurnGate};
use kria_core::agent::AgentLoop;
use kria_core::llm::orchestrator::Orchestrator;
use kria_core::llm::{ChatMessage, ModelRouter};
use kria_core::resource::{GpuLeaseManager, GpuLeaseState};
use kria_core::safety::hitl::HitlGateway;
use kria_core::safety::{AuditLogger, PolicyEngine, RollbackManager};
use kria_core::tools::registry;
use tokio::sync::mpsc;
use uuid::Uuid;

fn build_messages(user_text: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            role: "system".into(),
            content: "You are K.R.I.A.".into(),
            name: None,
            images: None,
        },
        ChatMessage {
            role: "user".into(),
            content: user_text.to_string(),
            name: None,
            images: None,
        },
    ]
}

async fn collect_events(mut rx: mpsc::UnboundedReceiver<StreamEvent>) -> Vec<StreamEvent> {
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }
    events
}

fn resolve_kria_data_dir() -> PathBuf {
    if let Ok(path) = std::env::var("KRIA_DATA_DIR") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".kria")
}

fn text_indicates_cancel_or_abort(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("cancel")
        || lower.contains("aborted")
        || lower.contains("abort")
        || lower.contains("stale")
        || lower.contains("supersed")
}

fn text_indicates_oom(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("out of memory") || lower.contains("oom")
}

fn preview_events(events: &[StreamEvent], max_events: usize) -> Vec<String> {
    events
        .iter()
        .take(max_events)
        .map(|ev| format!("{ev:?}"))
        .collect()
}

async fn wait_for_generate_image_tool_start(
    rx: &mut mpsc::UnboundedReceiver<StreamEvent>,
    sink: &mut Vec<StreamEvent>,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;

    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(250), rx.recv()).await {
            Ok(Some(ev)) => {
                let matched = matches!(
                    &ev,
                    StreamEvent::ToolStart { name, .. } if name == "generate_image"
                );
                sink.push(ev);
                if matched {
                    return true;
                }
            }
            Ok(None) => return false,
            Err(_) => {
                // No event yet; keep waiting until deadline.
            }
        }
    }

    false
}

async fn wait_for_lease_reclaim(
    orchestrator: &Arc<kria_core::image::ImageOrchestrator>,
    timeout: Duration,
) -> GpuLeaseState {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let state = orchestrator.gpu_lease_state();
        if !matches!(state, GpuLeaseState::Held { .. }) {
            return state;
        }

        if tokio::time::Instant::now() >= deadline {
            return state;
        }

        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "live GPU collision harness: requires local llama-server + ComfyUI + NVIDIA GPU"]
async fn live_collision_stress_preempts_image_turn_and_completes_text_turn() {
    if !llm_available() {
        eprintln!("SKIP: live collision stress requires local LLM server at 127.0.0.1:8080");
        return;
    }

    let heavy_prompt =
        "Generate a highly detailed 4k image of a cyberpunk city at night with dense traffic.";
    let preempt_prompt = "Cancel that, just tell me a joke.";

    let gpu_lease_manager =
        GpuLeaseManager::shared(Duration::from_secs(180), Duration::from_secs(20));
    assert!(
        matches!(gpu_lease_manager.state(), GpuLeaseState::Idle),
        "fresh test-scoped lease manager should start idle"
    );

    let turn_gate = TurnGate::new();
    let plan_heavy = turn_gate.plan_turn(heavy_prompt, false);
    assert_eq!(
        plan_heavy.intent.operation,
        Operation::GenerateImage,
        "heavy prompt must compile to GenerateImage"
    );
    let plan_preempt = turn_gate.plan_turn(preempt_prompt, false);
    assert!(
        matches!(
            plan_preempt.intent.operation,
            Operation::Converse | Operation::Cancel
        ),
        "preempt prompt must stay text-path compatible"
    );

    let mut config = kria_core::config::KriaConfig::load(None)
        .expect("failed to load KriaConfig; ensure config/default.toml is readable");
    config.image_generation.enabled = true;
    config.image_generation.cloud_fallback = "off".into();
    config.image_generation.image_mode = "local_only".into();
    config.image_generation.max_concurrent_jobs = 1;

    let model_router = Arc::new(ModelRouter::from_config(&config));

    let tool_registry = registry::build_default_registry();
    let image_orchestrator = kria_core::image::ImageOrchestrator::new(
        config.image_generation.clone(),
        &resolve_kria_data_dir(),
    );

    let image_backend: Arc<dyn kria_core::image::ImageBackend> = image_orchestrator.clone();
    let llm_orch: Arc<tokio::sync::RwLock<Option<Arc<Orchestrator>>>> =
        Arc::new(tokio::sync::RwLock::new(None));

    kria_core::tools::image_generation::register(
        &tool_registry,
        image_backend,
        Arc::new(|_, _| {}),
        llm_orch,
    );

    let tool_registry = Arc::new(tool_registry);
    let policy_engine = Arc::new(PolicyEngine::new());
    let hitl = Arc::new(HitlGateway::new(5));

    let tmp = tempfile::tempdir().expect("create tempdir for audit/rollback");
    let audit_conn = rusqlite::Connection::open(tmp.path().join("collision_audit.db"))
        .expect("open collision audit db");
    let audit_logger = Arc::new(AuditLogger::new(audit_conn));
    let rollback_mgr = Arc::new(RollbackManager::new(tmp.path().join("rollback"), 1, 10));
    let mount_mgr = Arc::new(tokio::sync::RwLock::new(
        kria_core::tools::mount_manager::ToolMountManager::new(),
    ));

    let agent_loop = Arc::new(
        AgentLoop::new(
            model_router,
            tool_registry,
            mount_mgr,
            policy_engine,
            hitl,
            audit_logger,
            rollback_mgr,
        )
        .with_hardware_tier("standard")
        .with_max_tool_rounds(4),
    );

    let session_id = format!("live-collision-{}", Uuid::new_v4());

    let turn1_profile = TurnExecutionProfile::prompt_lab(
        None,
        Some("generate_image".to_string()),
        PromptLabToolSelectionStrategy::DirectLockedTool,
    );

    let (tx1, mut rx1) = mpsc::unbounded_channel();
    let mut turn1_messages = build_messages(heavy_prompt);
    let session_for_turn1 = session_id.clone();
    let agent_for_turn1 = Arc::clone(&agent_loop);
    let turn1_handle = tokio::spawn(async move {
        agent_for_turn1
            .run_with_profile(
                &session_for_turn1,
                &mut turn1_messages,
                tx1,
                Some(turn1_profile),
            )
            .await;
    });

    let mut turn1_events = Vec::new();
    let saw_generate_image_start =
        wait_for_generate_image_tool_start(&mut rx1, &mut turn1_events, Duration::from_secs(45))
            .await;
    assert!(
        saw_generate_image_start,
        "turn1 never reached generate_image ToolStart; ensure local LLM and image backend are available. pre-start events: {:?}",
        preview_events(&turn1_events, 16)
    );

    tokio::time::sleep(Duration::from_millis(800)).await;

    let lease_during_load = image_orchestrator.gpu_lease_state();
    assert!(
        !matches!(lease_during_load, GpuLeaseState::Idle),
        "expected non-idle image lease during heavy load, got: {lease_during_load:?}"
    );

    let (tx2, rx2) = mpsc::unbounded_channel();
    let mut turn2_messages = build_messages(preempt_prompt);
    agent_loop.run(&session_id, &mut turn2_messages, tx2).await;
    let turn2_events = collect_events(rx2).await;

    tokio::time::timeout(Duration::from_secs(240), turn1_handle)
        .await
        .expect("turn1 join timeout")
        .expect("turn1 task panicked");

    turn1_events.extend(collect_events(rx1).await);

    let turn1_turn_id = turn1_events.iter().find_map(|ev| match ev {
        StreamEvent::TurnAccepted {
            session_id: sid,
            turn_id,
        } if sid == &session_id => Some(turn_id.clone()),
        _ => None,
    });

    let turn1_terminal_cancelled = turn1_events.iter().any(|ev| match ev {
        StreamEvent::Error(msg) | StreamEvent::Done(msg) => text_indicates_cancel_or_abort(msg),
        StreamEvent::ToolEnd {
            name,
            result,
            success,
        } if name == "generate_image" && !*success => result["error"]
            .as_str()
            .map(text_indicates_cancel_or_abort)
            .unwrap_or(false),
        _ => false,
    });

    let turn1_superseded = turn1_turn_id
        .as_ref()
        .map(|tid| !agent_loop.is_turn_active(&session_id, tid))
        .unwrap_or(false);

    assert!(
        turn1_terminal_cancelled || turn1_superseded,
        "turn1 was not observed as cancelled/aborted after preemption"
    );

    let turn2_errors = turn2_events
        .iter()
        .filter_map(|ev| match ev {
            StreamEvent::Error(msg) => Some(msg.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(
        turn2_errors.is_empty(),
        "turn2 should complete without errors; got: {turn2_errors:?}"
    );

    let turn2_done_text = turn2_events.iter().find_map(|ev| match ev {
        StreamEvent::Done(text) => Some(text.clone()),
        _ => None,
    });
    assert!(
        turn2_done_text.is_some(),
        "turn2 should complete with Done event"
    );

    let turn2_any_oom = turn2_events.iter().any(|ev| match ev {
        StreamEvent::Error(text) | StreamEvent::Done(text) => text_indicates_oom(text),
        _ => false,
    });
    assert!(
        !turn2_any_oom,
        "turn2 reported OOM path; events: {turn2_events:?}"
    );

    let reclaimed_state =
        wait_for_lease_reclaim(&image_orchestrator, Duration::from_secs(12)).await;
    assert!(
        !matches!(reclaimed_state, GpuLeaseState::Held { .. }),
        "image GPU lease remained held after preemption: {reclaimed_state:?}"
    );
    assert!(
        !matches!(reclaimed_state, GpuLeaseState::Degraded { .. }),
        "image GPU lease entered degraded state after collision: {reclaimed_state:?}"
    );
}
