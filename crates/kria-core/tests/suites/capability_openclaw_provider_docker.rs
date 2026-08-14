//! Milestone 2 real-Docker validation: OpenClaw executes through the CPP
//! `OpenClawProvider` adapter, proving OpenClaw now runs as a provider behind the
//! provider-neutral boundary (not via a bespoke openclaw path).
//!
//! Gated on `KRIA_CPP_DOCKER=1` (needs a running Docker daemon + the
//! `kria/openclaw-substrate:latest` image) so it is skipped in CI-only
//! environments. Run:
//!
//! ```bash
//! KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_openclaw_provider_docker -- --nocapture
//! ```

use std::path::Path;
use std::sync::Arc;

use kria_core::capability::acl::OpenClawProvider;
use kria_core::capability::protocol::ClientCapabilities;
use kria_core::capability::provider::{CapabilityRequest, RequestContext};
use kria_core::capability::{CapabilityOutcome, CapabilityProvider};
use kria_core::openclaw::config::OpenClawConfig;
use kria_core::openclaw::pool::ContainerPool;
use kria_core::openclaw::registry::ProductionSkillRegistry;
use kria_core::openclaw::runtime::{DockerRuntime, SkillRuntime};

/// Real end-to-end: negotiate → execute `oc_calculator` `3+3` → `6`, entirely
/// through the neutral `CapabilityProvider` trait.
#[tokio::test]
async fn openclaw_provider_executes_calculator_via_real_docker() {
    if std::env::var("KRIA_CPP_DOCKER").is_err() {
        eprintln!("skipping: set KRIA_CPP_DOCKER=1 (needs Docker + substrate image)");
        return;
    }

    // Real container pool against the real substrate image.
    let mut cfg = OpenClawConfig::default();
    cfg.enabled = true;
    cfg.image = "kria/openclaw-substrate:latest".to_string();
    let pool = Arc::new(
        ContainerPool::new(cfg)
            .await
            .expect("build real container pool"),
    );
    let runtime: Arc<dyn SkillRuntime> = Arc::new(DockerRuntime::new(pool.clone()));

    // A throwaway in-memory registry (describe() is not exercised here; execute
    // targets the baked `oc_calculator` skill directly).
    let registry =
        Arc::new(ProductionSkillRegistry::new(Path::new(":memory:")).expect("in-memory registry"));

    let provider = OpenClawProvider::new(registry, runtime);

    // Negotiation yields the mandatory facets.
    let session = provider
        .negotiate(&ClientCapabilities::default())
        .await
        .expect("negotiate");
    assert!(session.has_mandatory(), "mandatory facets must be agreed");

    // Execute the calculator through the neutral boundary.
    let req = CapabilityRequest {
        provider_id: "openclaw".to_string(),
        capability_id: "oc_calculator".to_string(),
        args: serde_json::json!({ "expression": "3+3" }),
        context: RequestContext::new(),
        granted_effects: vec![],
    };
    let outcome = provider.execute(req).await.expect("execute ok");

    match outcome {
        CapabilityOutcome::Value(v) => {
            let rendered = v.to_string();
            eprintln!("oc_calculator result: {rendered}");
            assert!(
                rendered.contains('6'),
                "calculator 3+3 must yield 6, got: {rendered}"
            );
        }
        other => panic!("expected a Value outcome, got {other:?}"),
    }

    // Leak baseline: tear the warm pool down so no container survives the run.
    pool.shutdown().await.expect("pool shutdown");
}
