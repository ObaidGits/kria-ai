//! Wave 10 — Continuous Discovery & Maintenance validation (neutral, real).
//!
//! Proves the background discovery engine drives the EXISTING evolution +
//! marketplace + CKB machinery on a schedule: monitoring (refresh), health-driven
//! proposals (evolution), discovery-driven proposals (marketplace recommend),
//! autonomy-gated auto-apply, durable persistence (restart recovery), dedup,
//! quiet-hours, and cancellation. Real on-disk SQLite CKB; no provider cognition.

use std::sync::Arc;

use async_trait::async_trait;
use kria_core::capability::descriptor::{CapabilityDescriptor, Effects, Reversibility};
use kria_core::capability::error::CapError;
use kria_core::capability::index::{Embedder, InMemoryFederatedIndex};
use kria_core::capability::intelligence::{
    AutonomyLevel, CapabilityKnowledge, CatalogRanker, CatalogRankingPolicy,
    ContinuousDiscoveryEngine, DiscoveryPolicy, EvolutionStore, ProposalStatus,
    SqliteCapabilityKnowledge,
};
use kria_core::capability::platform::CapabilityPlatform;
use kria_core::capability::protocol::{
    ClientCapabilities, FeatureSet, ProtocolSession, ProtocolVersion, ProviderHealth,
};
use kria_core::capability::provider::{CapabilityOutcome, CapabilityProvider, CapabilityRequest};
use kria_core::capability::registry::ProviderRegistry;

struct HashEmb;
impl Embedder for HashEmb {
    fn embed(&self, t: &str) -> Result<Vec<f32>, CapError> {
        let mut v = vec![0.0f32; 32];
        for (i, b) in t.bytes().enumerate() {
            v[i % 32] += b as f32;
        }
        Ok(v)
    }
    fn dim(&self) -> usize {
        32
    }
    fn model_id(&self) -> &str {
        "h"
    }
}

/// A provider that has ONE installed capability + offers a not-installed
/// marketplace candidate in the same family (so discovery can surface it).
struct MarketProvider;

fn cap(id: &str, family: &str, installed: bool, version: &str) -> CapabilityDescriptor {
    let mut d = CapabilityDescriptor::minimal(
        "market",
        id,
        id,
        "text transform capability",
        serde_json::json!({"type":"object","properties":{"text":{"type":"string"}}}),
    );
    d.version = version.into();
    d.effects = Effects {
        classes: vec!["read".into()],
        reversible: Reversibility::Reversible,
        idempotent: true,
        resource_class: Default::default(),
    };
    d.extensions
        .insert("family".into(), serde_json::Value::String(family.into()));
    d.extensions
        .insert("installed".into(), serde_json::Value::Bool(installed));
    d
}

#[async_trait]
impl CapabilityProvider for MarketProvider {
    fn provider_id(&self) -> &String {
        static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        ID.get_or_init(|| "market".to_string())
    }
    async fn negotiate(&self, client: &ClientCapabilities) -> Result<ProtocolSession, CapError> {
        Ok(client.negotiate(
            "market".to_string(),
            ProtocolVersion::CURRENT,
            FeatureSet::mandatory(),
            serde_json::Map::new(),
        ))
    }
    async fn describe(&self, _s: &ProtocolSession) -> Result<Vec<CapabilityDescriptor>, CapError> {
        // One installed capability.
        Ok(vec![cap("installed_ocr", "ocr", true, "1.0.0")])
    }
    async fn catalog(&self) -> Result<Vec<CapabilityDescriptor>, CapError> {
        // A not-yet-installed, in-family marketplace candidate.
        Ok(vec![cap("better_ocr", "ocr", false, "2.0.0")])
    }
    async fn execute(&self, _req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
        Ok(CapabilityOutcome::Value(serde_json::json!({"result":"ok"})))
    }
    async fn acquire(
        &self,
        req: &kria_core::capability::provider::AcquireRequest,
    ) -> Result<CapabilityDescriptor, CapError> {
        // Idempotent re-acquire (repair/upgrade): return the installed descriptor.
        let id = req.capability_id.as_deref().unwrap_or("installed_ocr");
        Ok(cap(id, "ocr", true, "1.0.0"))
    }
    async fn health(&self) -> ProviderHealth {
        ProviderHealth::Ready
    }
}

fn platform(dir: &std::path::Path, ckb: Arc<SqliteCapabilityKnowledge>) -> Arc<CapabilityPlatform> {
    let _ = dir;
    let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmb)));
    let registry = ProviderRegistry::new(index);
    registry.register(Arc::new(MarketProvider));
    Arc::new(
        CapabilityPlatform::new(Arc::new(registry))
            .with_knowledge(ckb.clone())
            .with_evolution_store(ckb.clone())
            .with_marketplace_v2(
                CatalogRanker::new(CatalogRankingPolicy::default()),
                std::time::Duration::from_secs(60),
            )
            .with_events(Arc::new(
                kria_core::capability::events::CapabilityEventBus::new(256),
            )),
    )
}

#[tokio::test]
async fn scan_produces_discovery_proposal_for_a_better_marketplace_candidate() {
    let dir = std::env::temp_dir().join(format!("kria_w10_disc_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ckb = Arc::new(SqliteCapabilityKnowledge::open(&dir.join("ckb.db")).unwrap());
    let plat = platform(&dir, ckb.clone());
    plat.refresh().await;

    let engine = ContinuousDiscoveryEngine::new(
        plat.clone(),
        DiscoveryPolicy::default(),
        AutonomyLevel::ProposeOnly,
    );
    let report = engine.scan_once().await;
    assert!(!report.skipped_quiet);
    assert!(report.providers_seen >= 1);
    assert!(
        report.discovery_proposals >= 1,
        "expected a discovery proposal for the better in-family candidate, got {report:?}"
    );

    // Durable: the proposal is persisted (restart recovery) — reopen the CKB.
    drop(plat);
    let ckb2 = Arc::new(SqliteCapabilityKnowledge::open(&dir.join("ckb.db")).unwrap());
    let pending = EvolutionStore::list_proposals(&*ckb2, Some(ProposalStatus::Pending))
        .await
        .unwrap();
    assert!(
        pending.iter().any(|p| p.capability_id == "installed_ocr"),
        "discovery proposal must survive restart (durable in CKB)"
    );
    // ProposeOnly ⇒ nothing auto-applied.
    assert_eq!(report.auto_applied, 0);
}

#[tokio::test]
async fn scan_is_idempotent_no_duplicate_proposals() {
    let dir = std::env::temp_dir().join(format!("kria_w10_dedup_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ckb = Arc::new(SqliteCapabilityKnowledge::open(&dir.join("ckb.db")).unwrap());
    let plat = platform(&dir, ckb.clone());
    plat.refresh().await;
    let engine = ContinuousDiscoveryEngine::new(
        plat.clone(),
        DiscoveryPolicy::default(),
        AutonomyLevel::ProposeOnly,
    );
    engine.scan_once().await;
    let after_first = EvolutionStore::list_proposals(&*ckb, Some(ProposalStatus::Pending))
        .await
        .unwrap()
        .len();
    engine.scan_once().await; // second scan must not duplicate
    let after_second = EvolutionStore::list_proposals(&*ckb, Some(ProposalStatus::Pending))
        .await
        .unwrap()
        .len();
    assert_eq!(
        after_first, after_second,
        "a repeat scan must not create duplicate proposals (dedup)"
    );
}

#[tokio::test]
async fn quiet_hours_skips_the_scan() {
    let dir = std::env::temp_dir().join(format!("kria_w10_quiet_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ckb = Arc::new(SqliteCapabilityKnowledge::open(&dir.join("ckb.db")).unwrap());
    let plat = platform(&dir, ckb.clone());
    plat.refresh().await;
    // Quiet window covering all 24h → always skip.
    let policy = DiscoveryPolicy {
        quiet_hours_utc: Some((0, 24)),
        ..Default::default()
    };
    let engine = ContinuousDiscoveryEngine::new(plat.clone(), policy, AutonomyLevel::ProposeOnly);
    let report = engine.scan_once().await;
    assert!(report.skipped_quiet);
    assert_eq!(report.discovery_proposals, 0);
    assert_eq!(
        EvolutionStore::list_proposals(&*ckb, Some(ProposalStatus::Pending))
            .await
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn status_reflects_scans() {
    let dir = std::env::temp_dir().join(format!("kria_w10_status_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ckb = Arc::new(SqliteCapabilityKnowledge::open(&dir.join("ckb.db")).unwrap());
    let plat = platform(&dir, ckb.clone());
    plat.refresh().await;
    let engine = ContinuousDiscoveryEngine::new(
        plat.clone(),
        DiscoveryPolicy::default(),
        AutonomyLevel::ProposeOnly,
    );
    assert_eq!(engine.status().total_scans, 0);
    engine.scan_once().await;
    let s = engine.status();
    assert_eq!(s.total_scans, 1);
    assert!(s.last_scan_at.is_some());
    assert!(s.next_scan_at.is_some());
}

#[tokio::test]
async fn background_loop_runs_and_cancels() {
    let dir = std::env::temp_dir().join(format!("kria_w10_bg_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ckb = Arc::new(SqliteCapabilityKnowledge::open(&dir.join("ckb.db")).unwrap());
    let plat = platform(&dir, ckb.clone());
    plat.refresh().await;
    // Tiny interval so the loop scans quickly in the test.
    let policy = DiscoveryPolicy {
        interval: std::time::Duration::from_millis(200),
        jitter_frac: 0.0,
        ..Default::default()
    };
    let engine = Arc::new(ContinuousDiscoveryEngine::new(
        plat.clone(),
        policy,
        AutonomyLevel::ProposeOnly,
    ));
    engine.clone().spawn();
    assert!(engine.status().running);
    // Wait for at least one scan to run.
    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    engine.cancel();
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let s = engine.status();
    assert!(s.total_scans >= 1, "background loop must have scanned");
    assert!(!s.running, "loop must stop after cancel");
}

#[tokio::test]
async fn full_auto_applies_non_elevated_repair_but_propose_only_does_not() {
    // A chronically-failing installed capability → health Critical → Repair
    // (non-elevated). ProposeOnly must NOT auto-apply; FullAuto must.
    async fn run(autonomy: AutonomyLevel) -> kria_core::capability::intelligence::DiscoveryReport {
        let dir = std::env::temp_dir().join(format!(
            "kria_w10_auto_{}_{}",
            std::process::id(),
            autonomy.as_str()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ckb = Arc::new(SqliteCapabilityKnowledge::open(&dir.join("ckb.db")).unwrap());
        let plat = platform(&dir, ckb.clone());
        plat.refresh().await;
        // Record the installed cap + drive it chronically-failing.
        ckb.record_install(&cap("installed_ocr", "ocr", true, "1.0.0"))
            .await
            .unwrap();
        for _ in 0..8 {
            ckb.record_outcome("market", "installed_ocr", false, Some(5), Some("boom"))
                .await
                .unwrap();
        }
        let engine =
            ContinuousDiscoveryEngine::new(plat.clone(), DiscoveryPolicy::default(), autonomy);
        engine.scan_once().await
    }

    let propose = run(AutonomyLevel::ProposeOnly).await;
    assert_eq!(propose.auto_applied, 0, "propose-only must not auto-apply");

    let auto = run(AutonomyLevel::FullAuto).await;
    assert!(
        auto.health_proposals >= 1,
        "a critical capability must yield a health proposal"
    );
    assert!(
        auto.auto_applied >= 1,
        "full-auto must auto-apply the non-elevated repair, got {auto:?}"
    );
}

/// A provider with a LARGE installed set + large marketplace catalog, to prove
/// discovery stays bounded (max_findings_per_scan) and fast under scale.
struct ScaleProvider {
    n: usize,
}
#[async_trait]
impl CapabilityProvider for ScaleProvider {
    fn provider_id(&self) -> &String {
        static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        ID.get_or_init(|| "scale".to_string())
    }
    async fn negotiate(&self, client: &ClientCapabilities) -> Result<ProtocolSession, CapError> {
        Ok(client.negotiate(
            "scale".to_string(),
            ProtocolVersion::CURRENT,
            FeatureSet::mandatory(),
            serde_json::Map::new(),
        ))
    }
    async fn describe(&self, _s: &ProtocolSession) -> Result<Vec<CapabilityDescriptor>, CapError> {
        Ok((0..self.n)
            .map(|i| {
                let mut d = cap(&format!("inst_{i}"), "ocr", true, "1.0.0");
                d.provider_id = "scale".into();
                d
            })
            .collect())
    }
    async fn catalog(&self) -> Result<Vec<CapabilityDescriptor>, CapError> {
        Ok((0..self.n)
            .map(|i| {
                let mut d = cap(&format!("cand_{i}"), "ocr", false, "2.0.0");
                d.provider_id = "scale".into();
                d
            })
            .collect())
    }
    async fn execute(&self, _req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
        Ok(CapabilityOutcome::Value(serde_json::json!({"result":"ok"})))
    }
    async fn health(&self) -> ProviderHealth {
        ProviderHealth::Ready
    }
}

#[tokio::test]
async fn scan_stays_bounded_and_fast_under_scale() {
    let dir = std::env::temp_dir().join(format!("kria_w10_scale_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ckb = Arc::new(SqliteCapabilityKnowledge::open(&dir.join("ckb.db")).unwrap());
    let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmb)));
    let registry = ProviderRegistry::new(index);
    registry.register(Arc::new(ScaleProvider { n: 300 }));
    let plat = Arc::new(
        CapabilityPlatform::new(Arc::new(registry))
            .with_knowledge(ckb.clone())
            .with_evolution_store(ckb.clone())
            .with_marketplace_v2(
                CatalogRanker::new(CatalogRankingPolicy::default()),
                std::time::Duration::from_secs(60),
            ),
    );
    plat.refresh().await;

    let policy = DiscoveryPolicy {
        max_findings_per_scan: 5,
        ..Default::default()
    };
    let engine = ContinuousDiscoveryEngine::new(plat.clone(), policy, AutonomyLevel::ProposeOnly);
    let t0 = std::time::Instant::now();
    let report = engine.scan_once().await;
    let ms = t0.elapsed().as_millis();
    eprintln!(
        "[W10 perf] scan over 300 installed + 300 catalog = {ms} ms, {} findings",
        report.discovery_proposals
    );
    // Findings are hard-bounded by the budget regardless of catalog size.
    assert!(
        report.discovery_proposals <= 5,
        "must respect max_findings_per_scan"
    );
    // Bounded scan should complete quickly (generous CI budget).
    assert!(ms < 5000, "scan latency {ms}ms exceeds budget under scale");
}

/// FAILURE INJECTION: an evolution store that errors on read → the scan records
/// a degraded condition (`consecutive_errors`), which drives the loop's backoff.
/// Proves the error/backoff path is real, not vestigial.
#[tokio::test]
async fn store_errors_are_recorded_as_degraded_for_backoff() {
    use kria_core::capability::intelligence::{
        CapabilityHealth, EvolutionProposal, EvolutionStore, ProposalStatus,
    };

    struct FailingStore;
    #[async_trait]
    impl EvolutionStore for FailingStore {
        async fn health_snapshots(&self) -> Result<Vec<CapabilityHealth>, CapError> {
            Err(CapError::Io("db down".into()))
        }
        async fn record_benchmark(
            &self,
            _p: &str,
            _c: &str,
            _ok: bool,
            _l: u64,
            _s: f32,
        ) -> Result<(), CapError> {
            Ok(())
        }
        async fn benchmark_score(&self, _p: &str, _c: &str) -> Option<f32> {
            None
        }
        async fn record_proposal(&self, _p: &EvolutionProposal) -> Result<(), CapError> {
            Ok(())
        }
        async fn list_proposals(
            &self,
            _s: Option<ProposalStatus>,
        ) -> Result<Vec<EvolutionProposal>, CapError> {
            Err(CapError::Io("db down".into()))
        }
        async fn set_proposal_status(&self, _id: &str, _s: ProposalStatus) -> Result<(), CapError> {
            Ok(())
        }
        async fn get_proposal(&self, _id: &str) -> Result<Option<EvolutionProposal>, CapError> {
            Ok(None)
        }
    }

    let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmb)));
    let registry = ProviderRegistry::new(index);
    registry.register(Arc::new(MarketProvider));
    let store: Arc<dyn EvolutionStore> = Arc::new(FailingStore);
    let plat = Arc::new(CapabilityPlatform::new(Arc::new(registry)).with_evolution_store(store));
    plat.refresh().await;
    let engine = ContinuousDiscoveryEngine::new(
        plat,
        DiscoveryPolicy::default(),
        AutonomyLevel::ProposeOnly,
    );
    engine.scan_once().await;
    let s = engine.status();
    assert_eq!(s.consecutive_errors, 1, "degraded store must be recorded");
    assert!(s.last_error.is_some());
    // A subsequent healthy-ish scan is still counted; error count is monotonic
    // per consecutive failing scans.
    engine.scan_once().await;
    assert_eq!(engine.status().consecutive_errors, 2);
}

/// SAFETY REGRESSION (BUG #1): a discovery `Replace` proposal references an
/// UNINSTALLED marketplace candidate; `evolution.apply(Replace)` only retires the
/// old capability. Under FullAuto this would retire-without-install → capability
/// GAP. Discovery must therefore NEVER auto-apply an elevated Replace/Retire —
/// it stays a pending proposal for gated approval. Proven: after a FullAuto scan
/// the installed capability is still present and the Replace is still Pending.
#[tokio::test]
async fn full_auto_discovery_never_auto_retires_on_marketplace_replace() {
    let dir = std::env::temp_dir().join(format!("kria_w10_saferep_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ckb = Arc::new(SqliteCapabilityKnowledge::open(&dir.join("ckb.db")).unwrap());
    let plat = platform(&dir, ckb.clone());
    plat.refresh().await;
    // Record the installed capability so retirement would be observable.
    ckb.record_install(&cap("installed_ocr", "ocr", true, "1.0.0"))
        .await
        .unwrap();

    let engine = ContinuousDiscoveryEngine::new(
        plat.clone(),
        DiscoveryPolicy::default(),
        AutonomyLevel::FullAuto,
    );
    let report = engine.scan_once().await;

    // A discovery Replace proposal was produced (marketplace candidate).
    assert!(report.discovery_proposals >= 1);
    // But it was NOT auto-applied (elevated → gated), so nothing was retired.
    let applied = EvolutionStore::list_proposals(&*ckb, Some(ProposalStatus::Applied))
        .await
        .unwrap();
    assert!(
        !applied.iter().any(|p| p.capability_id == "installed_ocr"),
        "discovery must NOT auto-apply an elevated Replace (retire-without-install)"
    );
    // The installed capability is still present (not retired).
    assert!(
        ckb.list_installed()
            .await
            .unwrap()
            .iter()
            .any(|c| c.capability_id == "installed_ocr"),
        "the working capability must remain installed (no gap)"
    );
    // The Replace remains a Pending proposal for gated approval.
    let pending = EvolutionStore::list_proposals(&*ckb, Some(ProposalStatus::Pending))
        .await
        .unwrap();
    assert!(pending.iter().any(|p| p.capability_id == "installed_ocr"));
}

/// CONCURRENCY (BUG #2): overlapping scans must not double-propose. Two
/// simultaneous scans → the in-flight guard serializes them; the second returns
/// an empty report and no duplicate proposals are written.
#[tokio::test]
async fn concurrent_scans_do_not_double_propose() {
    let dir = std::env::temp_dir().join(format!("kria_w10_conc_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ckb = Arc::new(SqliteCapabilityKnowledge::open(&dir.join("ckb.db")).unwrap());
    let plat = platform(&dir, ckb.clone());
    plat.refresh().await;
    let engine = Arc::new(ContinuousDiscoveryEngine::new(
        plat.clone(),
        DiscoveryPolicy::default(),
        AutonomyLevel::ProposeOnly,
    ));
    let e1 = engine.clone();
    let e2 = engine.clone();
    let (_r1, _r2) = tokio::join!(
        tokio::spawn(async move { e1.scan_once().await }),
        tokio::spawn(async move { e2.scan_once().await }),
    );
    // Exactly one proposal per finding, despite two concurrent scans.
    let pending = EvolutionStore::list_proposals(&*ckb, Some(ProposalStatus::Pending))
        .await
        .unwrap();
    let for_ocr = pending
        .iter()
        .filter(|p| p.capability_id == "installed_ocr")
        .count();
    assert_eq!(
        for_ocr, 1,
        "concurrent scans must not create duplicate proposals"
    );
}
