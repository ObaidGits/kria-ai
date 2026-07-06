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
    assert!(
        warm_before > 0,
        "expected at least one warm container after initialize"
    );

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
    assert!(
        initial >= 2,
        "expected at least warm_per_class=2 containers"
    );

    // Checkout all Light containers to drain that class
    use kria_core::openclaw::ResourceClass;
    let h1 = pool
        .checkout(ResourceClass::Light, "test")
        .await
        .expect("checkout 1 failed");
    let h2 = pool
        .checkout(ResourceClass::Light, "test")
        .await
        .expect("checkout 2 failed");

    println!("Drained Light pool — active={}", pool.active_count().await);

    // Spawn pre-warm loop and wait one cycle (6s to be safe)
    ContainerPool::spawn_prewarm_loop(pool.clone());
    tokio::time::sleep(std::time::Duration::from_secs(7)).await;

    pool.checkin(h1).await.ok();
    pool.checkin(h2).await.ok();

    let after_prewarm = pool.warm_count_total().await;
    println!("After pre-warm loop cycle: warm={after_prewarm}");
    // Pool should have recovered (pre-warm loop refills it)
    assert!(
        after_prewarm >= 2,
        "pre-warm loop should have refilled the pool"
    );

    println!("[PASS] live_pool_prewarm_loop_maintains_count");
}

#[tokio::test]
async fn live_audit_records_invocation_started() {
    require_docker!();

    use kria_core::infra::ToolResult;
    use kria_core::openclaw::audit::AuditLedger;
    use kria_core::openclaw::types::AuditEventType;
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
        &ToolResult {
            success: true,
            data: serde_json::Value::Null,
            error: None,
        },
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
        &ToolResult {
            success: true,
            data: serde_json::json!("4"),
            error: None,
        },
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

/// A1 end-to-end success criterion: a real skill executes through the full `SkillRuntime`
/// pipeline (HRA admission → container checkout → bollard `exec` → MCP JSON-RPC → result →
/// SkillEvents → cleanup) and returns the correct value with no leaked containers/leases.
///
/// Requires Docker + the built `kria/openclaw-substrate:latest` image (which bakes in the
/// bundled `oc_calculator` skill). This is the hardware-validation gate for Phase A1.
#[tokio::test]
async fn live_runtime_executes_calculator_end_to_end() {
    require_docker!();

    use kria_core::openclaw::event::{self, Stage};
    use kria_core::openclaw::handler::build_runtime_registry;
    use kria_core::openclaw::runtime::{LaunchSpec, RuntimeContext, RuntimeKind};
    use kria_core::openclaw::{ContainerPool, OpenClawConfig, ResourceClass};
    use std::sync::Arc;
    use std::time::Duration;

    let pool = Arc::new(
        ContainerPool::new(OpenClawConfig::default())
            .await
            .expect("ContainerPool::new failed — is Docker running?"),
    );
    pool.initialize().await.expect("initialize failed");

    let runtimes = build_runtime_registry(pool.clone());
    let runtime = runtimes
        .get(RuntimeKind::Docker)
        .expect("docker runtime registered");
    assert_eq!(runtime.kind(), RuntimeKind::Docker);

    // Subscribe to the single event stream and assert lifecycle stages are emitted.
    let mut events = event::subscribe();

    let spec = LaunchSpec {
        skill_id: "oc_calculator".to_string(),
        params: serde_json::json!({ "expression": "2 * (3 + 4)" }),
        resource_class: ResourceClass::Light,
        timeout: Duration::from_secs(30),
        correlation_id: "live-e2e-1".to_string(),
        grants: Vec::new(),
        mounted_skill_dir: None,
    };

    let result = runtime.execute(spec, RuntimeContext::detached()).await;
    println!("runtime result: {result:?}");

    assert!(
        result.success,
        "calculator skill should succeed: {result:?}"
    );
    // Bundled calculator returns {"expression":..., "result": 14}.
    let value = result
        .data
        .get("result")
        .and_then(|v| v.as_f64())
        .expect("result field present");
    assert_eq!(value, 14.0, "2 * (3 + 4) must equal 14");

    // Drain events; assert we saw Started → Running → Completed for our correlation id.
    let mut saw_started = false;
    let mut saw_running = false;
    let mut saw_completed = false;
    while let Ok(ev) = events.try_recv() {
        if ev.correlation_id != "live-e2e-1" {
            continue;
        }
        match ev.stage {
            Stage::Started => saw_started = true,
            Stage::Running => saw_running = true,
            Stage::Completed => saw_completed = true,
            _ => {}
        }
    }
    assert!(
        saw_started && saw_running && saw_completed,
        "expected full lifecycle events"
    );

    // No leaked containers / leases: active count returns to 0 after execution.
    assert_eq!(
        pool.active_count().await,
        0,
        "no active invocations should leak"
    );

    println!("[PASS] live_runtime_executes_calculator_end_to_end — result=14, no leaks");
}

/// Regression: graceful shutdown must destroy every substrate container (no leak
/// on app exit). Also implicitly validates the unique-naming + reap fixes.
#[tokio::test]
async fn live_shutdown_destroys_all_containers() {
    require_docker!();

    use bollard::container::ListContainersOptions;
    use kria_core::openclaw::{ContainerPool, OpenClawConfig};
    use std::collections::HashMap;
    use std::sync::Arc;

    let cfg = OpenClawConfig::default();
    let name_prefix = cfg.container_name.clone();
    let pool = Arc::new(ContainerPool::new(cfg).await.expect("pool new failed"));

    let warm = pool.warm_count_total().await;
    assert!(warm > 0, "expected warm containers after new()");

    pool.shutdown().await.expect("shutdown failed");

    // Assert Docker has no containers left with our substrate name prefix.
    let docker = pool.docker();
    let mut filters = HashMap::new();
    filters.insert("name".to_string(), vec![name_prefix]);
    let remaining = docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        }))
        .await
        .expect("list_containers failed");
    assert_eq!(
        remaining.len(),
        0,
        "shutdown must leave no substrate containers"
    );
    println!("[PASS] live_shutdown_destroys_all_containers — no leak");
}

/// Regression: "Restart Substrate" (rewarm) drains and re-warms without leaving
/// the pool empty.
#[tokio::test]
async fn live_restart_rewarms_pool() {
    require_docker!();

    use kria_core::openclaw::{ContainerPool, OpenClawConfig};
    use std::sync::Arc;

    let pool = Arc::new(
        ContainerPool::new(OpenClawConfig::default())
            .await
            .expect("pool new failed"),
    );
    assert!(pool.warm_count_total().await > 0, "warm after new()");

    pool.rewarm().await.expect("rewarm failed");
    assert!(pool.warm_count_total().await > 0, "warm after rewarm()");

    // Clean up so the test leaves nothing behind.
    pool.shutdown().await.expect("shutdown failed");
    println!("[PASS] live_restart_rewarms_pool");
}

/// Regression (task 2 re-investigation): `RuntimeManager::shutdown()` must
/// GENUINELY stop the prewarming background loop before returning, not just
/// destroy the containers that existed at shutdown time. Proven by: shutting
/// down, then waiting past one full `prewarming_interval`, then asserting no
/// NEW warm container appeared — if the prewarm loop were still running
/// after `shutdown()` returned (the bug this test guards), it would have
/// created a fresh container during the wait and this assertion would fail.
///
/// Also exercises the `health_task`/`recycler_task` handle-swap fix
/// (`regr_r2_idle_recycling_overwrites_health_task_handle`): before that fix,
/// `start_idle_recycling` overwrote `self.health_task`, so the REAL health
/// monitor's `JoinHandle` was silently dropped and could never be joined —
/// `shutdown()` would (with the join-based fix) have joined the recycler loop
/// TWICE and the health monitor NEVER, which this test's "no task still
/// creating containers after shutdown" assertion also would have caught if
/// the health monitor were the one responsible for any creation path.
#[tokio::test]
async fn live_shutdown_genuinely_stops_prewarm_loop() {
    require_docker!();

    use bollard::container::ListContainersOptions;
    use kria_core::openclaw::{ContainerPool, OpenClawConfig};
    use std::collections::HashMap;
    use std::sync::Arc;

    let cfg = OpenClawConfig::default();
    let name_prefix = cfg.container_name.clone();
    let pool = Arc::new(ContainerPool::new(cfg).await.expect("pool new failed"));

    assert!(
        pool.warm_count_total().await > 0,
        "expected warm containers after new()"
    );

    pool.shutdown().await.expect("shutdown failed");

    // Confirm zero containers immediately after shutdown (existing coverage).
    let docker = pool.docker();
    let list_ours = || {
        let mut filters = HashMap::new();
        filters.insert("name".to_string(), vec![name_prefix.clone()]);
        ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        }
    };
    let immediately_after = docker
        .list_containers(Some(list_ours()))
        .await
        .expect("list_containers failed");
    assert_eq!(
        immediately_after.len(),
        0,
        "shutdown must leave no substrate containers immediately"
    );

    // The REAL bug this guards: if a background loop were still alive after
    // shutdown() returned, it would eventually create a new container on its
    // next tick. Default `prewarming_interval` — wait comfortably past one
    // tick and re-check. (Default warm pool config ticks well under 10s in
    // this codebase's test configuration; if this ever times out spuriously
    // because the interval grows, that is itself a signal worth re-checking,
    // not a reason to weaken this assertion.)
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    let after_wait = docker
        .list_containers(Some(list_ours()))
        .await
        .expect("list_containers failed");
    assert_eq!(
        after_wait.len(),
        0,
        "no background task may create a container after shutdown() has returned — \
         a non-zero count here means a background loop is still running post-shutdown"
    );

    println!("[PASS] live_shutdown_genuinely_stops_prewarm_loop — 0 containers immediately and after one full interval");
}

/// Real execution of a NEW skill (oc_text_tool) through the SAME real
/// Docker runtime path — proves the rebuilt substrate image with 8 skills
/// is genuinely reachable and functional end-to-end.
#[tokio::test]
async fn live_runtime_executes_text_tool_end_to_end() {
    if std::env::var("KRIA_LIVE_DOCKER_TESTS").is_err() {
        println!("[SKIP] set KRIA_LIVE_DOCKER_TESTS=1 to run live Docker tests");
        return;
    }

    use kria_core::openclaw::handler::build_runtime_registry;
    use kria_core::openclaw::runtime::{LaunchSpec, RuntimeContext, RuntimeKind};
    use kria_core::openclaw::{ContainerPool, OpenClawConfig, ResourceClass};
    use std::sync::Arc;
    use std::time::Duration;

    let pool = Arc::new(
        ContainerPool::new(OpenClawConfig::default())
            .await
            .expect("ContainerPool::new failed — is Docker running?"),
    );
    pool.initialize().await.expect("initialize failed");

    let runtimes = build_runtime_registry(pool.clone());
    let runtime = runtimes
        .get(RuntimeKind::Docker)
        .expect("docker runtime registered");

    let spec = LaunchSpec {
        skill_id: "oc_text_tool".to_string(),
        params: serde_json::json!({ "text": "Hello world from KRIA OpenClaw", "op": "stats" }),
        resource_class: ResourceClass::Light,
        timeout: Duration::from_secs(30),
        correlation_id: "live-text-tool-1".to_string(),
        grants: Vec::new(),
        mounted_skill_dir: None,
    };

    let result = runtime.execute(spec, RuntimeContext::detached()).await;
    println!("text_tool result: {result:?}");
    assert!(
        result.success,
        "oc_text_tool must execute successfully in the real substrate: {result:?}"
    );
    println!("[PASS] live_runtime_executes_text_tool_end_to_end");

    let _ = pool.shutdown().await;
}

/// RC2 (registry↔container sync): the registry must reflect EVERY skill the
/// container actually advertises via `tools/list`, each with its real
/// `inputSchema`. Proves the mis-routing root cause is fixed generally — the
/// router's candidate set is no longer a hardcoded subset.
#[tokio::test]
async fn live_registry_syncs_all_container_skills_with_schemas() {
    require_docker!();

    use kria_core::openclaw::init::sync_registry_from_container;
    use kria_core::openclaw::registry::ProductionSkillRegistry;
    use kria_core::openclaw::{ContainerPool, OpenClawConfig};
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let registry =
        Arc::new(ProductionSkillRegistry::new(&dir.path().join("sync.db")).expect("registry"));
    let pool = Arc::new(
        ContainerPool::new(OpenClawConfig::default())
            .await
            .expect("ContainerPool::new — is Docker running?"),
    );
    pool.initialize().await.expect("pool init");

    let changed = sync_registry_from_container(&registry, pool.clone())
        .await
        .expect("sync must succeed against a real container");
    println!("sync changed {changed} skills");

    let enabled = registry.get_enabled_skills().expect("enabled skills");
    let ids: Vec<String> = enabled.iter().map(|s| s.skill_id.clone()).collect();
    println!("enabled after sync: {ids:?}");

    // Every baked skill must now be present + routable, each with a real schema.
    for expected in [
        "oc_calculator",
        "oc_json_tool",
        "oc_csv_tool",
        "oc_regex_tool",
        "oc_markdown_tool",
        "oc_text_tool",
        "oc_gzip_tool",
        "oc_hash_tool",
    ] {
        let skill = enabled
            .iter()
            .find(|s| s.skill_id == expected)
            .unwrap_or_else(|| panic!("expected {expected} in registry after sync"));
        assert!(
            skill.input_schema.is_some(),
            "{expected} must have its inputSchema persisted after sync"
        );
    }

    println!("[PASS] live_registry_syncs_all_container_skills_with_schemas");
}

/// GOLD-STANDARD full e2e (RC1+RC2 together): a natural-language `query`
/// through the REAL semantic handler → RC1 schema-driven argument generation
/// (real LLM) → real Docker container → correct result. This is the exact
/// failing case ("missing required parameter: expression") now passing end to
/// end with NO prompt-specific logic.
///
/// Gated on BOTH `KRIA_LIVE_DOCKER_TESTS` and `KRIA_LLAMA_API_URL`.
#[tokio::test]
async fn live_e2e_natural_language_to_calculator_result() {
    require_docker!();
    let Ok(llm_url) = std::env::var("KRIA_LLAMA_API_URL") else {
        eprintln!("[SKIP] set KRIA_LLAMA_API_URL to run the full NL→arg-gen→container e2e");
        return;
    };

    use kria_core::llm::ModelRouter;
    use kria_core::openclaw::audit::AuditLedger;
    use kria_core::openclaw::handler::{build_runtime_registry, SemanticOpenClawHandler};
    use kria_core::openclaw::init::sync_registry_from_container;
    use kria_core::openclaw::registry::ProductionSkillRegistry;
    use kria_core::openclaw::{ContainerPool, OpenClawConfig};
    use kria_core::tools::registry::ToolHandler;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("e2e.db");
    let registry = Arc::new(ProductionSkillRegistry::new(&db).expect("registry"));
    let pool = Arc::new(
        ContainerPool::new(OpenClawConfig::default())
            .await
            .expect("pool"),
    );
    pool.initialize().await.expect("pool init");

    // RC2: registry reflects the container's real skill set + schemas.
    sync_registry_from_container(&registry, pool.clone())
        .await
        .expect("sync");

    // Real model router pointed at the running llama-server (RC1 arg-gen).
    let mut config = kria_core::config::KriaConfig::default();
    config.llm.local_api_url = if llm_url.ends_with("/v1") {
        llm_url
    } else {
        format!("{}/v1", llm_url.trim_end_matches('/'))
    };
    let router = Arc::new(ModelRouter::from_config(&config));

    let runtimes = build_runtime_registry(pool.clone());
    let audit = Arc::new(AuditLedger::open(&db, b"e2e-key".to_vec()).expect("audit"));
    let handler = SemanticOpenClawHandler::new(registry, runtimes, audit).with_arg_gen_llm(router);

    // The ORIGINAL failing prompt — freeform NL, no `expression` field supplied.
    let result = handler
        .execute(serde_json::json!({ "query": "calculate 3+3" }))
        .await;

    let payload = serde_json::to_string(&result.data).unwrap_or_default();
    println!(
        "e2e result: success={} data={payload} error={:?}",
        result.success, result.error
    );
    assert!(
        result.success,
        "execution must succeed, got error: {:?}",
        result.error
    );
    assert!(
        payload.contains('6'),
        "calculator must return 6 for 3+3 (got {payload})"
    );

    println!("[PASS] live_e2e_natural_language_to_calculator_result");
}
