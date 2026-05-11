//! Live Docker integration test for the OpenClaw container pool.
//!
//! Requires Docker to be running with `kria/openclaw-substrate:latest` image.
//! Run with: cargo test --package kria-core --test openclaw_live_docker -- --nocapture
//!
//! These tests are gated behind the `KRIA_LIVE_DOCKER_TESTS` env var so they
//! are skipped in CI unless explicitly opted in.

macro_rules! require_docker {
    () => {
        if std::env::var("KRIA_LIVE_DOCKER_TESTS").is_err() {
            eprintln!("[SKIP] set KRIA_LIVE_DOCKER_TESTS=1 to run live Docker tests");
            return;
        }
    };
}

#[tokio::test]
async fn live_pool_checkout_and_checkin() {
    require_docker!();

    use kria_core::openclaw::{ContainerPool, OpenClawConfig};
    use std::sync::Arc;

    let pool = Arc::new(
        ContainerPool::new(OpenClawConfig::default())
            .await
            .expect("ContainerPool::new failed — is Docker running?"),
    );

    pool.initialize().await.expect("initialize failed");

    let active_before = pool.active_count().await;
    let warm_before = pool.warm_count_total().await;
    println!("Before checkout: active={active_before}, warm={warm_before}");
    assert!(warm_before > 0, "expected at least one warm container after initialize");

    // Checkout a Light container
    use kria_core::openclaw::ResourceClass;
    let handle = pool
        .checkout(ResourceClass::Light, "oc_calculator")
        .await
        .expect("checkout failed");

    println!("Checked out container: id={}", handle.container_id);
    assert!(!handle.container_id.is_empty());

    let active_during = pool.active_count().await;
    println!("During invocation: active={active_during}");
    assert_eq!(active_during, 1, "should have exactly 1 active invocation");

    // Checkin — destroys the container
    pool.checkin(handle).await.expect("checkin failed");

    let active_after = pool.active_count().await;
    println!("After checkin: active={active_after}");
    assert_eq!(active_after, 0, "active count should be 0 after checkin");

    println!("[PASS] live_pool_checkout_and_checkin");
}

#[tokio::test]
async fn live_pool_prewarm_loop_maintains_count() {
    require_docker!();

    use kria_core::openclaw::{ContainerPool, OpenClawConfig};
    use std::sync::Arc;

    let pool = Arc::new(
        ContainerPool::new(OpenClawConfig::default())
            .await
            .expect("ContainerPool::new failed"),
    );
    pool.initialize().await.expect("initialize failed");

    let initial = pool.warm_count_total().await;
    println!("Initial warm count: {initial}");
    assert!(initial >= 2, "expected at least warm_per_class=2 containers");

    // Checkout all Light containers to drain that class
    use kria_core::openclaw::ResourceClass;
    let h1 = pool.checkout(ResourceClass::Light, "test").await.expect("checkout 1 failed");
    let h2 = pool.checkout(ResourceClass::Light, "test").await.expect("checkout 2 failed");

    println!("Drained Light pool — active={}", pool.active_count().await);

    // Spawn pre-warm loop and wait one cycle (6s to be safe)
    ContainerPool::spawn_prewarm_loop(pool.clone());
    tokio::time::sleep(std::time::Duration::from_secs(7)).await;

    pool.checkin(h1).await.ok();
    pool.checkin(h2).await.ok();

    let after_prewarm = pool.warm_count_total().await;
    println!("After pre-warm loop cycle: warm={after_prewarm}");
    // Pool should have recovered (pre-warm loop refills it)
    assert!(after_prewarm >= 2, "pre-warm loop should have refilled the pool");

    println!("[PASS] live_pool_prewarm_loop_maintains_count");
}

#[tokio::test]
async fn live_audit_records_invocation_started() {
    require_docker!();

    use kria_core::openclaw::audit::AuditLedger;
    use kria_core::openclaw::types::AuditEventType;
    use kria_core::infra::ToolResult;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let db = dir.path().join("audit_test.db");
    let ledger = AuditLedger::open(&db, b"test-key-0001".to_vec()).expect("open failed");

    let mut entry = AuditLedger::create_invocation_entry(
        AuditEventType::InvocationStarted,
        "oc_calculator",
        "test-inv-001",
        "session-1",
        "turn-1",
        "oc_calculator",
        "green",
        &serde_json::json!({"expr": "2+2"}),
        &ToolResult { success: true, data: serde_json::Value::Null, error: None },
        0,
        "light",
        "",
    );
    entry.signature = ledger.sign_entry(&entry);
    let row_id = ledger.append(&entry).expect("append failed");
    println!("Wrote InvocationStarted row_id={row_id}");
    assert!(row_id > 0);

    let mut completed = AuditLedger::create_invocation_entry(
        AuditEventType::InvocationCompleted,
        "oc_calculator",
        "test-inv-001",
        "session-1",
        "turn-1",
        "oc_calculator",
        "green",
        &serde_json::json!({"expr": "2+2"}),
        &ToolResult { success: true, data: serde_json::json!("4"), error: None },
        45,
        "light",
        "c0ntainer123",
    );
    completed.signature = ledger.sign_entry(&completed);
    let row_id2 = ledger.append(&completed).expect("append completed failed");
    println!("Wrote InvocationCompleted row_id={row_id2}");

    // Verify chain integrity
    let tampered = ledger.verify_chain().expect("verify_chain failed");
    assert!(tampered.is_none(), "audit chain should be intact");
    println!("[PASS] live_audit_records_invocation_started — chain intact");
}
