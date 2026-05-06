use std::fs;

use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn enable_local_test_urls() {
    std::env::set_var("KRIA_ALLOW_LOCAL_TEST_URLS", "1");
}

#[tokio::test]
async fn fetch_webpage_maps_404_to_structured_tool_error() {
    enable_local_test_urls();

    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .expect(3)
        .mount(&mock_server)
        .await;

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("fetch_webpage")
        .expect("fetch_webpage handler missing");

    let url = format!("{}/missing", mock_server.uri());
    let result = handler
        .execute(serde_json::json!({
            "url": url,
            "max_chars": 1024
        }))
        .await;

    assert!(
        !result.success,
        "fetch_webpage should fail for mocked 404: {result:?}"
    );

    let error = result.error.unwrap_or_default();
    assert!(
        error.contains("fetch_webpage") && error.contains("HTTP status 404"),
        "error should include operation + status code: {error}"
    );
    assert!(
        error.contains("/missing"),
        "error should include failing URL context: {error}"
    );

    mock_server.verify().await;
}

#[tokio::test]
async fn download_file_is_idempotent_when_destination_exists() {
    enable_local_test_urls();

    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/asset"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"integration-bytes".to_vec()))
        .expect(1)
        .mount(&mock_server)
        .await;

    let sandbox = TempDir::new().expect("failed to create tempdir");
    let destination = sandbox.path().join("downloaded.bin");

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("download_file")
        .expect("download_file handler missing");

    let url = format!("{}/asset", mock_server.uri());
    let first = handler
        .execute(serde_json::json!({
            "url": url,
            "destination": destination.to_string_lossy(),
            "max_size_mb": 5
        }))
        .await;

    assert!(first.success, "first download should succeed: {first:?}");
    assert_eq!(first.data["changed"].as_bool(), Some(true));
    assert_eq!(
        first.data["already_in_desired_state"].as_bool(),
        Some(false)
    );

    let second = handler
        .execute(serde_json::json!({
            "url": format!("{}/asset", mock_server.uri()),
            "destination": destination.to_string_lossy(),
            "max_size_mb": 5
        }))
        .await;

    assert!(
        second.success,
        "second download should no-op idempotently: {second:?}"
    );
    assert_eq!(second.data["changed"].as_bool(), Some(false));
    assert_eq!(
        second.data["already_in_desired_state"].as_bool(),
        Some(true)
    );

    let on_disk = fs::read(&destination).expect("failed to read downloaded file");
    assert_eq!(on_disk, b"integration-bytes");

    mock_server.verify().await;
}
