use kria_core::config::OrchestratorConfig;
use kria_core::llm::orchestrator::server_manager::LlamaServerManager;
use std::time::Duration;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn build_manager(api_root: &str, request_timeout_secs: u64) -> LlamaServerManager {
    let mut config = OrchestratorConfig::default();
    config.health_check_timeout_secs = request_timeout_secs.max(1);

    let mgr = LlamaServerManager::new(
        config,
        "/tmp/Qwen3VL-4B-Instruct-Q4_K_M.gguf".to_string(),
        Some("/tmp/mmproj-Qwen3VL-4B-Instruct-F16.gguf".to_string()),
    );
    mgr.set_api_url_for_testing(format!("{api_root}/v1")).await;
    mgr
}

#[tokio::test]
async fn api_unload_model_calls_v1_models_unload_endpoint() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/models/unload"))
        .and(body_json(serde_json::json!({
            "model": "Qwen3VL-4B-Instruct-Q4_K_M.gguf"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mgr = build_manager(&server.uri(), 5).await;
    mgr.api_unload_model()
        .await
        .expect("expected API unload to succeed");
}

#[tokio::test]
async fn api_unload_model_returns_router_mode_error_on_404() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/models/unload"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string("router mode endpoint not available in this llama build"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mgr = build_manager(&server.uri(), 5).await;
    let err = mgr
        .api_unload_model()
        .await
        .expect_err("expected 404 to force fallback error");
    let msg = err.to_string();

    assert!(
        msg.contains("Router Mode not supported") && msg.contains("404"),
        "unexpected error message: {msg}"
    );
}

#[tokio::test]
async fn api_unload_model_returns_router_mode_error_on_501() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/models/unload"))
        .respond_with(ResponseTemplate::new(501).set_body_string("not implemented"))
        .expect(1)
        .mount(&server)
        .await;

    let mgr = build_manager(&server.uri(), 5).await;
    let err = mgr
        .api_unload_model()
        .await
        .expect_err("expected 501 to force fallback error");
    let msg = err.to_string();

    assert!(
        msg.contains("Router Mode not supported") && msg.contains("501"),
        "unexpected error message: {msg}"
    );
}

#[tokio::test]
async fn api_unload_model_surfaces_timeout_as_transport_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/models/unload"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(2))
                .set_body_json(serde_json::json!({"ok": true})),
        )
        .expect(1)
        .mount(&server)
        .await;

    // api_unload_model timeout is clamped from config.health_check_timeout_secs to <= 30.
    // Set to 1s so this test runs fast and deterministically triggers timeout handling.
    let mgr = build_manager(&server.uri(), 1).await;
    let err = mgr
        .api_unload_model()
        .await
        .expect_err("expected delayed mock response to trigger timeout");
    let msg = err.to_string();

    assert!(
        msg.contains("transport error") || msg.contains("timed out"),
        "unexpected timeout error message: {msg}"
    );
}
