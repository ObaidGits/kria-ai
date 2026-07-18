//! Integration tests for the KRIA server REST API.
//!
//! These tests spin up an ephemeral Axum server on a random port,
//! exercise every API route, and tear down cleanly — making them
//! fully idempotent with no database side-effects.

use axum::Router;
use reqwest::{Client, StatusCode};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

// ── Test helpers ────────────────────────────────────────────────────

/// Build the full application router with a default config.
async fn build_test_app() -> Router {
    use kria_core::config::KriaConfig;

    let config = KriaConfig::default();
    let fleet = Arc::new(
        kria_server::inventory::FleetRuntime::initialize(&config)
            .await
            .expect("fleet runtime init"),
    );

    let state = Arc::new(kria_server::ServerState {
        config,
        fleet,
        executive_sender: None,
        turn_admission: Arc::new(kria_core::agent::TurnAdmission::new()),
        agent_loop: None,
        device_registry: None,
        notifier: None,
        session_store: None,
        memory_system: None,
        remote_desktop: None,
        remote_desktop_backend: None,
    });

    kria_server::build_router(state)
}

/// Build the app router wired to a **live** `MemorySystem` (M4) so the memory
/// routes can be exercised end-to-end (real 200s, real shapes), not just gated.
/// Uses the headless embedder (degrades to the FTS/keyword floor with no ONNX
/// model), so no model download is needed.
async fn build_test_app_with_memory() -> (Router, Arc<kria_core::memory::api::MemorySystem>) {
    use kria_core::config::KriaConfig;
    use kria_core::memory::api::{MemoryConfig, MemorySystem};
    use kria_core::memory::embedding::OnnxEmbedder;
    use kria_core::memory::types::WriteCandidate;

    let config = KriaConfig::default();
    let fleet = Arc::new(
        kria_server::inventory::FleetRuntime::initialize(&config)
            .await
            .expect("fleet runtime init"),
    );

    let embedder = Arc::new(OnnxEmbedder::new_minilm().expect("headless embedder"));
    let ms = MemorySystem::open_for_test(MemoryConfig::default(), embedder).expect("memory system");
    // Seed one durable memory so the read routes return real data.
    ms.remember(WriteCandidate::global(
        "the kria server exposes live memory routes over http",
    ))
    .expect("remember");
    ms.flush().await.expect("flush enrichment");

    let state = Arc::new(kria_server::ServerState {
        config,
        fleet,
        executive_sender: None,
        turn_admission: Arc::new(kria_core::agent::TurnAdmission::new()),
        agent_loop: None,
        device_registry: None,
        notifier: None,
        session_store: None,
        memory_system: Some(ms.clone()),
        remote_desktop: None,
        remote_desktop_backend: None,
    });

    (kria_server::build_router(state), ms)
}

/// Start the test server on a random OS-assigned port and return its base URL.
async fn spawn_test_server() -> String {
    let app = build_test_app().await;
    spawn_app(app).await
}

async fn spawn_app(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

// ── Health endpoint ─────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_ok_with_version() {
    let base = spawn_test_server().await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["status"], "healthy");
    assert!(
        body.get("version").is_some(),
        "response must include version"
    );
}

// ── Chat endpoint ───────────────────────────────────────────────────

#[tokio::test]
async fn chat_accepts_valid_message() {
    let base = spawn_test_server().await;
    let client = Client::new();

    let payload = serde_json::json!({
        "message": "Hello, KRIA!",
    });

    let res = client
        .post(format!("{base}/api/chat"))
        .json(&payload)
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);

    let body: serde_json::Value = res.json().await.unwrap();
    // The test harness has no agent runtime (agent_loop: None), so /api/chat now
    // honestly reports the runtime is unavailable instead of echoing a fake
    // reply. A session_id is still auto-generated.
    assert_eq!(body["status"], "unavailable");
    assert!(body.get("session_id").is_some());
}

#[tokio::test]
async fn chat_preserves_explicit_session_id() {
    let base = spawn_test_server().await;
    let client = Client::new();

    let payload = serde_json::json!({
        "message": "test",
        "session_id": "my-session-42",
    });

    let res = client
        .post(format!("{base}/api/chat"))
        .json(&payload)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["session_id"], "my-session-42");
}

#[tokio::test]
async fn chat_rejects_missing_message_field() {
    let base = spawn_test_server().await;
    let client = Client::new();

    // Malformed request — no `message` field
    let payload = serde_json::json!({ "wrong_field": "oops" });

    let res = client
        .post(format!("{base}/api/chat"))
        .json(&payload)
        .send()
        .await
        .unwrap();

    // Axum returns 422 when JSON deserialization fails
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn chat_rejects_non_json_body() {
    let base = spawn_test_server().await;
    let client = Client::new();

    let res = client
        .post(format!("{base}/api/chat"))
        .header("content-type", "text/plain")
        .body("not json")
        .send()
        .await
        .unwrap();

    // Should be a 4xx error — either 415 or 422
    assert!(res.status().is_client_error());
}

// ── Sessions endpoint ───────────────────────────────────────────────

#[tokio::test]
async fn sessions_returns_empty_list() {
    let base = spawn_test_server().await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/sessions"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body.is_array());
    assert_eq!(body.as_array().unwrap().len(), 0);
}

// ── Models endpoint ─────────────────────────────────────────────────

#[tokio::test]
async fn models_returns_models_array() {
    let base = spawn_test_server().await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body: serde_json::Value = res.json().await.unwrap();
    // The key must exist even if models dir is missing
    assert!(body.get("models").is_some());
}

// ── Settings endpoints ──────────────────────────────────────────────

#[tokio::test]
async fn get_settings_returns_full_config() {
    let base = spawn_test_server().await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body: serde_json::Value = res.json().await.unwrap();
    // Should contain top-level config sections
    assert!(
        body.get("llm").is_some(),
        "settings must include llm section"
    );
    assert!(
        body.get("voice").is_some(),
        "settings must include voice section"
    );
    assert!(
        body.get("memory").is_some(),
        "settings must include memory section"
    );
    assert!(
        body.get("safety").is_some(),
        "settings must include safety section"
    );
    assert!(
        body.get("server").is_some(),
        "settings must include server section"
    );
    assert!(body.get("ui").is_some(), "settings must include ui section");
}

#[tokio::test]
async fn update_settings_returns_updated_status() {
    let base = spawn_test_server().await;
    let client = Client::new();

    let payload = serde_json::json!({
        "ui": { "theme": "light" },
    });

    let res = client
        .post(format!("{base}/api/settings"))
        .json(&payload)
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["status"], "updated");
}

// ── Broken API response simulation ──────────────────────────────────

#[tokio::test]
async fn nonexistent_route_returns_404() {
    let base = spawn_test_server().await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/nonexistent"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

// ── CORS layer ──────────────────────────────────────────────────────

#[tokio::test]
async fn cors_allows_any_origin() {
    let base = spawn_test_server().await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/health"))
        .header("Origin", "http://localhost:5173")
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    // CorsLayer::permissive() reflects the origin
    let cors = res.headers().get("access-control-allow-origin");
    assert!(cors.is_some(), "CORS header must be present");
}

// ── Memory routes (P7) ──────────────────────────────────────────────

#[tokio::test]
async fn memory_routes_are_mounted_and_gate_when_unavailable() {
    let base = spawn_test_server().await;
    let client = Client::new();

    // The test ServerState has `memory_system: None`, so the routes exist but
    // return 503 (proving they are mounted + gracefully gated, not 404).
    for path in ["/memory/health", "/memory/metrics", "/memory/goals"] {
        let res = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{path} should be mounted and gated"
        );
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(body.get("error").is_some(), "{path} returns an error body");
    }

    // Search route with a query param is also mounted + gated.
    let res = client
        .get(format!("{base}/memory/search?q=hello"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);

    // UI-1: the live SSE stream endpoint is mounted and gated (503 without a
    // memory system, not 404).
    let res = client
        .get(format!("{base}/memory/events"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "/memory/events (SSE) should be mounted and gated"
    );
}

#[tokio::test]
async fn memory_routes_serve_live_data_with_a_real_memory_system() {
    // M4: exercise the server memory routes against a LIVE MemorySystem (thin
    // adapters over the shared contract), asserting real 200s + shapes.
    let (app, _ms) = build_test_app_with_memory().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    // Health: 200 with the canonical shape + the seeded memory counted.
    let res = client
        .get(format!("{base}/memory/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body.get("api_version").is_some());
    assert!(
        body.get("memory_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            >= 1,
        "the seeded memory is counted"
    );
    // AUD-01: the enrichment-backlog gauge is surfaced over HTTP.
    assert!(
        body.get("pending_enrichment").is_some(),
        "health exposes pending_enrichment"
    );

    // AUD-02: metrics surfaces tool-outcome telemetry over HTTP.
    let mres = client
        .get(format!("{base}/memory/metrics"))
        .send()
        .await
        .unwrap();
    assert_eq!(mres.status(), StatusCode::OK);
    let mbody: serde_json::Value = mres.json().await.unwrap();
    let to = mbody
        .get("tool_outcomes")
        .expect("metrics exposes tool_outcomes");
    assert!(to.get("seen").is_some() && to.get("gated").is_some());

    // Search: 200 with results/count/trace (FTS floor finds the seeded text).
    let res = client
        .get(format!("{base}/memory/search?q=memory+routes"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body.get("results").unwrap().is_array());
    assert!(body.get("trace").unwrap().get("query_class").is_some());

    // Remember: POST returns a decision (write path is live).
    let res = client
        .post(format!("{base}/memory/remember"))
        .json(&serde_json::json!({ "text": "a second durable server-side fact" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res
        .json::<serde_json::Value>()
        .await
        .unwrap()
        .get("decision")
        .is_some());

    // Metrics + report also serve live 200s.
    for path in ["/memory/metrics", "/memory/report", "/memory/library"] {
        let res = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path} serves live data");
    }
}
