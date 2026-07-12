//! Wave 8 — Evolution + Benchmark against the REAL SQLite CKB (EvolutionStore).
//!
//! Proves the full loop on real storage: record execution outcomes → health
//! degrades (chronic failure) → evolution engine proposes a benchmarked,
//! auditable, reversible action → proposal persists → approve/apply/undo status
//! transitions round-trip. No provider, no network, no Docker.

use std::sync::Arc;

use kria_core::capability::descriptor::CapabilityDescriptor;
use kria_core::capability::intelligence::{
    AutonomyLevel, CapabilityKnowledge, DefaultEvolutionEngine, EvolutionStore, HealthStatus,
    ProposalKind, ProposalStatus, SqliteCapabilityKnowledge,
};

fn desc(provider: &str, cap: &str, family_tag: &str) -> CapabilityDescriptor {
    let mut d = CapabilityDescriptor::minimal(provider, cap, cap, "", serde_json::json!({}));
    d.tags = vec![kria_core::capability::descriptor::CapabilityTag::new(
        family_tag,
    )];
    d
}

#[tokio::test]
async fn chronic_failure_triggers_reversible_replace_proposal() {
    let ckb = Arc::new(SqliteCapabilityKnowledge::in_memory().unwrap());

    // Two OCR capabilities: one will fail chronically, one stays healthy.
    ckb.record_install(&desc("openclaw", "bad_ocr", "media.image.ocr"))
        .await
        .unwrap();
    ckb.record_install(&desc("localfs", "good_ocr", "media.image.ocr"))
        .await
        .unwrap();

    // bad_ocr: 5 consecutive failures → Critical health.
    for _ in 0..5 {
        ckb.record_outcome("openclaw", "bad_ocr", false, Some(50), Some("timeout"))
            .await
            .unwrap();
    }
    // good_ocr: healthy.
    for _ in 0..10 {
        ckb.record_outcome("localfs", "good_ocr", true, Some(20), None)
            .await
            .unwrap();
    }

    // Health snapshots reflect reality.
    let snaps = EvolutionStore::health_snapshots(&*ckb).await.unwrap();
    let bad = snaps.iter().find(|s| s.capability_id == "bad_ocr").unwrap();
    assert_eq!(bad.consecutive_failures, 5);
    assert_eq!(bad.success_rate(), Some(0.0));

    // Evolution engine proposes a Replace (bad → good) — gated (propose-only).
    let engine = DefaultEvolutionEngine::new(ckb.clone(), AutonomyLevel::ProposeOnly);
    let proposals = engine.analyze().await.unwrap();
    let replace = proposals
        .iter()
        .find(|p| p.kind == ProposalKind::Replace && p.capability_id == "bad_ocr")
        .expect("a replace proposal for the chronic failure");
    assert_eq!(
        replace.replacement.as_ref().unwrap(),
        &("localfs".to_string(), "good_ocr".to_string())
    );
    assert!(
        replace.requires_approval,
        "propose-only must gate application"
    );
    assert!(
        replace.rationale.contains("bad_ocr"),
        "explainable rationale"
    );

    // Persisted + queryable (auditable history, R6.2).
    let pending = EvolutionStore::list_proposals(&*ckb, Some(ProposalStatus::Pending))
        .await
        .unwrap();
    assert!(pending.iter().any(|p| p.id == replace.id));

    // Approve → apply → undo (reversible), each persisted.
    EvolutionStore::set_proposal_status(&*ckb, &replace.id, ProposalStatus::Approved)
        .await
        .unwrap();
    EvolutionStore::set_proposal_status(&*ckb, &replace.id, ProposalStatus::Applied)
        .await
        .unwrap();
    let applied = EvolutionStore::get_proposal(&*ckb, &replace.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(applied.status, ProposalStatus::Applied);
    EvolutionStore::set_proposal_status(&*ckb, &replace.id, ProposalStatus::Undone)
        .await
        .unwrap();
    let undone = EvolutionStore::get_proposal(&*ckb, &replace.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(undone.status, ProposalStatus::Undone);
}

#[tokio::test]
async fn recovery_resets_consecutive_failures_and_health() {
    let ckb = Arc::new(SqliteCapabilityKnowledge::in_memory().unwrap());
    ckb.record_install(&desc("p", "flaky", "data"))
        .await
        .unwrap();
    for _ in 0..4 {
        ckb.record_outcome("p", "flaky", false, Some(10), Some("err"))
            .await
            .unwrap();
    }
    // A success resets the chronic-failure streak (real recovery).
    ckb.record_outcome("p", "flaky", true, Some(10), None)
        .await
        .unwrap();
    let snaps = EvolutionStore::health_snapshots(&*ckb).await.unwrap();
    let s = snaps.iter().find(|s| s.capability_id == "flaky").unwrap();
    assert_eq!(s.consecutive_failures, 0, "success must reset the streak");
}

#[tokio::test]
async fn benchmark_scores_persist_and_average() {
    let ckb = Arc::new(SqliteCapabilityKnowledge::in_memory().unwrap());
    EvolutionStore::record_benchmark(&*ckb, "p", "c", true, 20, 1.0)
        .await
        .unwrap();
    EvolutionStore::record_benchmark(&*ckb, "p", "c", false, 30, 0.0)
        .await
        .unwrap();
    let avg = EvolutionStore::benchmark_score(&*ckb, "p", "c")
        .await
        .unwrap();
    assert!(
        (avg - 0.5).abs() < 0.01,
        "mean benchmark score should be 0.5, got {avg}"
    );
    assert!(EvolutionStore::benchmark_score(&*ckb, "p", "never")
        .await
        .is_none());
}

#[tokio::test]
async fn healthy_ecosystem_yields_no_proposals() {
    let ckb = Arc::new(SqliteCapabilityKnowledge::in_memory().unwrap());
    ckb.record_install(&desc("p", "solid", "data"))
        .await
        .unwrap();
    for _ in 0..10 {
        ckb.record_outcome("p", "solid", true, Some(5), None)
            .await
            .unwrap();
    }
    let snaps = EvolutionStore::health_snapshots(&*ckb).await.unwrap();
    let policy = kria_core::capability::intelligence::HealthPolicy::default();
    let classified = kria_core::capability::intelligence::health::classify(&policy, snaps);
    assert_eq!(
        classified
            .iter()
            .find(|s| s.capability_id == "solid")
            .unwrap()
            .status,
        HealthStatus::Healthy
    );
    let engine = DefaultEvolutionEngine::new(ckb, AutonomyLevel::ProposeOnly);
    assert!(
        engine.analyze().await.unwrap().is_empty(),
        "no proposals for a healthy ecosystem"
    );
}

// ── REAL apply/undo through the neutral LifecycleManager (not status-only) ───

use kria_core::capability::index::{Embedder, InMemoryFederatedIndex};
use kria_core::capability::intelligence::DefaultLifecycleManager;
use kria_core::capability::platform::CapabilityPlatform;
use kria_core::capability::registry::ProviderRegistry;

struct HashEmb;
impl Embedder for HashEmb {
    fn embed(&self, t: &str) -> Result<Vec<f32>, kria_core::capability::error::CapError> {
        let mut v = vec![0.0f32; 16];
        for (i, b) in t.bytes().enumerate() {
            v[i % 16] += b as f32;
        }
        Ok(v)
    }
    fn dim(&self) -> usize {
        16
    }
    fn model_id(&self) -> &str {
        "h"
    }
}

#[tokio::test]
async fn retire_proposal_apply_and_undo_perform_real_state_changes() {
    use kria_core::capability::intelligence::CapabilityKnowledge;

    let ckb = Arc::new(SqliteCapabilityKnowledge::in_memory().unwrap());
    ckb.record_install(&desc("p", "sketchy", "data"))
        .await
        .unwrap();
    // Mark quarantined in the CKB so health → Quarantined → Retire proposal.
    ckb.set_state("p", "sketchy", "quarantined").await.unwrap();
    // (record an outcome so the row has activity)
    ckb.record_outcome("p", "sketchy", false, Some(10), Some("bad"))
        .await
        .unwrap();

    // Real platform (empty registry is fine — retire/recover are CKB-state ops).
    let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmb)));
    let platform = Arc::new(
        CapabilityPlatform::new(Arc::new(ProviderRegistry::new(index)))
            .with_knowledge(ckb.clone())
            .with_evolution_store(ckb.clone()),
    );
    let lifecycle = DefaultLifecycleManager::new(platform).with_knowledge(ckb.clone());
    let engine = DefaultEvolutionEngine::new(ckb.clone(), AutonomyLevel::ProposeOnly);

    // Quarantined → Retire proposal.
    let proposals = engine.analyze().await.unwrap();
    let retire = proposals
        .iter()
        .find(|p| p.kind == ProposalKind::Retire && p.capability_id == "sketchy")
        .expect("retire proposal for quarantined capability");

    // Before apply: still present (archived not yet).
    assert!(ckb
        .list_installed()
        .await
        .unwrap()
        .iter()
        .any(|d| d.capability_id == "sketchy"));

    // REAL apply → lifecycle.retire → CKB state archived → excluded from listing.
    engine.apply(retire, &lifecycle).await.unwrap();
    assert!(
        !ckb.list_installed()
            .await
            .unwrap()
            .iter()
            .any(|d| d.capability_id == "sketchy"),
        "applied retire must archive the capability (real state change)"
    );
    assert_eq!(
        EvolutionStore::get_proposal(&*ckb, &retire.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ProposalStatus::Applied
    );

    // REAL undo → lifecycle.recover → CKB state enabled → back in listing.
    engine.undo(retire, &lifecycle).await.unwrap();
    assert!(
        ckb.list_installed()
            .await
            .unwrap()
            .iter()
            .any(|d| d.capability_id == "sketchy"),
        "undo must recover the archived capability (real reversal)"
    );
    assert_eq!(
        EvolutionStore::get_proposal(&*ckb, &retire.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ProposalStatus::Undone
    );
}

#[tokio::test]
async fn full_auto_applies_without_approval() {
    use kria_core::capability::intelligence::CapabilityKnowledge;
    let ckb = Arc::new(SqliteCapabilityKnowledge::in_memory().unwrap());
    ckb.record_install(&desc("p", "q", "data")).await.unwrap();
    ckb.set_state("p", "q", "quarantined").await.unwrap();
    ckb.record_outcome("p", "q", false, Some(10), Some("bad"))
        .await
        .unwrap();

    let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmb)));
    let platform = Arc::new(
        CapabilityPlatform::new(Arc::new(ProviderRegistry::new(index)))
            .with_knowledge(ckb.clone())
            .with_evolution_store(ckb.clone()),
    );
    let lifecycle = DefaultLifecycleManager::new(platform).with_knowledge(ckb.clone());
    // FullAuto → retire auto-applies (no approval gate).
    let engine = DefaultEvolutionEngine::new(ckb.clone(), AutonomyLevel::FullAuto);
    let applied = engine.auto_apply(&lifecycle).await.unwrap();
    assert!(applied.iter().any(|p| p.kind == ProposalKind::Retire));
    // The quarantined capability was actually archived.
    assert!(!ckb
        .list_installed()
        .await
        .unwrap()
        .iter()
        .any(|d| d.capability_id == "q"));
}

/// Real on-disk CKB validation: open the ACTUAL app DB file (copy, to avoid the
/// running app's lock) and drive the full evolution loop on real persisted
/// capabilities. Gated on `KRIA_REALDB=/path/to/cpp_knowledge.db`.
#[tokio::test]
async fn real_ondisk_ckb_drives_evolution_and_apply() {
    use kria_core::capability::intelligence::CapabilityKnowledge;

    let Ok(path) = std::env::var("KRIA_REALDB") else {
        eprintln!("skipping: set KRIA_REALDB=/path/to/copy of cpp_knowledge.db");
        return;
    };
    let ckb = Arc::new(SqliteCapabilityKnowledge::open(std::path::Path::new(&path)).unwrap());

    // The real installed capabilities are present + Wave 8 schema is live.
    let installed = ckb.list_installed().await.unwrap();
    eprintln!("real installed capabilities: {}", installed.len());
    assert!(
        !installed.is_empty(),
        "real DB must contain installed capabilities"
    );
    let target = installed[0].capability_id.clone();
    let provider = installed[0].provider_id.clone();

    // Drive a REAL chronic failure on a real capability, persisted to the real DB.
    for _ in 0..5 {
        ckb.record_outcome(&provider, &target, false, Some(40), Some("induced failure"))
            .await
            .unwrap();
    }
    // Also quarantine it (trust/integrity gate) so the proposal is a Retire —
    // which applies via CKB state only (no provider needed in this bare harness),
    // proving the real-DB apply/undo round-trip end-to-end.
    ckb.set_state(&provider, &target, "quarantined")
        .await
        .unwrap();
    let snaps = EvolutionStore::health_snapshots(&*ckb).await.unwrap();
    let h = snaps.iter().find(|s| s.capability_id == target).unwrap();
    assert_eq!(
        h.consecutive_failures, 5,
        "real DB must persist the failure streak"
    );
    assert!(h.quarantined, "real DB must persist quarantine state");
    eprintln!(
        "health of {target}: rate={:?} consec={} quarantined={}",
        h.success_rate(),
        h.consecutive_failures,
        h.quarantined
    );

    // Evolution engine proposes from the real health data.
    let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmb)));
    let platform = Arc::new(
        CapabilityPlatform::new(Arc::new(ProviderRegistry::new(index)))
            .with_knowledge(ckb.clone())
            .with_evolution_store(ckb.clone()),
    );
    let lifecycle = DefaultLifecycleManager::new(platform).with_knowledge(ckb.clone());
    let engine = DefaultEvolutionEngine::new(ckb.clone(), AutonomyLevel::ProposeOnly);
    let proposals = engine.analyze().await.unwrap();
    eprintln!("proposals from real DB: {}", proposals.len());
    let prop = proposals
        .iter()
        .find(|p| p.capability_id == target)
        .expect("a proposal for the failing real capability");
    eprintln!(
        "proposal: {} {} — {}",
        prop.kind.as_str(),
        prop.capability_id,
        prop.rationale
    );

    // Proposal persisted in the real DB (auditable).
    let persisted = EvolutionStore::list_proposals(&*ckb, Some(ProposalStatus::Pending))
        .await
        .unwrap();
    assert!(
        persisted.iter().any(|p| p.id == prop.id),
        "proposal persisted to real DB"
    );

    // Real apply + undo round-trip on the real DB.
    engine.apply(prop, &lifecycle).await.unwrap();
    assert_eq!(
        EvolutionStore::get_proposal(&*ckb, &prop.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ProposalStatus::Applied
    );
    engine.undo(prop, &lifecycle).await.unwrap();
    assert_eq!(
        EvolutionStore::get_proposal(&*ckb, &prop.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ProposalStatus::Undone
    );
    eprintln!("real-DB evolution apply/undo round-trip OK");
}
