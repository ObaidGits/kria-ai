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
        quarantine_registry: Arc::new(
            kria_core::tools::quarantine::QuarantineRegistry::open_in_memory().unwrap(),
        ),
        turn_admission: Arc::new(kria_core::agent::TurnAdmission::new()),
        agent_loop: None,
        device_registry: None,
        notifier: None,
        session_store: None,
        memory_system: Some(ms.clone()),
        caller: test_caller(),
        remote_desktop: None,
        remote_desktop_backend: None,
    });

    (kria_server::build_router(state), ms)
}

/// Same as [`build_test_app_with_memory`] but lets the caller override the
/// `ServerState::caller` identity — used by the F1.5.3 negative-auth matrix to
/// prove a `LocalDesktop`-origin caller (never reachable in the real server
/// binary, which always constructs `AuthenticatedRemote` at its adapter
/// boundary — see `main.rs`) is exempt from the remote capability lattice,
/// while the real `AuthenticatedRemote` caller is denied.
async fn build_test_app_with_memory_and_caller(
    caller: kria_core::memory::model::CallerContext,
) -> (Router, Arc<kria_core::memory::api::MemorySystem>) {
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
    ms.remember(WriteCandidate::global(
        "the kria server exposes live memory routes over http",
    ))
    .expect("remember");
    ms.flush().await.expect("flush enrichment");

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
        memory_system: Some(ms.clone()),
        caller,
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

// ── F1.5.3 negative-auth matrix (MGR-003 AC2/AC3/AC6) ──────────────────────
//
// The real server binary always constructs an `AuthenticatedRemote` caller at
// its adapter boundary (see `main.rs`), so every one of its callers — even a
// caller reaching it over loopback — is subject to the remote capability
// lattice. These tests prove: (a) an `AuthenticatedRemote` caller is denied
// every mutation capability beyond `Observe`-class writes with a non-revealing
// `403` envelope, (b) full backup/restore is denied for every server caller
// regardless of origin (design §8.3: local-desktop-only), and (c) the
// Observe-class writes (`remember`, `reflect`, `consolidate`) remain live.

#[tokio::test]
async fn remote_caller_is_denied_every_destructive_mutation_capability() {
    let (app, _ms) = build_test_app_with_memory().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    // Each case: (method, path, json body) for a capability an
    // AuthenticatedRemote caller may not issue without an explicit grant.
    let denied_forget = client
        .post(format!("{base}/memory/forget"))
        .json(&serde_json::json!({ "kind": "memory", "value": uuid::Uuid::new_v4().to_string() }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied_forget.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = denied_forget.json().await.unwrap();
    // Non-revealing shape (MGR-003 AC3) now includes the opaque correlation
    // id every deny path shares (F1.6.4 normalization).
    assert_eq!(body["error"], "unsupported_capability");
    assert_eq!(body.as_object().unwrap().len(), 2);
    assert!(body["correlation_id"].is_string());

    let denied_delete = client
        .post(format!("{base}/memory/delete"))
        .json(&serde_json::json!({ "kind": "memory", "value": uuid::Uuid::new_v4().to_string() }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied_delete.status(), StatusCode::FORBIDDEN);
    let delete_body: serde_json::Value = denied_delete.json().await.unwrap();
    assert_eq!(
        delete_body["error"], "unsupported_capability",
        "delete denial carries the same non-revealing shape as forget denial"
    );
    assert_eq!(delete_body.as_object().unwrap().len(), 2);

    let denied_verify = client
        .post(format!("{base}/memory/verify"))
        .json(&serde_json::json!({ "id": uuid::Uuid::new_v4().to_string() }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied_verify.status(), StatusCode::FORBIDDEN);

    // Full-authority backup/restore is denied for every server caller —
    // loopback or remote — per the design §8.3 capability matrix.
    let denied_backup = client
        .post(format!("{base}/memory/backup"))
        .json(&serde_json::json!({ "dest": "/tmp/should-never-run.db" }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied_backup.status(), StatusCode::FORBIDDEN);

    let denied_restore = client
        .post(format!("{base}/memory/restore"))
        .json(&serde_json::json!({ "src": "/tmp/should-never-run.db" }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied_restore.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn remote_caller_may_still_issue_observe_class_writes() {
    let (app, _ms) = build_test_app_with_memory().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    // `remember` is Observe-class — the one mutation an AuthenticatedRemote
    // caller may issue by default (MGR-003 AC2).
    let res = client
        .post(format!("{base}/memory/remember"))
        .json(&serde_json::json!({ "text": "an observe-class remote write stays allowed" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let res = client
        .post(format!("{base}/memory/reflect"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn local_desktop_origin_caller_is_not_subject_to_the_remote_lattice() {
    // Never reachable in the real server binary (it always constructs
    // `AuthenticatedRemote` at its adapter boundary), but proves the
    // capability lattice keys strictly on `CallerOrigin`, not on the route.
    let local_caller = kria_core::memory::model::CallerContext::local_desktop(
        "test-local",
        kria_core::memory::model::PolicyPartition::new("user", "chat", 0).unwrap(),
    )
    .unwrap();
    let (app, _ms) = build_test_app_with_memory_and_caller(local_caller).await;
    let base = spawn_app(app).await;
    let client = Client::new();

    let res = client
        .post(format!("{base}/memory/forget"))
        .json(&serde_json::json!({ "kind": "memory", "value": uuid::Uuid::new_v4().to_string() }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "a LocalDesktop-origin caller is exempt from the remote capability lattice"
    );

    // Full backup/restore stays denied even for a LocalDesktop-origin caller —
    // the server host itself never supports it, regardless of origin.
    let res = client
        .post(format!("{base}/memory/backup"))
        .json(&serde_json::json!({ "dest": "/tmp/should-never-run.db" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

// ── MGR-003 / F1.6.2 — remote bearer-token auth middleware ─────────

/// Build the app router in REMOTE mode (`remote_enabled = true`,
/// `enable_auth = true`, a fixed `jwt_secret`) so `auth_middleware` is
/// actually layered onto the router (see `build_router`). Returns the app
/// plus the secret bytes so tests can mint their own tokens with
/// `kria_server::auth::issue_token`.
async fn build_test_app_remote_auth() -> (Router, Vec<u8>) {
    use kria_core::config::KriaConfig;

    let mut config = KriaConfig::default();
    config.server.remote_enabled = true;
    config.server.enable_auth = true;
    config.server.jwt_secret = "integration-test-jwt-secret".to_string();
    let secret = config.server.jwt_secret.clone().into_bytes();

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

    (kria_server::build_router(state), secret)
}

#[tokio::test]
async fn remote_mode_rejects_missing_authorization_header() {
    let (app, _secret) = build_test_app_remote_auth().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Non-revealing deny body (MGR-003 AC3) — fixed generic shape, no
    // detail beyond the opaque correlation id every deny path now includes
    // (F1.6.4 normalization — matches origin/rate-limit/capability denies).
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["error"], "unauthorized");
    assert_eq!(body.as_object().unwrap().len(), 2);
    assert!(body["correlation_id"].is_string());
}

#[tokio::test]
async fn remote_mode_rejects_malformed_authorization_header() {
    let (app, _secret) = build_test_app_remote_auth().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    for bad_header in ["not-bearer-scheme", "Bearer ", "Bearer garbage.not.a.real.token"] {
        let res = client
            .get(format!("{base}/api/health"))
            .header("Authorization", bad_header)
            .send()
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "header: {bad_header:?}"
        );
    }
}

#[tokio::test]
async fn remote_mode_accepts_valid_signed_token() {
    let (app, secret) = build_test_app_remote_auth().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    let token = kria_server::auth::issue_token(&secret, "remote-actor-1", "remote-device-1", 60);

    let res = client
        .get(format!("{base}/api/health"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn remote_mode_rejects_expired_token() {
    let (app, secret) = build_test_app_remote_auth().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    // ttl=0 is clamped to a minimum of 1s by `issue_token`; instead, mint a
    // valid token then wait it out is too slow for a unit test — issue with a
    // 1s TTL and sleep past it.
    let token = kria_server::auth::issue_token(&secret, "remote-actor-1", "remote-device-1", 1);
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let res = client
        .get(format!("{base}/api/health"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn remote_mode_rejects_replayed_token() {
    let (app, secret) = build_test_app_remote_auth().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    let token = kria_server::auth::issue_token(&secret, "remote-actor-1", "remote-device-1", 60);

    let first = client
        .get(format!("{base}/api/health"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    // Same token (same nonce) reused — must be rejected as a replay.
    let second = client
        .get(format!("{base}/api/health"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn remote_mode_rejects_signature_tampered_with_wrong_secret() {
    let (app, _secret) = build_test_app_remote_auth().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    // Token signed with a DIFFERENT secret than the server is configured
    // with — signature verification must fail.
    let token = kria_server::auth::issue_token(
        b"a-completely-wrong-secret",
        "remote-actor-1",
        "remote-device-1",
        60,
    );

    let res = client
        .get(format!("{base}/api/health"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn default_loopback_mode_is_unaffected_by_auth_middleware() {
    // Default `build_test_app()` has `remote_enabled = false` — the auth
    // middleware must not even be layered on, so requests with NO
    // Authorization header at all still succeed (MGR-003 AC1/AC2 only
    // impose token auth in the non-loopback/remote case).
    let base = spawn_test_server().await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

/// Same as [`build_test_app_remote_auth`] but with a live `MemorySystem`
/// wired up, so a validated remote token's per-request `CallerContext` can be
/// proven to actually gate `/memory/*` writes (not just `/api/health`).
async fn build_test_app_remote_auth_with_memory(
) -> (Router, Vec<u8>, Arc<kria_core::memory::api::MemorySystem>) {
    use kria_core::config::KriaConfig;
    use kria_core::memory::api::{MemoryConfig, MemorySystem};
    use kria_core::memory::embedding::OnnxEmbedder;
    use kria_core::memory::types::WriteCandidate;

    let mut config = KriaConfig::default();
    config.server.remote_enabled = true;
    config.server.enable_auth = true;
    config.server.jwt_secret = "integration-test-jwt-secret-2".to_string();
    let secret = config.server.jwt_secret.clone().into_bytes();

    let fleet = Arc::new(
        kria_server::inventory::FleetRuntime::initialize(&config)
            .await
            .expect("fleet runtime init"),
    );

    let embedder = Arc::new(OnnxEmbedder::new_minilm().expect("headless embedder"));
    let ms = MemorySystem::open_for_test(MemoryConfig::default(), embedder).expect("memory system");
    ms.remember(WriteCandidate::global(
        "the kria server exposes live memory routes over http",
    ))
    .expect("remember");
    ms.flush().await.expect("flush enrichment");

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
        memory_system: Some(ms.clone()),
        caller: test_caller(),
        remote_desktop: None,
        remote_desktop_backend: None,
    });

    (kria_server::build_router(state), secret, ms)
}

#[tokio::test]
async fn valid_remote_token_yields_authenticated_remote_caller_context_for_memory_routes() {
    let (app, secret, _ms) = build_test_app_remote_auth_with_memory().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    // Each request needs its OWN token — the replay cache would otherwise
    // reject a second request reusing the same nonce (working as intended).
    let remember_token = kria_server::auth::issue_token(&secret, "phone-user-9", "phone-device-9", 60);
    let forget_token = kria_server::auth::issue_token(&secret, "phone-user-9", "phone-device-9", 60);

    // The per-request CallerContext the middleware built from the verified
    // token is `AuthenticatedRemote` — so Observe-class writes stay allowed…
    let remember = client
        .post(format!("{base}/memory/remember"))
        .header("Authorization", format!("Bearer {remember_token}"))
        .json(&serde_json::json!({ "text": "written by a real per-request remote identity" }))
        .send()
        .await
        .unwrap();
    assert_eq!(remember.status(), StatusCode::OK);

    // …while a mutation kind outside the default remote grant is still
    // denied — proving the token-derived identity feeds the SAME capability
    // lattice as the static server caller, not a bypass.
    let forget = client
        .post(format!("{base}/memory/forget"))
        .header("Authorization", format!("Bearer {forget_token}"))
        .json(&serde_json::json!({ "kind": "memory", "value": uuid::Uuid::new_v4().to_string() }))
        .send()
        .await
        .unwrap();
    assert_eq!(forget.status(), StatusCode::FORBIDDEN);

    // No token at all: denied before ever reaching the capability lattice.
    let no_token = client
        .post(format!("{base}/memory/remember"))
        .json(&serde_json::json!({ "text": "should never be stored" }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_token.status(), StatusCode::UNAUTHORIZED);
}

// ── F1.6.5 — SSE change stream denied under remote exposure (MGR-004) ──
//
// `/memory/events` broadcasts every namespace's/scope's changes with no
// per-subscriber policy filtering (see `memory_routes::events_sse` doc
// comment). Once this deployment is remotely exposed, that is a
// cross-namespace leak regardless of which capability grants the
// authenticated caller otherwise holds — so the route is denied outright in
// remote mode and stays available in the safe default loopback mode.

#[tokio::test]
async fn sse_events_route_is_denied_once_remote_exposure_is_configured() {
    let (app, secret, _ms) = build_test_app_remote_auth_with_memory().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    let token = kria_server::auth::issue_token(&secret, "remote-actor-sse", "remote-device-sse", 60);
    let res = client
        .get(format!("{base}/memory/events"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "the SSE change stream must be denied once remote_enabled = true, \
         even for a fully authenticated caller, since the broadcast is \
         unfiltered across namespaces"
    );
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["error"], "unsupported_capability");
    assert_eq!(body.as_object().unwrap().len(), 2);
}

#[tokio::test]
async fn sse_events_route_remains_available_in_default_loopback_mode() {
    // Default `build_test_app_with_memory()` has `remote_enabled = false` —
    // the SSE stream must stay reachable (mounted + live, not 404/403) for
    // the safe default local-only deployment.
    let (app, _ms) = build_test_app_with_memory().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/memory/events"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "the SSE change stream stays available in default loopback mode"
    );
}

// ── MGR-003 / F1.6.3 — origin/transport/limits/audit-correlation ───

/// Same as [`build_test_app_remote_auth`] but lets the caller further
/// customize `config.server` (allowed_origins, body/rate/concurrency/timeout
/// limits) before the router is built.
async fn build_test_app_remote_auth_with(
    configure: impl FnOnce(&mut kria_core::config::ServerConfig),
) -> (Router, Vec<u8>) {
    use kria_core::config::KriaConfig;

    let mut config = KriaConfig::default();
    config.server.remote_enabled = true;
    config.server.enable_auth = true;
    config.server.jwt_secret = "f1-6-3-integration-secret".to_string();
    configure(&mut config.server);
    let secret = config.server.jwt_secret.clone().into_bytes();

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

    (kria_server::build_router(state), secret)
}

#[tokio::test]
async fn remote_mode_denies_request_with_disallowed_origin_header() {
    let (app, secret) = build_test_app_remote_auth_with(|s| {
        s.allowed_origins = vec!["https://allowed.example.com".to_string()];
    })
    .await;
    let base = spawn_app(app).await;
    let client = Client::new();
    let token = kria_server::auth::issue_token(&secret, "actor-1", "device-1", 60);

    let res = client
        .get(format!("{base}/api/health"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Origin", "https://evil.example.com")
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["error"], "origin_not_allowed");
    // Non-revealing (MGR-003 AC3): only an opaque correlation id alongside
    // the generic error code, no protected detail.
    assert!(body["correlation_id"].is_string());
}

#[tokio::test]
async fn remote_mode_allows_request_with_allowed_origin_header() {
    let (app, secret) = build_test_app_remote_auth_with(|s| {
        s.allowed_origins = vec!["https://allowed.example.com".to_string()];
    })
    .await;
    let base = spawn_app(app).await;
    let client = Client::new();
    let token = kria_server::auth::issue_token(&secret, "actor-1", "device-1", 60);

    let res = client
        .get(format!("{base}/api/health"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Origin", "https://allowed.example.com")
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn remote_mode_with_empty_allowlist_denies_any_browser_origin_fail_closed() {
    // Default allowed_origins is empty — MGR-003 AC2 fail-closed reading:
    // this must deny every Origin-bearing request, not allow every one.
    let (app, secret) = build_test_app_remote_auth_with(|_s| {}).await;
    let base = spawn_app(app).await;
    let client = Client::new();
    let token = kria_server::auth::issue_token(&secret, "actor-1", "device-1", 60);

    let res = client
        .get(format!("{base}/api/health"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Origin", "https://anything.example.com")
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn non_browser_request_with_no_origin_header_is_unaffected_by_allowlist() {
    // A non-browser client (no Origin header at all) must not be denied by
    // the origin check even with an empty/mismatched allowlist — origin
    // enforcement is a browser concept; identity/auth gates non-browser
    // callers instead.
    let (app, secret) = build_test_app_remote_auth_with(|s| {
        s.allowed_origins = vec!["https://allowed.example.com".to_string()];
    })
    .await;
    let base = spawn_app(app).await;
    let client = Client::new();
    let token = kria_server::auth::issue_token(&secret, "actor-1", "device-1", 60);

    let res = client
        .get(format!("{base}/api/health"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn oversized_request_body_is_rejected_with_413() {
    let (app, secret) = build_test_app_remote_auth_with(|s| {
        s.max_body_bytes = 64;
    })
    .await;
    let base = spawn_app(app).await;
    let client = Client::new();
    let token = kria_server::auth::issue_token(&secret, "actor-1", "device-1", 60);

    let big_text = "x".repeat(1024);
    let res = client
        .post(format!("{base}/memory/remember"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "text": big_text }))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn oversized_request_body_rejection_uses_the_normalized_json_envelope() {
    // F1.6.4 — `RequestBodyLimitLayer`'s raw built-in rejection is a plain
    // `text/plain` body (`"length limit exceeded"`), not the JSON envelope
    // convention every other deny path in this crate uses.
    // `deny::normalize_builtin_denies` rewrites it before it leaves the
    // process; this proves that rewrite actually happens end-to-end, not
    // just at the unit level.
    let (app, secret) = build_test_app_remote_auth_with(|s| {
        s.max_body_bytes = 64;
    })
    .await;
    let base = spawn_app(app).await;
    let client = Client::new();
    let token = kria_server::auth::issue_token(&secret, "actor-1", "device-1", 60);

    let big_text = "x".repeat(1024);
    let res = client
        .post(format!("{base}/memory/remember"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "text": big_text }))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        res.headers().get("content-type").unwrap(),
        "application/json"
    );
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["error"], "payload_too_large");
    assert_eq!(body.as_object().unwrap().len(), 2);
    assert!(body["correlation_id"].is_string());
}

#[tokio::test]
async fn timeout_rejection_uses_the_normalized_json_envelope() {
    // F1.6.4 — `TimeoutLayer`'s raw built-in rejection is an empty body with
    // no content-type; `deny::normalize_builtin_denies` rewrites it into the
    // same JSON envelope convention every other deny path uses.
    //
    // A real `/memory/*` route cannot deterministically exercise this: SSE
    // handlers return their `Response` object (headers + stream) almost
    // immediately, so `TimeoutLayer` — which wraps time-to-response, not
    // stream duration — would never fire. This builds the exact same layer
    // stack `build_router` uses (correlation → normalize → timeout) around
    // a handler that deliberately outlives the configured deadline, so the
    // `TimeoutLayer` rejection path is actually exercised.
    use axum::{routing::get, Router};
    use std::time::Duration;
    use tower_http::timeout::TimeoutLayer;

    let app: Router = Router::new()
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                "too slow"
            }),
        )
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_millis(20),
        ))
        .layer(axum::middleware::from_fn(
            kria_server::deny::normalize_builtin_denies,
        ))
        .layer(axum::middleware::from_fn(
            kria_server::correlation::correlation_middleware,
        ));

    let base = spawn_app(app).await;
    let client = Client::new();
    let res = client.get(format!("{base}/slow")).send().await.unwrap();

    assert_eq!(res.status(), StatusCode::REQUEST_TIMEOUT);
    assert_eq!(
        res.headers().get("content-type").unwrap(),
        "application/json"
    );
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["error"], "request_timeout");
    assert_eq!(body.as_object().unwrap().len(), 2);
    assert!(body["correlation_id"].is_string());
}

#[tokio::test]
async fn small_request_body_is_unaffected_by_a_small_configured_limit() {
    let (app, secret) = build_test_app_remote_auth_with(|s| {
        s.max_body_bytes = 64 * 1024;
    })
    .await;
    let base = spawn_app(app).await;
    let client = Client::new();
    let token = kria_server::auth::issue_token(&secret, "actor-1", "device-1", 60);

    let res = client
        .get(format!("{base}/api/health"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn caller_is_denied_after_exceeding_the_remote_rate_limit() {
    let (app, secret) = build_test_app_remote_auth_with(|s| {
        s.remote_rate_limit_per_minute = 3;
        // Keep origin/body checks out of the way so only rate-limit denies.
        s.allowed_origins = vec![];
    })
    .await;
    let base = spawn_app(app).await;
    let client = Client::new();

    // Fixed actor/device identity so every request shares one rate-limit
    // bucket; a fresh token per request avoids tripping replay protection.
    let mut last_status = StatusCode::OK;
    for _ in 0..5 {
        let token = kria_server::auth::issue_token(&secret, "rate-actor", "rate-device", 60);
        let res = client
            .get(format!("{base}/api/health"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .unwrap();
        last_status = res.status();
    }

    assert_eq!(last_status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn requests_under_the_rate_limit_all_succeed() {
    let (app, secret) = build_test_app_remote_auth_with(|s| {
        s.remote_rate_limit_per_minute = 10;
    })
    .await;
    let base = spawn_app(app).await;
    let client = Client::new();

    for _ in 0..3 {
        let token = kria_server::auth::issue_token(&secret, "under-limit-actor", "device", 60);
        let res = client
            .get(format!("{base}/api/health"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn concurrency_limit_config_is_applied_and_does_not_block_normal_load() {
    // A focused correctness check (not a load test): a router built with a
    // small configured concurrency limit still serves a handful of
    // sequential/concurrent requests successfully — proving the layer is
    // wired without needing to actually saturate the semaphore.
    let (app, secret) = build_test_app_remote_auth_with(|s| {
        s.max_concurrent_requests = 4;
    })
    .await;
    let base = spawn_app(app).await;
    let client = Client::new();

    let mut handles = Vec::new();
    for _ in 0..8 {
        let base = base.clone();
        let client = client.clone();
        let token = kria_server::auth::issue_token(&secret, "concurrency-actor", "device", 60);
        handles.push(tokio::spawn(async move {
            client
                .get(format!("{base}/api/health"))
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await
                .unwrap()
                .status()
        }));
    }
    for h in handles {
        assert_eq!(h.await.unwrap(), StatusCode::OK);
    }
}

#[tokio::test]
async fn every_response_carries_a_correlation_id_header_including_denies() {
    let (app, secret) = build_test_app_remote_auth_with(|_s| {}).await;
    let base = spawn_app(app).await;
    let client = Client::new();
    let token = kria_server::auth::issue_token(&secret, "actor-1", "device-1", 60);

    // Success path.
    let ok = client
        .get(format!("{base}/api/health"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    assert!(ok.headers().contains_key("x-correlation-id"));

    // Deny path (auth failure — no token at all).
    let denied = client.get(format!("{base}/api/health")).send().await.unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    assert!(denied.headers().contains_key("x-correlation-id"));
}

#[tokio::test]
async fn default_loopback_mode_is_still_unaffected_by_origin_and_rate_limit_layers() {
    // `build_test_app()` uses defaults: remote_enabled = false. Even with a
    // browser-shaped Origin header and no token, loopback mode must not
    // apply the remote-only origin/rate-limit layers at all.
    let base = spawn_test_server().await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/health"))
        .header("Origin", "https://anything.example.com")
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    // Correlation ID and universal limits (body/timeout/concurrency) still
    // apply in loopback mode.
    assert!(res.headers().contains_key("x-correlation-id"));
}

// ── F1.6.6 — no route registration bypasses the equivalent-strength
// authentication/authorization boundary (MGR-003 AC2/AC6) ──────────────────
//
// The gateway (mobile pairing/device-management, agent WS, remote desktop,
// static PWA) is mounted alongside the bearer-token-guarded `api` subtree,
// but is NOT wrapped by `auth::auth_middleware` — it has its own device-token
// boundary instead (`DeviceRegistry`-backed). These tests build a REMOTE-mode
// app WITH a live `PhoneGatewayState` (`device_registry` + `remote_desktop`
// wired) to prove every mounted route class is denied or is an intentionally/
// adequately exempt public bootstrap/static path, and that the F1.6.6 fix
// (origin/rate-limit now covering `api ∪ gateway`, not just `api`) actually
// applies to gateway routes end-to-end.

/// Build a REMOTE-mode app with a live `DeviceRegistry` + `RemoteDesktopManager`
/// so the gateway's own device-token boundary is exercised for real (not just
/// gated-because-registry-is-None).
async fn build_test_app_remote_with_gateway_auth() -> (
    Router,
    Vec<u8>,
    std::sync::Arc<kria_core::mobile::DeviceRegistry>,
    tempfile::TempDir,
) {
    use kria_core::config::KriaConfig;

    std::env::set_var("KRIA_VAULT_PASSPHRASE", "f1-6-6-gateway-test-pass-000000");
    let vault_dir = tempfile::tempdir().unwrap();
    let vault = std::sync::Arc::new(
        kria_core::auth::SecretsVault::open(vault_dir.path().join("vault.enc"), vault_dir.path())
            .unwrap(),
    );
    let registry = std::sync::Arc::new(
        kria_core::mobile::DeviceRegistry::open(vault_dir.path().join("devices.db"), &vault)
            .unwrap(),
    );

    let mut config = KriaConfig::default();
    config.server.remote_enabled = true;
    config.server.enable_auth = true;
    config.server.jwt_secret = "f1-6-6-gateway-test-secret".to_string();
    config.mobile.enabled = true;
    config.mobile.require_device_auth = true;
    let secret = config.server.jwt_secret.clone().into_bytes();

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
        device_registry: Some(registry.clone()),
        notifier: None,
        session_store: None,
        memory_system: None,
        caller: test_caller(),
        remote_desktop: None,
        remote_desktop_backend: None,
    });

    (kria_server::build_router(state), secret, registry, vault_dir)
}

#[tokio::test]
async fn mobile_device_management_routes_require_a_device_token_in_remote_mode() {
    // F1.6.6 fix: `list_devices`/`revoke_device` previously had NO auth check
    // at all — reachable by any caller that reached the gateway, even though
    // the sibling `remote_desktop_routes` already gated its own analogous
    // endpoints on exactly this device-token check.
    let (app, _secret, _registry, _vault_dir) = build_test_app_remote_with_gateway_auth().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/mobile/devices"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "listing paired devices with no token must be denied"
    );

    let res = client
        .post(format!("{base}/api/mobile/devices/some-device-id/revoke"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "revoking a device with no token must be denied"
    );
}

#[tokio::test]
async fn mobile_device_management_routes_accept_a_valid_device_token() {
    let (app, _secret, registry, _vault_dir) = build_test_app_remote_with_gateway_auth().await;
    // Pair one real device so we have a valid device token to present.
    let challenge = registry.begin_pairing("test-host:8787");
    let (_info, device_token) = registry
        .complete_pairing(&challenge.code, "integration-test-phone")
        .unwrap();

    let base = spawn_app(app).await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/mobile/devices"))
        .header("Authorization", format!("Bearer {device_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["devices"].is_array());
}

#[tokio::test]
async fn pairing_bootstrap_routes_remain_exempt_from_device_token_auth() {
    // `/pair`, `pair/begin`, `pair/complete` must stay reachable with NO
    // device token — they are the bootstrap path a not-yet-paired phone must
    // reach, and `pair/complete` is itself the credential-issuing operation.
    let (app, _secret, _registry, _vault_dir) = build_test_app_remote_with_gateway_auth().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    let pair_page = client.get(format!("{base}/pair")).send().await.unwrap();
    assert_eq!(pair_page.status(), StatusCode::OK);

    let begin = client
        .post(format!("{base}/api/mobile/pair/begin"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(begin.status(), StatusCode::OK);
}

#[tokio::test]
async fn agent_ws_requires_a_device_token_when_mobile_auth_is_required() {
    // `ws.rs` already gated this before F1.6.6 — included here as part of
    // the complete route-class enumeration this task requires, proving the
    // existing gate is still live end-to-end.
    let (app, _secret, _registry, _vault_dir) = build_test_app_remote_with_gateway_auth().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/ws"))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn remote_desktop_control_plane_requires_a_device_token_when_mobile_auth_is_required() {
    // `remote_desktop_routes::authorize` already gated this before F1.6.6 —
    // included for the same complete-enumeration reason as the `/ws` case.
    let (app, _secret, _registry, _vault_dir) = build_test_app_remote_with_gateway_auth().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/remote-desktop/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn static_pwa_fallback_remains_public_by_design() {
    // The static PWA shell (`fallback_service`) is intentionally public — it
    // serves no protected data, only the built frontend shell assets. This
    // documents that exemption rather than leaving it silently unaudited.
    let (app, _secret, _registry, _vault_dir) = build_test_app_remote_with_gateway_auth().await;
    let base = spawn_app(app).await;
    let client = Client::new();

    // No `ui/dist` build exists in the test environment, so `ServeDir`
    // 404s — the key assertion is that this is NOT 401/403 (i.e. no auth
    // boundary applies to it), regardless of whether the asset itself
    // exists on disk.
    let res = client
        .get(format!("{base}/some-static-asset.js"))
        .send()
        .await
        .unwrap();
    assert_ne!(res.status(), StatusCode::UNAUTHORIZED);
    assert_ne!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn gateway_routes_are_now_covered_by_origin_allowlisting_in_remote_mode() {
    // F1.6.6 fix: before this change, `origin_middleware` was layered only
    // onto `api` BEFORE the `gateway` merge, so a gateway route (here,
    // `/api/mobile/devices`, reached with a valid device token so ONLY the
    // origin check is under test) was completely unprotected by the origin
    // allowlist even in remote mode.
    use kria_core::config::KriaConfig;

    std::env::set_var(
        "KRIA_VAULT_PASSPHRASE",
        "f1-6-6-gateway-origin-test-pass-000",
    );
    let vault_dir = tempfile::tempdir().unwrap();
    let vault = std::sync::Arc::new(
        kria_core::auth::SecretsVault::open(vault_dir.path().join("vault.enc"), vault_dir.path())
            .unwrap(),
    );
    let registry = std::sync::Arc::new(
        kria_core::mobile::DeviceRegistry::open(vault_dir.path().join("devices.db"), &vault)
            .unwrap(),
    );
    let challenge = registry.begin_pairing("test-host:8787");
    let (_info, device_token) = registry.complete_pairing(&challenge.code, "phone").unwrap();

    let mut config = KriaConfig::default();
    config.server.remote_enabled = true;
    config.server.enable_auth = true;
    config.server.jwt_secret = "f1-6-6-gateway-origin-secret".to_string();
    config.server.allowed_origins = vec!["https://allowed.example.com".to_string()];
    config.mobile.enabled = true;
    config.mobile.require_device_auth = true;

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
        device_registry: Some(registry),
        notifier: None,
        session_store: None,
        memory_system: None,
        caller: test_caller(),
        remote_desktop: None,
        remote_desktop_backend: None,
    });
    let app = kria_server::build_router(state);
    let base = spawn_app(app).await;
    let client = Client::new();

    let res = client
        .get(format!("{base}/api/mobile/devices"))
        .header("Authorization", format!("Bearer {device_token}"))
        .header("Origin", "https://evil.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "a gateway route must now be denied by the origin allowlist just like an api route"
    );
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["error"], "origin_not_allowed");
}
