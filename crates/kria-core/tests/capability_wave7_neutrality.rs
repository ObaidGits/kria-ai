//! Wave 7 — Provider Neutrality proof.
//!
//! Proves the Brain's acquisition → trust → CKB → execution → upgrade → removal
//! path is genuinely provider-neutral by driving the FULL lifecycle through a
//! brand-new, non-Docker, non-OpenClaw provider (`LocalFsProvider`) added with
//! ZERO kria-core change — through the identical `CapabilityPlatform` API. If the
//! Brain were coupled to OpenClaw, none of this would work.

use std::sync::Arc;
use std::time::Duration;

use kria_core::capability::acl::local_fs::{write_source_catalog, LocalFsProvider, LocalManifest};
use kria_core::capability::error::CapError;
use kria_core::capability::events::CapabilityEventBus;
use kria_core::capability::index::{Embedder, InMemoryFederatedIndex};
use kria_core::capability::intelligence::{
    CapabilityKnowledge, CatalogRanker, CatalogRankingPolicy, DefaultLifecycleManager,
    LifecycleManager, SqliteCapabilityKnowledge,
};
use kria_core::capability::platform::CapabilityPlatform;
use kria_core::capability::provider::{CapabilityOutcome, CapabilityRequest, RequestContext};
use kria_core::capability::registry::ProviderRegistry;

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
        "hash"
    }
}

fn manifest(id: &str, name: &str, desc: &str, op: &str, version: &str) -> LocalManifest {
    LocalManifest {
        capability_id: id.into(),
        name: name.into(),
        description: desc.into(),
        version: version.into(),
        operation: op.into(),
        trust_tier: Some("community".into()),
        dependencies: vec![],
    }
}

fn platform_with_localfs(
    dir: &std::path::Path,
    ckb: Arc<SqliteCapabilityKnowledge>,
    bus: Arc<CapabilityEventBus>,
) -> Arc<CapabilityPlatform> {
    let source = dir.join("source");
    let store = dir.join("store");
    write_source_catalog(
        &source,
        &[
            manifest(
                "reverser",
                "Text Reverser",
                "reverse a string of text",
                "reverse",
                "1.0.0",
            ),
            manifest(
                "upper",
                "Uppercaser",
                "convert text to uppercase letters",
                "upper",
                "1.0.0",
            ),
            manifest(
                "b64",
                "Base64 Encoder",
                "encode text as base64",
                "base64_encode",
                "1.0.0",
            ),
        ],
    )
    .unwrap();
    let provider = LocalFsProvider::new("localfs", &source, &store).unwrap();
    let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmbedder {
        dim: 128,
    })));
    let registry = ProviderRegistry::new(index);
    registry.register(Arc::new(provider));
    Arc::new(
        CapabilityPlatform::new(Arc::new(registry))
            .with_events(bus)
            .with_knowledge(ckb)
            .with_marketplace_v2(
                CatalogRanker::new(CatalogRankingPolicy::default()),
                Duration::from_secs(300),
            ),
    )
}

#[tokio::test]
async fn second_provider_full_lifecycle_through_neutral_brain() {
    let tmp = std::env::temp_dir().join(format!("kria_w7_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let ckb = Arc::new(SqliteCapabilityKnowledge::in_memory().unwrap());
    let bus = Arc::new(CapabilityEventBus::new(512));
    let platform = platform_with_localfs(&tmp, ckb.clone(), bus);
    platform.refresh().await;

    // 1) DISCOVER/RANK: the Brain ranks the local provider's catalog.
    let recs = platform
        .recommend("reverse a string", 5)
        .await
        .expect("recommend");
    assert!(
        !recs.is_empty(),
        "local catalog must be ranked by the Brain"
    );
    assert_eq!(recs[0].descriptor.provider_id, "localfs");

    // 2) ACQUIRE: Brain selects + installs the chosen capability + CKB + trust gate.
    let installed = platform
        .acquire_for_goal("reverse a string")
        .await
        .expect("acquire");
    assert_eq!(installed.provider_id, "localfs");
    let (pid, cid) = (
        installed.provider_id.clone(),
        installed.capability_id.clone(),
    );
    assert!(!platform.is_quarantined(&pid, &cid));
    assert!(ckb
        .list_installed()
        .await
        .unwrap()
        .iter()
        .any(|d| d.capability_id == cid));

    // 3) EXECUTE: the same neutral platform.execute drives the second provider —
    //    real transform, real output (no OpenClaw, no Docker).
    platform.refresh().await;
    let out = platform
        .execute(CapabilityRequest {
            provider_id: pid.clone(),
            capability_id: cid.clone(),
            args: serde_json::json!({ "text": "hello" }),
            context: RequestContext::new(),
            granted_effects: vec![],
        })
        .await
        .expect("execute");
    match out {
        CapabilityOutcome::Value(v) => {
            assert_eq!(v.get("result").and_then(|x| x.as_str()), Some("olleh"))
        }
        other => panic!("expected value, got {other:?}"),
    }

    // 4) UPGRADE: bump the source version → re-acquire the SPECIFIC installed
    //    capability by id (the LifecycleManager/upgrade path — distinct from a
    //    fresh goal-based acquire, which native-first-filters installed items).
    write_source_catalog(
        &tmp.join("source"),
        &[manifest(
            "reverser",
            "Text Reverser",
            "reverse a string of text",
            "reverse",
            "2.0.0",
        )],
    )
    .unwrap();
    platform.invalidate_catalog_cache(None);
    let provider = platform
        .registry()
        .get("localfs")
        .expect("provider present");
    let upgrade_req = kria_core::capability::provider::AcquireRequest {
        capability_tag: cid.clone(),
        hint: None,
        capability_id: Some(cid.clone()),
        proposed_graph: None,
        context: RequestContext::new(),
    };
    let upgraded = provider
        .acquire(&upgrade_req)
        .await
        .expect("upgrade re-acquire");
    assert_eq!(
        upgraded.version, "2.0.0",
        "upgrade must install the newer version"
    );
    // The installed descriptor now reflects the new version.
    platform.refresh().await;
    let now = platform
        .descriptor("localfs", &cid)
        .unwrap()
        .expect("installed");
    assert_eq!(now.version, "2.0.0");

    // 5) REMOVE via the neutral registry/provider path.
    let provider = platform
        .registry()
        .get("localfs")
        .expect("provider present");
    provider.remove(&cid).await.expect("remove");
    platform.refresh().await;
    let after = platform.discover("", 100).unwrap();
    assert!(
        !after.iter().any(|s| s.descriptor.capability_id == cid),
        "removed capability must disappear from discovery"
    );
}

#[tokio::test]
async fn dual_provider_identical_brain_artifacts() {
    // Two DIFFERENT providers acquired through the SAME neutral pipeline must
    // each produce a Decision Record + CKB install + the same event shape —
    // proving the Brain path is identical regardless of provider (Stage 5).
    use kria_core::capability::events::{Outcome, Stage};

    let tmp = std::env::temp_dir().join(format!("kria_w7dp_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    write_source_catalog(
        &tmp.join("a/source"),
        &[manifest("rev", "Rev", "reverse text", "reverse", "1.0.0")],
    )
    .unwrap();
    write_source_catalog(
        &tmp.join("b/source"),
        &[manifest("up", "Up", "uppercase text", "upper", "1.0.0")],
    )
    .unwrap();
    let pa = LocalFsProvider::new("prov-a", tmp.join("a/source"), tmp.join("a/store")).unwrap();
    let pb = LocalFsProvider::new("prov-b", tmp.join("b/source"), tmp.join("b/store")).unwrap();
    let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmbedder {
        dim: 128,
    })));
    let registry = ProviderRegistry::new(index);
    registry.register(Arc::new(pa));
    registry.register(Arc::new(pb));
    let ckb = Arc::new(SqliteCapabilityKnowledge::in_memory().unwrap());
    let bus = Arc::new(CapabilityEventBus::new(512));
    let mut rx = bus.subscribe();
    let platform = Arc::new(
        CapabilityPlatform::new(Arc::new(registry))
            .with_events(bus)
            .with_knowledge(ckb.clone())
            .with_marketplace_v2(
                CatalogRanker::new(CatalogRankingPolicy::default()),
                Duration::from_secs(300),
            ),
    );
    platform.refresh().await;

    let a = platform
        .acquire_for_goal("reverse text")
        .await
        .expect("acquire a");
    let b = platform
        .acquire_for_goal("uppercase text")
        .await
        .expect("acquire b");
    assert_ne!(a.provider_id, b.provider_id, "two distinct providers");

    // Both installs recorded in the ONE neutral CKB.
    let known = ckb.list_installed().await.unwrap();
    assert!(known.iter().any(|d| d.provider_id == "prov-a"));
    assert!(known.iter().any(|d| d.provider_id == "prov-b"));

    // Identical event shape per provider: each emitted Rank + Acquire.Ok + Learn.
    let mut per_provider: std::collections::HashMap<String, Vec<(Stage, Outcome)>> =
        Default::default();
    while let Ok(ev) = rx.try_recv() {
        per_provider
            .entry(ev.provider_id)
            .or_default()
            .push((ev.stage, ev.outcome));
    }
    for pid in ["prov-a", "prov-b"] {
        let evs = per_provider
            .get(pid)
            .unwrap_or_else(|| panic!("no events for {pid}"));
        assert!(
            evs.contains(&(Stage::Acquire, Outcome::Ok)),
            "{pid} missing Acquire.Ok: {evs:?}"
        );
        assert!(
            evs.contains(&(Stage::Learn, Outcome::Ok)),
            "{pid} missing Learn.Ok"
        );
    }
}

#[tokio::test]
async fn brain_does_not_branch_on_provider_identity() {
    // Two DISTINCT provider ids backed by the same neutral adapter type: the
    // Brain must treat them identically (no id-based special-casing).
    let tmp = std::env::temp_dir().join(format!("kria_w7b_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let src_a = tmp.join("a/source");
    let src_b = tmp.join("b/source");
    write_source_catalog(
        &src_a,
        &[manifest("rev", "Rev", "reverse text", "reverse", "1.0.0")],
    )
    .unwrap();
    write_source_catalog(
        &src_b,
        &[manifest("up", "Up", "uppercase text", "upper", "1.0.0")],
    )
    .unwrap();
    let pa = LocalFsProvider::new("provider-alpha", &src_a, tmp.join("a/store")).unwrap();
    let pb = LocalFsProvider::new("provider-beta", &src_b, tmp.join("b/store")).unwrap();
    let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmbedder {
        dim: 128,
    })));
    let registry = ProviderRegistry::new(index);
    registry.register(Arc::new(pa));
    registry.register(Arc::new(pb));
    let ckb = Arc::new(SqliteCapabilityKnowledge::in_memory().unwrap());
    let platform = Arc::new(
        CapabilityPlatform::new(Arc::new(registry))
            .with_knowledge(ckb)
            .with_marketplace_v2(
                CatalogRanker::new(CatalogRankingPolicy::default()),
                Duration::from_secs(300),
            ),
    );
    platform.refresh().await;

    // Acquire from each — the Brain selects by evidence, not provider name.
    let a = platform
        .acquire_for_goal("reverse text")
        .await
        .expect("acquire alpha");
    let b = platform
        .acquire_for_goal("uppercase text")
        .await
        .expect("acquire beta");
    assert_eq!(a.provider_id, "provider-alpha");
    assert_eq!(b.provider_id, "provider-beta");

    // Execute both through the identical neutral path.
    for (pid, cid, input, expect) in [
        ("provider-alpha", "rev", "abc", "cba"),
        ("provider-beta", "up", "abc", "ABC"),
    ] {
        platform.refresh().await;
        let out = platform
            .execute(CapabilityRequest {
                provider_id: pid.into(),
                capability_id: cid.into(),
                args: serde_json::json!({ "text": input }),
                context: RequestContext::new(),
                granted_effects: vec![],
            })
            .await
            .expect("execute");
        if let CapabilityOutcome::Value(v) = out {
            assert_eq!(v.get("result").and_then(|x| x.as_str()), Some(expect));
        } else {
            panic!("expected value");
        }
    }
}

#[tokio::test]
async fn brain_lifecycle_manager_drives_second_provider_incl_upgrade() {
    // The Brain's DefaultLifecycleManager (acquire_verified → smoke → activate,
    // upgrade, rollback) must operate the second provider through the identical
    // neutral path — and upgrade must actually re-install the newer version
    // (regression guard for the installed-filter/upgrade bug).
    let tmp = std::env::temp_dir().join(format!("kria_w7lm_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let source = tmp.join("source");
    let store = tmp.join("store");
    write_source_catalog(
        &source,
        &[manifest(
            "rev",
            "Reverser",
            "reverse a string of text",
            "reverse",
            "1.0.0",
        )],
    )
    .unwrap();
    let provider = LocalFsProvider::new("localfs", &source, &store).unwrap();
    let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmbedder {
        dim: 128,
    })));
    let registry = ProviderRegistry::new(index);
    registry.register(Arc::new(provider));
    let ckb = Arc::new(SqliteCapabilityKnowledge::in_memory().unwrap());
    let platform = Arc::new(
        CapabilityPlatform::new(Arc::new(registry))
            .with_knowledge(ckb.clone())
            .with_marketplace_v2(
                CatalogRanker::new(CatalogRankingPolicy::default()),
                Duration::from_secs(300),
            ),
    );
    platform.refresh().await;

    let lifecycle = DefaultLifecycleManager::new(platform.clone()).with_knowledge(ckb.clone());

    // acquire_verified: install + smoke + activate through the Brain.
    let d = lifecycle
        .acquire_verified("reverse a string of text")
        .await
        .expect("acquire_verified");
    assert_eq!(d.provider_id, "localfs");
    let cid = d.capability_id.clone();

    // Bump source → upgrade must install v2 (the fixed path: re-acquire by id).
    write_source_catalog(
        &source,
        &[manifest(
            "rev",
            "Reverser",
            "reverse a string of text",
            "reverse",
            "2.0.0",
        )],
    )
    .unwrap();
    lifecycle.upgrade("localfs", &cid).await.expect("upgrade");
    platform.refresh().await;
    let now = platform
        .descriptor("localfs", &cid)
        .unwrap()
        .expect("installed");
    assert_eq!(
        now.version, "2.0.0",
        "Brain lifecycle upgrade must install the newer version"
    );

    // rollback: uninstall + purge — capability gone from discovery + CKB.
    lifecycle.rollback("localfs", &cid).await.expect("rollback");
    let after = platform.discover("", 100).unwrap();
    assert!(!after.iter().any(|s| s.descriptor.capability_id == cid));
    assert!(ckb
        .list_installed()
        .await
        .unwrap()
        .iter()
        .all(|x| x.capability_id != cid));
}
