//! Milestone 10 (implementation-phase) diverse real-prompt battery.
//!
//! Executes a DIVERSE set of real capabilities across BOTH providers through the
//! CPP `CapabilityPlatform` on real Docker + real node — not calculator-only.
//! Covers: arithmetic, hashing, JSON, regex, text ops, CSV, markdown (OpenClaw
//! baked skills) + reverse/word-count (MCP). Proves the provider-neutral
//! execution path works across capability categories.
//!
//! Gated on `KRIA_CPP_DOCKER=1` (Docker + substrate image + node). Run:
//! ```bash
//! KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_prompt_battery_docker -- --nocapture
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use kria_core::capability::acl::mcp::McpProvider;
use kria_core::capability::index::{InMemoryFederatedIndex, MemoryEmbedder};
use kria_core::capability::platform::CapabilityPlatform;
use kria_core::capability::provider::{CapabilityRequest, RequestContext};
use kria_core::capability::registry::ProviderRegistry;
use kria_core::capability::{CapabilityOutcome, OpenClawProvider};
use kria_core::openclaw::config::OpenClawConfig;
use kria_core::openclaw::pool::ContainerPool;
use kria_core::openclaw::registry::ProductionSkillRegistry;
use kria_core::openclaw::runtime::{DockerRuntime, SkillRuntime};

fn req(provider: &str, cap: &str, args: serde_json::Value) -> CapabilityRequest {
    CapabilityRequest {
        provider_id: provider.to_string(),
        capability_id: cap.to_string(),
        args,
        context: RequestContext::new(),
        granted_effects: vec![],
    }
}

#[tokio::test]
async fn diverse_prompt_battery_across_providers() {
    if std::env::var("KRIA_CPP_DOCKER").is_err() {
        eprintln!("skipping: set KRIA_CPP_DOCKER=1");
        return;
    }

    // OpenClaw (Docker) + MCP (node) behind one platform.
    let tmp_db: PathBuf =
        std::env::temp_dir().join(format!("kria_cpp_battery_{}.db", std::process::id()));
    let real_db = dirs::home_dir().unwrap().join(".kria/skills.db");
    if real_db.exists() {
        std::fs::copy(&real_db, &tmp_db).ok();
    }
    let oc_registry = Arc::new(
        ProductionSkillRegistry::new(if tmp_db.exists() {
            &tmp_db
        } else {
            std::path::Path::new(":memory:")
        })
        .expect("registry"),
    );
    let mut cfg = OpenClawConfig::default();
    cfg.enabled = true;
    cfg.image = "kria/openclaw-substrate:latest".to_string();
    let pool = Arc::new(ContainerPool::new(cfg).await.expect("pool"));
    let runtime: Arc<dyn SkillRuntime> = Arc::new(DockerRuntime::new(pool.clone()));
    let openclaw = OpenClawProvider::new(oc_registry, runtime);

    let stub = format!(
        "{}/tests/fixtures/mcp_stub_server.js",
        env!("CARGO_MANIFEST_DIR")
    );
    let mcp = McpProvider::connect("mcp:stub", "node", &[stub])
        .await
        .expect("mcp");

    let embedder = Arc::new(MemoryEmbedder::load().expect("embedder"));
    let index = Arc::new(InMemoryFederatedIndex::new(embedder));
    let registry = Arc::new(ProviderRegistry::new(index));
    registry.register(Arc::new(openclaw));
    registry.register(Arc::new(mcp));
    let platform = CapabilityPlatform::new(registry);
    platform.refresh().await;

    // (provider, capability, args, substring-that-must-appear-in-result)
    let battery: Vec<(&str, &str, serde_json::Value, &str)> = vec![
        (
            "openclaw",
            "oc_calculator",
            serde_json::json!({"expression": "7 * 6"}),
            "42",
        ),
        (
            "openclaw",
            "oc_text_tool",
            serde_json::json!({"text": "Hello", "op": "upper"}),
            "HELLO",
        ),
        (
            "openclaw",
            "oc_json_tool",
            serde_json::json!({"json": "{\"b\":2,\"a\":1}", "mode": "minify"}),
            "\\\"b\\\":2",
        ),
        (
            "openclaw",
            "oc_regex_tool",
            serde_json::json!({"text": "a1b2c3", "pattern": "[0-9]", "mode": "match"}),
            "1",
        ),
        (
            "openclaw",
            "oc_hash_tool",
            serde_json::json!({"text": "kria", "algorithm": "sha256"}),
            "",
        ),
        (
            "openclaw",
            "oc_csv_tool",
            serde_json::json!({"csv": "a,b\n1,2", "mode": "to_json"}),
            "1",
        ),
        (
            "openclaw",
            "oc_markdown_tool",
            serde_json::json!({"markdown": "# Title"}),
            "Title",
        ),
        (
            "mcp:stub",
            "reverse_text",
            serde_json::json!({"text": "kria"}),
            "airk",
        ),
        (
            "mcp:stub",
            "word_count",
            serde_json::json!({"text": "one two three"}),
            "3",
        ),
    ];

    let mut passed = 0;
    for (provider, cap, args, expect) in &battery {
        match platform.execute(req(provider, cap, args.clone())).await {
            Ok(CapabilityOutcome::Value(v)) => {
                let s = v.to_string();
                let ok = expect.is_empty() || s.contains(expect);
                eprintln!(
                    "  [{}] {}/{} -> {}",
                    if ok { "PASS" } else { "FAIL" },
                    provider,
                    cap,
                    s
                );
                assert!(ok, "{provider}/{cap} expected '{expect}' in {s}");
                passed += 1;
            }
            Ok(other) => panic!("{provider}/{cap}: unexpected outcome {other:?}"),
            Err(e) => panic!("{provider}/{cap}: execution error {e}"),
        }
    }
    eprintln!("battery: {passed}/{} passed", battery.len());
    assert_eq!(passed, battery.len());

    pool.shutdown().await.expect("shutdown");
    let _ = std::fs::remove_file(&tmp_db);
}
