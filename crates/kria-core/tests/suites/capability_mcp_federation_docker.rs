//! Milestone 6 real validation: **multi-provider federation**.
//!
//! Two DIFFERENT providers — OpenClaw (real Docker) and a plain MCP stdio server
//! (real `node` process) — are registered behind the SAME provider-neutral
//! boundary and federated into one index. This proves the architecture is not
//! overfit to OpenClaw: a standards-compliant MCP server is a first-class
//! provider with zero KRIA-core change, discovered + executed through the exact
//! same `CapabilityPlatform` path.
//!
//! Gated on `KRIA_CPP_DOCKER=1` (needs Docker + substrate image + skills.db +
//! `node`). Non-destructive (copies skills.db). Run:
//!
//! ```bash
//! KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_mcp_federation_docker -- --nocapture
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
async fn openclaw_and_mcp_providers_federate_and_execute() {
    if std::env::var("KRIA_CPP_DOCKER").is_err() {
        eprintln!("skipping: set KRIA_CPP_DOCKER=1 (needs Docker + substrate + skills.db + node)");
        return;
    }

    // ── Provider A: OpenClaw (real Docker) over a copy of the real skills DB.
    let real_db = dirs::home_dir().unwrap().join(".kria/skills.db");
    if !real_db.exists() {
        eprintln!("skipping: ~/.kria/skills.db not found");
        return;
    }
    let tmp_db: PathBuf =
        std::env::temp_dir().join(format!("kria_cpp_m6_{}.db", std::process::id()));
    std::fs::copy(&real_db, &tmp_db).expect("copy skills.db");
    let oc_registry = Arc::new(ProductionSkillRegistry::new(&tmp_db).expect("registry"));
    let mut cfg = OpenClawConfig::default();
    cfg.enabled = true;
    cfg.image = "kria/openclaw-substrate:latest".to_string();
    let pool = Arc::new(ContainerPool::new(cfg).await.expect("pool"));
    let runtime: Arc<dyn SkillRuntime> = Arc::new(DockerRuntime::new(pool.clone()));
    let openclaw = OpenClawProvider::new(oc_registry, runtime);

    // ── Provider B: a plain MCP stdio server (real node process).
    let stub = format!(
        "{}/tests/fixtures/mcp_stub_server.js",
        env!("CARGO_MANIFEST_DIR")
    );
    let mcp = McpProvider::connect("mcp:stub", "node", &[stub])
        .await
        .expect("start mcp stub");

    // ── One platform, both providers.
    let embedder = Arc::new(MemoryEmbedder::load().expect("embedder"));
    let index = Arc::new(InMemoryFederatedIndex::new(embedder));
    let registry = Arc::new(ProviderRegistry::new(index));
    registry.register(Arc::new(openclaw));
    registry.register(Arc::new(mcp));
    // M8: attach the observability event bus.
    let bus = Arc::new(kria_core::capability::events::CapabilityEventBus::new(128));
    let mut events_rx = bus.subscribe();
    let platform = CapabilityPlatform::new(registry).with_events(bus.clone());

    let report = platform.refresh().await;
    eprintln!(
        "refresh: {} descriptors, {} healthy providers",
        report.total_descriptors,
        report.healthy_count()
    );
    assert_eq!(report.healthy_count(), 2, "both providers must federate");
    assert!(
        report.total_descriptors >= 5,
        "3 openclaw + 2 mcp descriptors expected, got {}",
        report.total_descriptors
    );

    // ── Discovery routes to the right provider by goal (no provider named).
    let rev = platform
        .discover("reverse the characters of this text string", 5)
        .unwrap();
    eprintln!(
        "reverse discovery top: {:?}",
        rev.first()
            .map(|h| (&h.descriptor.provider_id, &h.descriptor.capability_id))
    );
    assert_eq!(rev[0].descriptor.provider_id, "mcp:stub");
    assert_eq!(rev[0].descriptor.capability_id, "reverse_text");

    let arith = platform
        .discover("evaluate this arithmetic expression", 5)
        .unwrap();
    assert_eq!(arith[0].descriptor.provider_id, "openclaw");

    // ── Execute a capability from EACH provider through the same platform.
    // MCP:
    let mcp_out = platform
        .execute(req(
            "mcp:stub",
            "reverse_text",
            serde_json::json!({ "text": "capability" }),
        ))
        .await
        .expect("mcp execute");
    match mcp_out {
        CapabilityOutcome::Value(v) => {
            eprintln!("mcp reverse_text result: {v}");
            assert_eq!(v.as_str(), Some("ytilibapac"));
        }
        other => panic!("expected Value, got {other:?}"),
    }

    // OpenClaw (varied prompt — not 3+3):
    let oc_out = platform
        .execute(req(
            "openclaw",
            "oc_calculator",
            serde_json::json!({ "expression": "12 * 12" }),
        ))
        .await
        .expect("openclaw execute");
    match oc_out {
        CapabilityOutcome::Value(v) => {
            eprintln!("openclaw calculator result: {v}");
            assert!(v.to_string().contains("144"), "12*12 must be 144");
        }
        other => panic!("expected Value, got {other:?}"),
    }

    // ── M8: observability — the real executions above emitted execute events.
    let mut execute_events = 0;
    while let Ok(ev) = events_rx.try_recv() {
        if matches!(ev.stage, kria_core::capability::Stage::Execute) {
            execute_events += 1;
            eprintln!(
                "event: {} {}/{:?} {}",
                ev.stage.as_str(),
                ev.provider_id,
                ev.capability_id,
                ev.outcome.as_str()
            );
        }
    }
    assert!(
        execute_events >= 4,
        "expected ≥4 execute events (2 runs × started+terminal), got {execute_events}"
    );

    // ── M7: provider conformance for BOTH real providers (SDK harness).
    use kria_core::capability::conformance::run_conformance;
    let reg = platform.registry();
    for pid in ["openclaw", "mcp:stub"] {
        let provider = reg.get(pid).expect("provider registered");
        let report = run_conformance(provider.as_ref()).await;
        eprintln!(
            "conformance[{pid}]: passed={} failures={:?}",
            report.passed(),
            report.failures()
        );
        assert!(
            report.passed(),
            "provider '{pid}' must pass conformance: {:?}",
            report.failures()
        );
    }

    // ── Cleanup + leak baseline.
    pool.shutdown().await.expect("pool shutdown");
    let _ = std::fs::remove_file(&tmp_db);
}
