//! Integration tests for the Intelligence subsystem server wiring.
//!
//! Verifies that:
//! 1. Feature-flagged endpoints return 503 when disabled (default config).
//! 2. When `executive.enabled = true`, /api/chat bypasses the legacy router
//!    and submits to the ExecutiveController queue.
//! 3. Intelligence status endpoint reports current feature flag state.
//! 4. Quarantine endpoints require `skill_compiler.enabled`.

use axum::Router;
use reqwest::{Client, StatusCode};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

// ── Test helpers ────────────────────────────────────────────────────

/// Build app with default config (all intelligence flags OFF).
/// The authenticated-remote caller the server adapter constructs at its
/// boundary (F1.2.4) — shared by the test `ServerState` literals.
fn test_caller() -> kria_core::memory::model::CallerContext {
    kria_core::memory::model::CallerContext::authenticated_remote(
        "test-server",
        "test-server",
        kria_core::memory::model::PolicyPartition::new("user", "chat", 0).unwrap(),
    )
    .unwrap()
}

async fn build_app_default() -> Router {
    let config = kria_core::config::KriaConfig::default();
    let fleet = Arc::new(
        kria_server::inventory::FleetRuntime::initialize(&config)
            .await
            .expect("fleet runtime init"),
    );
    let state = Arc::new(kria_server::ServerState {
        config,
        fleet,
        executive_sender: None,
        quarantine_registry: Arc::new(
            kria_core::tools::quarantine::QuarantineRegistry::open_in_memory().unwrap(),
        ),
        turn_admission: Arc::new(kria_core::agent::TurnAdmission::new()),
        agent_loop: None,
        device_registry: None,
        notifier: None,
        session_store: None,
        memory_system: None,
        caller: test_caller(),
        remote_desktop: None,
        remote_desktop_backend: None,
    });
    kria_server::build_router(state)
}

/// Build app with executive enabled and a real ExecutiveController.
async fn build_app_executive() -> Router {
    use kria_core::agent::executive::ExecutiveController;
    use kria_core::resource::gpu_lease::GpuLeaseManager;
    use kria_core::safety::policy_gate::CapabilityPolicyGate;

    let mut config = kria_core::config::KriaConfig::default();
    config.executive.enabled = true;

    let fleet = Arc::new(
        kria_server::inventory::FleetRuntime::initialize(&config)
            .await
            .expect("fleet runtime init"),
    );

    let gpu = GpuLeaseManager::shared(Duration::from_secs(180), Duration::from_secs(15));
    let policy_gate: Arc<dyn kria_core::safety::policy_gate::PolicyGate> =
        Arc::new(CapabilityPolicyGate::new());

    let executive_config = kria_core::agent::executive::ExecutiveConfig {
        max_background_tasks: config.executive.max_background_tasks,
        preemption_grace_ms: config.executive.preemption_grace_ms,
        ..Default::default()
    };

    let (mut controller, sender) = ExecutiveController::new(executive_config, gpu, policy_gate);

    // Spawn the controller's dispatch loop so it can process tasks.
    tokio::spawn(async move {
        controller.run().await;
    });

    let state = Arc::new(kria_server::ServerState {
        config,
        fleet,
        executive_sender: Some(sender),
        quarantine_registry: Arc::new(
            kria_core::tools::quarantine::QuarantineRegistry::open_in_memory().unwrap(),
        ),
        turn_admission: Arc::new(kria_core::agent::TurnAdmission::new()),
        agent_loop: None,
        device_registry: None,
        notifier: None,
        session_store: None,
        memory_system: None,
        caller: test_caller(),
        remote_desktop: None,
        remote_desktop_backend: None,
    });
    kria_server::build_router(state)
}

async fn spawn_server(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_default_server() -> String {
    spawn_server(build_app_default().await).await
}

async fn spawn_executive_server() -> String {
    spawn_server(build_app_executive().await).await
}

// ── Feature-flag disabled tests ─────────────────────────────────────

#[tokio::test]
async fn executive_snapshot_503_when_disabled() {
    let base = spawn_default_server().await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/executive/snapshot"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn executive_events_503_when_disabled() {
    let base = spawn_default_server().await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/executive/events"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn executive_cancel_503_when_disabled() {
    let base = spawn_default_server().await;
    let client = Client::new();

    let res = client
        .post(format!("{base}/api/executive/tasks/test-123/cancel"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn quarantine_tools_503_when_skill_compiler_disabled() {
    let base = spawn_default_server().await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/quarantine/tools"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn intelligence_status_always_returns_200() {
    let base = spawn_default_server().await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/intelligence/status"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value = res.json().await.unwrap();
    // Intelligence status is always accessible — nested object with enabled field
    assert_eq!(body["executive"]["enabled"], false);
    assert_eq!(body["planner"]["enabled"], false);
    assert_eq!(body["skill_compiler"]["enabled"], false);
}

// ── Executive enabled tests ─────────────────────────────────────────

#[tokio::test]
async fn intelligence_status_shows_executive_enabled() {
    let base = spawn_executive_server().await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/intelligence/status"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["executive"]["enabled"], true);
}

#[tokio::test]
async fn chat_submits_to_executive_when_enabled() {
    let base = spawn_executive_server().await;
    let client = Client::new();

    let payload = serde_json::json!({
        "message": "What is the system status?",
    });

    let res = client
        .post(format!("{base}/api/chat"))
        .json(&payload)
        .send()
        .await
        .unwrap();

    // The chat handler returns 200 OK with a JSON body indicating submission.
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["status"], "submitted");
    assert!(
        body.get("session_id").is_some(),
        "response must include session_id when routed to executive"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .contains("ExecutiveController"),
        "message should mention ExecutiveController routing"
    );
}

#[tokio::test]
async fn executive_snapshot_returns_structure() {
    let base = spawn_executive_server().await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/executive/snapshot"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value = res.json().await.unwrap();
    // Snapshot is controller-owned runtime state, not static config.
    assert!(body.get("active_foreground").is_some());
    assert!(body["active_background"].is_array());
    assert!(body["queued"].is_array());
    assert!(body["total_completed"].is_number());
}

#[tokio::test]
async fn executive_cancel_returns_ok_when_enabled() {
    let base = spawn_executive_server().await;
    let client = Client::new();

    let task_id = uuid::Uuid::new_v4().to_string();
    let res = client
        .post(format!("{base}/api/executive/tasks/{task_id}/cancel"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["status"], "cancel_requested");
    assert_eq!(body["task_id"], task_id);
}

#[tokio::test]
async fn quarantine_approve_503_when_skill_compiler_disabled() {
    // Even with executive enabled, quarantine requires skill_compiler.enabled
    let base = spawn_executive_server().await;
    let client = Client::new();

    let res = client
        .post(format!("{base}/api/quarantine/test-tool/approve"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}
