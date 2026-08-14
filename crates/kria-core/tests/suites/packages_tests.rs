/// Package tools regression tests.
///
/// linux-os-control-production Task 3.4: `search_package` is now a thin
/// facade over the governed `PackageControl` provider (never a direct
/// subprocess). These tests guard the input-validation surface that runs
/// before the governed runtime call.

#[test]
fn search_package_schema_keeps_query_and_alias_name() {
    use kria_core::tools::registry::build_default_registry;

    let reg = build_default_registry();
    let def = reg
        .get_def("search_package")
        .expect("search_package should be registered");

    let query_param = def
        .parameters
        .iter()
        .find(|p| p.name == "query")
        .expect("query param should exist");
    assert!(
        query_param.required,
        "query should remain required for canonical calls"
    );

    let alias_param = def
        .parameters
        .iter()
        .find(|p| p.name == "name")
        .expect("name alias should exist");
    assert!(!alias_param.required, "name alias should be optional");
}

#[tokio::test]
async fn search_package_requires_query_or_name() {
    use kria_core::tools::registry::build_default_registry;
    use tokio_util::sync::CancellationToken;

    let reg = build_default_registry();
    let handler = reg
        .get_handler("search_package")
        .expect("search_package handler should exist");
    let ctx = reg.make_tool_context(CancellationToken::new());

    // Input validation happens in `execute_with_context` before the governed
    // runtime call, so it runs even though no live PackageControl provider
    // is composed in this test registry.
    let result = handler
        .execute_with_context(
            serde_json::json!({
                "provider": "apt"
            }),
            ctx,
        )
        .await;

    assert!(!result.success);
    let err = result.error.unwrap_or_default();
    assert!(
        err.contains("query parameter is required"),
        "expected missing query/name error, got: {err}"
    );
}

#[tokio::test]
async fn search_package_accepts_name_alias_when_query_missing() {
    use kria_core::tools::registry::build_default_registry;
    use tokio_util::sync::CancellationToken;

    let reg = build_default_registry();
    let handler = reg
        .get_handler("search_package")
        .expect("search_package handler should exist");
    let ctx = reg.make_tool_context(CancellationToken::new());

    // No live PackageControl provider is composed in this test registry, so
    // the handler still fails closed with the frozen `Unavailable` envelope
    // — the important assertion is that the `name` alias satisfies the
    // query requirement rather than failing with "query parameter is
    // required".
    let result = handler
        .execute_with_context(
            serde_json::json!({
                "name": "chromium"
            }),
            ctx,
        )
        .await;

    assert!(
        !result.success,
        "no live provider composed means Unavailable"
    );
    let err = result.error.unwrap_or_default();
    assert!(
        !err.contains("query parameter is required"),
        "name alias should satisfy query input, got: {err}"
    );
}

#[tokio::test]
async fn search_package_rejects_unknown_provider() {
    use kria_core::tools::registry::build_default_registry;
    use tokio_util::sync::CancellationToken;

    let reg = build_default_registry();
    let handler = reg
        .get_handler("search_package")
        .expect("search_package handler should exist");
    let ctx = reg.make_tool_context(CancellationToken::new());

    let result = handler
        .execute_with_context(
            serde_json::json!({
                "query": "htop",
                "provider": "not-a-real-provider"
            }),
            ctx,
        )
        .await;

    assert!(!result.success, "invalid provider should still fail");
    let err = result.error.unwrap_or_default();
    assert!(
        err.contains("unknown package provider"),
        "expected provider validation error, got: {err}"
    );
}
