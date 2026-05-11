use tempfile::TempDir;

#[tokio::test]
async fn write_file_idempotency_skips_second_identical_write() {
    let sandbox = TempDir::new().expect("failed to create tempdir");
    let path = sandbox.path().join("idempotent.txt");
    let path_str = path.to_string_lossy().to_string();

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("write_file")
        .expect("write_file handler missing");

    let first = handler
        .execute(serde_json::json!({
            "path": path_str,
            "content": "hello from integration_file_ops",
            "overwrite": true
        }))
        .await;

    assert!(first.success, "initial write should succeed: {first:?}");
    assert_eq!(first.data["changed"].as_bool(), Some(true));
    assert_eq!(
        first.data["already_in_desired_state"].as_bool(),
        Some(false)
    );

    let second = handler
        .execute(serde_json::json!({
            "path": path.to_string_lossy(),
            "content": "hello from integration_file_ops",
            "overwrite": true
        }))
        .await;

    assert!(
        second.success,
        "idempotent second write should succeed: {second:?}"
    );
    assert_eq!(second.data["changed"].as_bool(), Some(false));
    assert_eq!(
        second.data["already_in_desired_state"].as_bool(),
        Some(true)
    );

    let disk_content = std::fs::read_to_string(path).expect("failed to read output file");
    assert_eq!(disk_content, "hello from integration_file_ops");
}

#[tokio::test]
async fn read_file_surfaces_io_error_for_missing_path() {
    let sandbox = TempDir::new().expect("failed to create tempdir");
    let missing = sandbox.path().join("missing.txt");

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("read_file")
        .expect("read_file handler missing");

    let result = handler
        .execute(serde_json::json!({
            "path": missing.to_string_lossy()
        }))
        .await;

    assert!(
        !result.success,
        "read_file should fail for a missing file: {result:?}"
    );

    let error = result.error.unwrap_or_default();
    assert!(
        error.contains("read_file failed for"),
        "error should include operation and path context: {error}"
    );
    assert!(
        error.contains("No such file") || error.contains("not found"),
        "error should include OS reason for missing file: {error}"
    );
}

#[tokio::test]
async fn create_directory_idempotency_reports_already_in_desired_state() {
    let sandbox = TempDir::new().expect("failed to create tempdir");
    let directory = sandbox.path().join("nested").join("dir");

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("create_directory")
        .expect("create_directory handler missing");

    let first = handler
        .execute(serde_json::json!({
            "path": directory.to_string_lossy()
        }))
        .await;

    assert!(
        first.success,
        "initial create_directory should succeed: {first:?}"
    );
    assert_eq!(first.data["changed"].as_bool(), Some(true));
    assert_eq!(
        first.data["already_in_desired_state"].as_bool(),
        Some(false)
    );

    let second = handler
        .execute(serde_json::json!({
            "path": directory.to_string_lossy()
        }))
        .await;

    assert!(
        second.success,
        "idempotent create_directory should succeed: {second:?}"
    );
    assert_eq!(second.data["changed"].as_bool(), Some(false));
    assert_eq!(
        second.data["already_in_desired_state"].as_bool(),
        Some(true)
    );
}

#[tokio::test]
async fn delete_file_idempotency_reports_already_in_desired_state_for_missing() {
    let sandbox = TempDir::new().expect("failed to create tempdir");
    let target = sandbox.path().join("delete_me.txt");

    let reg = kria_core::tools::registry::build_default_registry();
    let handler = reg
        .get_handler("delete_file")
        .expect("delete_file handler missing");

    let first = handler
        .execute(serde_json::json!({
            "path": target.to_string_lossy()
        }))
        .await;

    assert!(
        first.success,
        "delete_file should no-op for missing file: {first:?}"
    );
    assert_eq!(first.data["changed"].as_bool(), Some(false));
    assert_eq!(first.data["already_in_desired_state"].as_bool(), Some(true));

    std::fs::write(&target, "payload").expect("failed to create file for delete test");

    let second = handler
        .execute(serde_json::json!({
            "path": target.to_string_lossy()
        }))
        .await;

    assert!(
        second.success,
        "delete_file should delete existing file: {second:?}"
    );
    assert_eq!(second.data["changed"].as_bool(), Some(true));
    assert_eq!(
        second.data["already_in_desired_state"].as_bool(),
        Some(false)
    );
}
