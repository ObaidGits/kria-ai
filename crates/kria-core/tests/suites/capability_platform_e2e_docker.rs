//! Milestone 3 real end-to-end validation: goal → federated discovery →
//! execution, entirely through the CPP `CapabilityPlatform`, with a real
//! `OpenClawProvider` describing the **real** skills registry and executing on
//! **real** Docker.
//!
//! Non-destructive: the user's `~/.kria/skills.db` is COPIED to a temp file; the
//! test never mutates the real database.
//!
//! Gated on `KRIA_CPP_DOCKER=1` (needs Docker + `kria/openclaw-substrate:latest`
//! + a populated `~/.kria/skills.db`). Run:
//!
//! ```bash
//! KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_platform_e2e_docker -- --nocapture
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use kria_core::capability::index::{InMemoryFederatedIndex, MemoryEmbedder};
use kria_core::capability::platform::CapabilityPlatform;
use kria_core::capability::provider::{CapabilityRequest, RequestContext};
use kria_core::capability::registry::ProviderRegistry;
use kria_core::capability::{CapabilityOutcome, OpenClawProvider};
use kria_core::openclaw::config::OpenClawConfig;
use kria_core::openclaw::pool::ContainerPool;
use kria_core::openclaw::registry::ProductionSkillRegistry;
use kria_core::openclaw::runtime::{DockerRuntime, SkillRuntime};

#[tokio::test]
async fn platform_discovers_and_executes_calculator_end_to_end() {
    if std::env::var("KRIA_CPP_DOCKER").is_err() {
        eprintln!("skipping: set KRIA_CPP_DOCKER=1 (needs Docker + substrate image + skills.db)");
        return;
    }

    // Non-destructive copy of the real skills DB into a temp path.
    let real_db = dirs::home_dir().unwrap().join(".kria/skills.db");
    if !real_db.exists() {
        eprintln!("skipping: ~/.kria/skills.db not found");
        return;
    }
    let tmp_db: PathBuf =
        std::env::temp_dir().join(format!("kria_cpp_skills_{}.db", std::process::id()));
    std::fs::copy(&real_db, &tmp_db).expect("copy skills.db");

    // Real registry (read from the copy) + real Docker runtime.
    let registry = Arc::new(ProductionSkillRegistry::new(&tmp_db).expect("open skills registry"));
    let enabled = registry.get_enabled_skills().expect("enabled skills");
    eprintln!(
        "enabled skills in DB: {:?}",
        enabled.iter().map(|s| &s.skill_id).collect::<Vec<_>>()
    );

    let mut cfg = OpenClawConfig::default();
    cfg.enabled = true;
    cfg.image = "kria/openclaw-substrate:latest".to_string();
    let pool = Arc::new(ContainerPool::new(cfg).await.expect("container pool"));
    let runtime: Arc<dyn SkillRuntime> = Arc::new(DockerRuntime::new(pool.clone()));

    // Wire the CPP platform: OpenClawProvider behind the neutral boundary.
    let provider = OpenClawProvider::new(registry, runtime);
    let embedder = Arc::new(MemoryEmbedder::load().expect("embedder"));
    let index = Arc::new(InMemoryFederatedIndex::new(embedder));
    let provider_registry = Arc::new(ProviderRegistry::new(index));
    provider_registry.register(Arc::new(provider));
    let platform = CapabilityPlatform::new(provider_registry);

    // Refresh federates the provider's descriptors into the index.
    let report = platform.refresh().await;
    eprintln!(
        "refresh: {} descriptors, {} healthy providers",
        report.total_descriptors,
        report.healthy_count()
    );
    assert!(
        report.total_descriptors > 0,
        "OpenClaw must describe at least one enabled skill"
    );

    // Discover for an arithmetic goal — the calculator should rank at/near top.
    let hits = platform
        .discover("evaluate the arithmetic expression 3 plus 3", 5)
        .unwrap();
    eprintln!(
        "discovery top: {:?}",
        hits.iter()
            .map(|h| (&h.descriptor.capability_id, h.score))
            .collect::<Vec<_>>()
    );
    let calc = hits
        .iter()
        .find(|h| h.descriptor.capability_id == "oc_calculator");
    assert!(
        calc.is_some(),
        "oc_calculator must be discoverable for an arithmetic goal"
    );

    // Execute the discovered calculator through the platform on real Docker.
    let out = platform
        .execute(CapabilityRequest {
            provider_id: "openclaw".to_string(),
            capability_id: "oc_calculator".to_string(),
            args: serde_json::json!({ "expression": "3+3" }),
            context: RequestContext::new(),
            granted_effects: vec![],
        })
        .await
        .expect("execute");

    match out {
        CapabilityOutcome::Value(v) => {
            eprintln!("execute result: {v}");
            assert!(v.to_string().contains('6'), "3+3 must be 6");
        }
        other => panic!("expected Value, got {other:?}"),
    }

    // Permission gate over REAL descriptors (M4): the neutral engine decides
    // purely from each descriptor's effects — GREEN calculator never prompts;
    // any elevated skill (e.g. network) requires approval.
    use kria_core::capability::grants::GrantStore;
    use kria_core::capability::permission::{
        AuthorizeRequest, DefaultPermissionEngine, PermissionEngine, PermissionTier,
    };
    let engine = DefaultPermissionEngine;
    let grants = GrantStore::in_memory().expect("grants");
    let all = platform.discover("", 50).unwrap_or_default();
    for sd in &all {
        let req = AuthorizeRequest::from_descriptor(&sd.descriptor, Some("sess".into()), None);
        let decision = engine.authorize(&req, &grants);
        eprintln!(
            "permission[{}] effects={:?} -> {:?}",
            sd.descriptor.capability_id, sd.descriptor.effects.classes, decision
        );
        if sd.descriptor.capability_id == "oc_calculator" {
            assert!(
                matches!(
                    decision,
                    kria_core::capability::PermissionDecision::Allow {
                        tier: PermissionTier::NeverAsk,
                        ..
                    }
                ),
                "calculator (GREEN/reversible) must be NeverAsk-allowed"
            );
        }
    }

    // Leak baseline + cleanup.
    pool.shutdown().await.expect("pool shutdown");
    let _ = std::fs::remove_file(&tmp_db);
}
