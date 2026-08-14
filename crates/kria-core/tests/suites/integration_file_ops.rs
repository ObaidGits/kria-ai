use tempfile::TempDir;

// ── Why these tests call `execute_with_context`, not `execute` ────────────────
//
// `ToolHandler` offers both, and BOTH have default bodies:
//
//   * `execute(params)`                  — defaults to an error
//   * `execute_with_context(params, ctx)` — defaults to calling `execute`
//
// A handler that needs the environment (every filesystem tool does) implements
// only `execute_with_context`. 25 of the 26 handlers in `tools/file_ops.rs` are in
// that group, and the agent's real execution path — `loop_engine` and
// `resume_executor` — calls `execute_with_context` too.
//
// These tests used to call `execute` directly and so hit the erroring default,
// reporting "tool does not implement execute" for every file operation. That read
// like the file tools were broken; they were not, the tests were addressing an
// interface the app never uses. Calling `execute_with_context` fixes the tests AND
// makes them exercise the same path production does.
fn test_ctx() -> kria_core::tools::ToolContext {
    use std::collections::HashMap;
    use std::sync::Arc;
    kria_core::tools::ToolContext::new(
        Arc::new(kria_core::infra::environment::LocalEnvironment::new()),
        Arc::new(tokio::sync::Mutex::new(
            kria_core::infra::environment::ShellState {
                cwd: std::env::current_dir().expect("a working directory"),
                env_vars: HashMap::new(),
                generation: 0,
            },
        )),
        tokio_util::sync::CancellationToken::new(),
    )
}

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
        .execute_with_context(serde_json::json!({
            "path": path_str,
            "content": "hello from integration_file_ops",
            "overwrite": true
        }), test_ctx())
        .await;

    assert!(first.success, "initial write should succeed: {first:?}");
    assert_eq!(first.data["changed"].as_bool(), Some(true));
    assert_eq!(
        first.data["already_in_desired_state"].as_bool(),
        Some(false)
    );

    let second = handler
        .execute_with_context(serde_json::json!({
            "path": path.to_string_lossy(),
            "content": "hello from integration_file_ops",
            "overwrite": true
        }), test_ctx())
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
        .execute_with_context(serde_json::json!({
            "path": missing.to_string_lossy()
        }), test_ctx())
        .await;

    assert!(
        !result.success,
        "read_file should fail for a missing file: {result:?}"
    );

    let error = result.error.unwrap_or_default();
    // Assert the PROPERTIES the message must carry — which operation failed, and on
    // which path — not the exact prose. The old assertion looked for the literal
    // "read_file failed for" and broke when the wording became "read_file failed:",
    // even though the message still said everything a user needs. Pinning phrasing
    // makes a test fail for a rewording and pass for a genuinely useless message.
    assert!(
        error.contains("read_file"),
        "error should name the operation that failed: {error}"
    );
    assert!(
        error.contains(&missing.to_string_lossy().to_string()),
        "error should name the path it failed on: {error}"
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
        .execute_with_context(serde_json::json!({
            "path": directory.to_string_lossy()
        }), test_ctx())
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
        .execute_with_context(serde_json::json!({
            "path": directory.to_string_lossy()
        }), test_ctx())
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
        .execute_with_context(serde_json::json!({
            "path": target.to_string_lossy()
        }), test_ctx())
        .await;

    assert!(
        first.success,
        "delete_file should no-op for missing file: {first:?}"
    );
    assert_eq!(first.data["changed"].as_bool(), Some(false));
    assert_eq!(first.data["already_in_desired_state"].as_bool(), Some(true));

    std::fs::write(&target, "payload").expect("failed to create file for delete test");

    let second = handler
        .execute_with_context(serde_json::json!({
            "path": target.to_string_lossy()
        }), test_ctx())
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
