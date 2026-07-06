//! Regression tests for Issue-1: the local LLM must not be reported "not reachable"
//! while the server is alive-but-busy/loading, and a single transport blip must not
//! flip the reachability signal (debounce).

use kria_core::llm::local::LocalBackend;
use kria_core::llm::LlmBackend;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn backend_for(api_root: &str) -> LocalBackend {
    LocalBackend::new(
        format!("{api_root}/v1"),
        "test-model".to_string(),
        vec!["chat".to_string()],
        4096,
    )
}

#[tokio::test]
async fn health_true_when_server_ready_200() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status":"ok"})))
        .mount(&server)
        .await;

    let backend = backend_for(&server.uri());
    assert!(
        backend.health_check().await,
        "200 /health must be reachable"
    );
}

#[tokio::test]
async fn health_true_when_busy_503_no_slot() {
    // Alive but busy: llama.cpp can return 503 when no slot is available.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(
            ResponseTemplate::new(503)
                .set_body_json(serde_json::json!({"status":"no slot available"})),
        )
        .mount(&server)
        .await;

    let backend = backend_for(&server.uri());
    assert!(
        backend.health_check().await,
        "a busy (503) server is ALIVE and must not be reported unreachable"
    );
}

#[tokio::test]
async fn health_true_when_loading_503() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(
            ResponseTemplate::new(503).set_body_json(serde_json::json!({"status":"loading model"})),
        )
        .mount(&server)
        .await;

    let backend = backend_for(&server.uri());
    assert!(
        backend.health_check().await,
        "a loading (503) server is ALIVE and must not be reported unreachable"
    );
}

#[tokio::test]
async fn health_debounces_transient_unreachable() {
    // Point at a closed port → connection refused (transport failure).
    let backend = backend_for("http://127.0.0.1:1");

    // First failures are debounced (reported reachable) so a single blip during
    // generation never flips the banner.
    assert!(
        backend.health_check().await,
        "1st transport failure is debounced"
    );
    assert!(
        backend.health_check().await,
        "2nd transport failure is debounced"
    );
    // Threshold (3) reached → now reported unreachable.
    assert!(
        !backend.health_check().await,
        "sustained transport failure must eventually report unreachable"
    );
}

#[tokio::test]
async fn health_recovers_after_failure_streak() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let backend = backend_for(&server.uri());
    // A ready probe resets the failure counter.
    assert!(backend.health_check().await);
    assert!(backend.health_check().await);
}
