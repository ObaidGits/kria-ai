//! Wave 6 — REAL Brain-owned acquisition pipeline validation.
//!
//! Drives the ACTUAL production path (no FakeProvider): real OpenClaw provider
//! against the REAL marketplace index (`ObaidGits/kria-skills`), through the real
//! `CapabilityPlatform` with marketplace_v2 (CatalogRanker + TTL cache + trust
//! gate + quarantine), a real SQLite CKB, and the real event bus. Proves:
//! catalog → ClawHub-schema mapping → Brain ranking/selection → acquire the
//! Brain-chosen capability → trust gate → CKB install+outcome → events.
//!
//! Gated on `KRIA_CPP_NET=1` (network to raw.githubusercontent.com). Run:
//! ```bash
//! KRIA_CPP_NET=1 cargo test -p kria-core --test capability_wave6_pipeline -- --nocapture
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use kria_core::capability::acl::openclaw::OpenClawProvider;
use kria_core::capability::error::CapError;
use kria_core::capability::events::{CapabilityEventBus, Stage};
use kria_core::capability::index::{Embedder, InMemoryFederatedIndex};
use kria_core::capability::intelligence::{
    CapabilityKnowledge, CatalogRanker, CatalogRankingPolicy, SqliteCapabilityKnowledge,
    TrustPolicy,
};
use kria_core::capability::platform::CapabilityPlatform;
use kria_core::capability::protocol::ClientCapabilities;
use kria_core::capability::provider::CapabilityProvider;
use kria_core::capability::provider::{CapabilityRequest, RequestContext};
use kria_core::capability::registry::ProviderRegistry;
use kria_core::infra::isolation::ToolResult;
use kria_core::openclaw::audit::AuditLedger;
use kria_core::openclaw::registry::ProductionSkillRegistry;
use kria_core::openclaw::runtime::{LaunchSpec, RuntimeContext, RuntimeKind, SkillRuntime};

struct NullRuntime;

#[async_trait]
impl SkillRuntime for NullRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Docker
    }
    async fn execute(&self, _spec: LaunchSpec, _ctx: RuntimeContext) -> ToolResult {
        ToolResult::err("null runtime")
    }
}

struct HashEmbedder {
    dim: usize,
}
impl Embedder for HashEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, CapError> {
        let mut v = vec![0.0f32; self.dim];
        for tok in text.to_lowercase().split_whitespace() {
            let mut h: u64 = 1469598103934665603;
            for b in tok.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(1099511628211);
            }
            v[(h as usize) % self.dim] += 1.0;
        }
        Ok(v)
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn model_id(&self) -> &str {
        "hash-test"
    }
}

#[tokio::test]
async fn wave6_brain_owned_pipeline_real_marketplace() {
    if std::env::var("KRIA_CPP_NET").is_err() {
        eprintln!("skipping: set KRIA_CPP_NET=1 (needs network to the skills repo)");
        return;
    }

    let dir = std::env::temp_dir().join(format!("kria_wave6_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = dir.join("store");
    std::fs::create_dir_all(&store).unwrap();
    let skill_registry =
        Arc::new(ProductionSkillRegistry::new(&dir.join("skills.db")).expect("registry"));
    let audit = Arc::new(
        AuditLedger::open(
            &dir.join("skills.db"),
            b"kria-wave6-audit-key-0001".to_vec(),
        )
        .expect("audit"),
    );
    let runtime: Arc<dyn SkillRuntime> = Arc::new(NullRuntime);

    let provider = OpenClawProvider::new(skill_registry.clone(), runtime).with_lifecycle(
        "https://raw.githubusercontent.com/ObaidGits/kria-skills/refs/heads/main/index.json",
        vec!["raw.githubusercontent.com".to_string()],
        audit,
        store,
    );
    // Sanity: lifecycle advertised.
    let session = provider
        .negotiate(&ClientCapabilities::default())
        .await
        .unwrap();
    assert!(session.supports_lifecycle());

    // Real platform: registry + index + marketplace_v2 + CKB + events.
    let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmbedder {
        dim: 128,
    })));
    let registry = ProviderRegistry::new(index);
    registry.register(Arc::new(provider));
    let ckb = Arc::new(SqliteCapabilityKnowledge::in_memory().expect("ckb"));
    let bus = Arc::new(CapabilityEventBus::new(1024));
    let mut rx = bus.subscribe();
    let platform = Arc::new(
        CapabilityPlatform::new(Arc::new(registry))
            .with_events(bus.clone())
            .with_knowledge(ckb.clone())
            .with_marketplace_v2(
                CatalogRanker::new(CatalogRankingPolicy::default()),
                std::time::Duration::from_secs(300),
            ),
    );
    platform.refresh().await;

    // 1) recommend() must return real catalog entries mapped through the ClawHub
    //    schema (capability_id == slug, installed=false).
    let recs = platform
        .recommend("execute python code in a sandbox", 8)
        .await
        .expect("recommend");
    assert!(!recs.is_empty(), "real marketplace catalog must be ranked");
    assert!(
        recs.iter().all(|r| r
            .descriptor
            .extensions
            .get("installed")
            .and_then(|v| v.as_bool())
            == Some(false)),
        "catalog entries must be flagged not-installed"
    );
    eprintln!(
        "recommend top: {}/{} score {:.3}",
        recs[0].descriptor.provider_id, recs[0].descriptor.capability_id, recs[0].score
    );

    // 2) Brain-owned acquisition: rank → select → acquire the chosen capability
    //    → trust gate → CKB. Community tier ⇒ trusted ⇒ activated.
    let installed = platform
        .acquire_for_goal("execute python code in a sandbox")
        .await
        .expect("acquisition must install a trusted capability");
    eprintln!(
        "acquired: {}/{}",
        installed.provider_id, installed.capability_id
    );
    assert!(!platform.is_quarantined(&installed.provider_id, &installed.capability_id));

    // 3) CKB persisted the install + a successful outcome (learning).
    let known = ckb.list_installed().await.expect("list_installed");
    assert!(
        known
            .iter()
            .any(|d| d.capability_id == installed.capability_id),
        "CKB must record the installed capability"
    );
    let rate = ckb
        .success_rate(&installed.provider_id, &installed.capability_id)
        .await;
    assert!(
        rate > 0.5,
        "successful acquisition must lift learned success rate, got {rate}"
    );

    // 4) Events: Rank + Acquire emitted on the real bus.
    let mut stages = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        stages.push(ev.stage);
    }
    assert!(
        stages.contains(&Stage::Rank),
        "Rank event must fire: {stages:?}"
    );
    assert!(
        stages.contains(&Stage::Acquire),
        "Acquire event must fire: {stages:?}"
    );

    // 5) Post-install: the just-installed capability must NOT be recommended
    //    again (installed-filter + catalog-cache invalidation, Phase 8).
    platform.refresh().await;
    let recs2 = platform
        .recommend("execute python code in a sandbox", 8)
        .await
        .expect("recommend");
    assert!(
        !recs2
            .iter()
            .any(|r| r.descriptor.capability_id == installed.capability_id),
        "an installed capability must not appear as an installable recommendation"
    );

    // Cleanup: uninstall the real skill.
    let _ = skill_registry.uninstall(&installed.capability_id);
    let _ = std::fs::remove_dir_all(&dir);
}

/// REAL failure path: a strict trust policy (require_signature) must quarantine
/// the real (unsigned, community) marketplace skill and BLOCK activation + any
/// subsequent execution — proving the Brain trust gate + quarantine are real.
#[tokio::test]
async fn wave6_strict_trust_quarantines_real_unsigned_skill() {
    if std::env::var("KRIA_CPP_NET").is_err() {
        eprintln!("skipping: set KRIA_CPP_NET=1 (needs network to the skills repo)");
        return;
    }

    let dir = std::env::temp_dir().join(format!("kria_wave6q_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = dir.join("store");
    std::fs::create_dir_all(&store).unwrap();
    let skill_registry =
        Arc::new(ProductionSkillRegistry::new(&dir.join("skills.db")).expect("registry"));
    let audit = Arc::new(
        AuditLedger::open(
            &dir.join("skills.db"),
            b"kria-wave6q-audit-key-0001".to_vec(),
        )
        .expect("audit"),
    );
    let runtime: Arc<dyn SkillRuntime> = Arc::new(NullRuntime);
    let provider = OpenClawProvider::new(skill_registry.clone(), runtime).with_lifecycle(
        "https://raw.githubusercontent.com/ObaidGits/kria-skills/refs/heads/main/index.json",
        vec!["raw.githubusercontent.com".to_string()],
        audit,
        store,
    );

    let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmbedder {
        dim: 128,
    })));
    let registry = ProviderRegistry::new(index);
    registry.register(Arc::new(provider));
    let ckb = Arc::new(SqliteCapabilityKnowledge::in_memory().expect("ckb"));
    let platform = Arc::new(
        CapabilityPlatform::new(Arc::new(registry))
            .with_knowledge(ckb.clone())
            .with_marketplace_v2(
                CatalogRanker::new(CatalogRankingPolicy::default()),
                std::time::Duration::from_secs(300),
            )
            // Strict: require trusted-or-higher tier (rank >= 3). The real
            // marketplace skill installs as Community (rank 2), so it MUST be
            // quarantined, not activated.
            .with_trust_policy(TrustPolicy {
                require_signature: false,
                min_tier_rank: 3,
            }),
    );
    platform.refresh().await;

    let recs = platform
        .recommend("execute python code in a sandbox", 8)
        .await
        .expect("recommend");
    assert!(!recs.is_empty());
    let target = recs[0].descriptor.capability_id.clone();

    let err = platform
        .acquire_for_goal("execute python code in a sandbox")
        .await
        .expect_err("strict trust policy must reject the unsigned skill");
    eprintln!("strict-trust rejection: {err}");
    assert!(matches!(err, CapError::Permission(_)), "got {err:?}");
    assert!(
        platform.is_quarantined("openclaw", &target),
        "the unsigned skill must be quarantined"
    );

    // A direct execute of the quarantined capability must be refused.
    let exec = platform
        .execute(CapabilityRequest {
            provider_id: "openclaw".into(),
            capability_id: target.clone(),
            args: serde_json::json!({}),
            context: RequestContext::new(),
            granted_effects: vec![],
        })
        .await;
    assert!(matches!(exec, Err(CapError::Permission(_))), "got {exec:?}");

    // Release restores executability path (no longer quarantined).
    assert!(platform.release_quarantine("openclaw", &target));
    assert!(!platform.is_quarantined("openclaw", &target));

    let _ = std::fs::remove_dir_all(&dir);
}
