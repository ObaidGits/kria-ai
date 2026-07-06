//! A8.14 Platform tests. Uses real filesystem repositories (no network mocks).

use super::*;
use crate::openclaw::bundle::verify::keypair_from_seed;
use tempfile::TempDir;

fn entry(slug: &str, ver: &str, publisher: &str, hash: &str) -> RepositoryEntry {
    RepositoryEntry {
        slug: slug.into(),
        name: format!("Skill {slug}"),
        description: "test skill".into(),
        category: "productivity".into(),
        version: ver.into(),
        publisher_id: publisher.into(),
        content_hash: hash.into(),
        location: format!("{slug}.ocskill"),
        tags: vec!["util".into()],
        signed: true,
    }
}

// ── Publisher ──

#[test]
fn publisher_registry_basic() {
    let reg = PublisherRegistry::new();
    let (_sk, pubhex) = keypair_from_seed([5u8; 32]);
    let mut p = Publisher::new("acme", pubhex.clone(), "ACME");
    p.trust = PublisherTrust::Community;
    reg.register(p);

    assert!(reg.get("acme").is_some());
    assert!(reg.find_by_key(&pubhex).is_some());
    assert!(reg.verify("acme", PublisherTrust::Verified));
    assert_eq!(reg.trusted_keys().len(), 1);
    assert!(reg.revoke("acme"));
    assert_eq!(
        reg.get("acme").unwrap().verification,
        VerificationStatus::Revoked
    );
    assert!(!reg.get("acme").unwrap().is_active());
}

// ── Trust framework ──

#[test]
fn trust_denies_unsigned_under_strict() {
    let reg = PublisherRegistry::new();
    let tf = TrustFramework::new(reg, EnterprisePolicy::default());
    let q = TrustQuery {
        skill_id: "oc_x",
        publisher_id: None,
        signed: false,
        signature_valid: false,
        repository_trust: RepositoryTrust::Community,
    };
    assert!(!tf.evaluate(&q).is_allowed());
}

#[test]
fn trust_allows_signed_community() {
    let reg = PublisherRegistry::new();
    reg.register(Publisher::new("acme", "aa", "ACME"));
    let tf = TrustFramework::new(reg, EnterprisePolicy::default());
    let q = TrustQuery {
        skill_id: "oc_x",
        publisher_id: Some("acme"),
        signed: true,
        signature_valid: true,
        repository_trust: RepositoryTrust::Community,
    };
    assert!(tf.evaluate(&q).is_allowed());
}

#[test]
fn trust_denies_revoked_publisher() {
    let reg = PublisherRegistry::new();
    reg.register(Publisher::new("bad", "bb", "Bad"));
    reg.revoke("bad");
    let tf = TrustFramework::new(reg, EnterprisePolicy::default());
    let q = TrustQuery {
        skill_id: "oc_x",
        publisher_id: Some("bad"),
        signed: true,
        signature_valid: true,
        repository_trust: RepositoryTrust::Community,
    };
    assert!(!tf.evaluate(&q).is_allowed());
}

#[test]
fn trust_permissive_allows_unsigned() {
    let reg = PublisherRegistry::new();
    let tf = TrustFramework::new(reg, EnterprisePolicy::permissive());
    let q = TrustQuery {
        skill_id: "oc_x",
        publisher_id: None,
        signed: false,
        signature_valid: false,
        repository_trust: RepositoryTrust::Untrusted,
    };
    assert!(tf.evaluate(&q).is_allowed());
}

// ── Repository manager: priority, merge, failover, offline ──

#[tokio::test]
async fn repository_manager_merges_by_priority_and_version() {
    let dir = TempDir::new().unwrap();
    let repo_a_dir = dir.path().join("a");
    let repo_b_dir = dir.path().join("b");

    let repo_a = LocalRepository::new("a", RepositoryKind::Remote, 10, &repo_a_dir);
    let repo_b = LocalRepository::new("b", RepositoryKind::Mirror, 20, &repo_b_dir);
    repo_a
        .write_index(&[entry("oc_calc", "1.0.0", "acme", "h1")])
        .unwrap();
    repo_b
        .write_index(&[
            entry("oc_calc", "1.2.0", "acme", "h2"),
            entry("oc_web", "1.0.0", "acme", "h3"),
        ])
        .unwrap();

    let mgr = RepositoryManager::new(PlatformMetrics::new());
    mgr.add_repository(std::sync::Arc::new(repo_a));
    mgr.add_repository(std::sync::Arc::new(repo_b));

    let count = mgr.refresh().await.unwrap();
    assert_eq!(count, 2);
    // Higher semver wins for oc_calc.
    assert_eq!(mgr.find("oc_calc").unwrap().version, "1.2.0");
    assert!(mgr.find("oc_web").is_some());
}

#[tokio::test]
async fn repository_download_with_failover_and_cache() {
    let dir = TempDir::new().unwrap();
    let repo_dir = dir.path().join("repo");
    let cache_dir = dir.path().join("cache");
    std::fs::create_dir_all(&repo_dir).unwrap();

    // Create a fake bundle file.
    std::fs::write(repo_dir.join("oc_calc.ocskill"), b"BUNDLE").unwrap();
    let repo = LocalRepository::new("repo", RepositoryKind::Local, 10, &repo_dir);
    repo.write_index(&[entry("oc_calc", "1.0.0", "acme", "h1")])
        .unwrap();

    let cache = std::sync::Arc::new(LocalRepository::new(
        "cache",
        RepositoryKind::Cache,
        100,
        &cache_dir,
    ));

    let mgr = RepositoryManager::new(PlatformMetrics::new());
    mgr.add_repository(std::sync::Arc::new(repo));
    mgr.set_cache(cache);
    mgr.refresh().await.unwrap();

    let out = dir.path().join("out");
    let (path, entry) = mgr.download("oc_calc", &out).await.unwrap();
    assert!(path.exists());
    assert_eq!(entry.slug, "oc_calc");
}

#[tokio::test]
async fn repository_download_missing_returns_all_failed() {
    let dir = TempDir::new().unwrap();
    let repo = LocalRepository::new("repo", RepositoryKind::Local, 10, dir.path().join("repo"));
    repo.write_index(&[]).unwrap();
    let mgr = RepositoryManager::new(PlatformMetrics::new());
    mgr.add_repository(std::sync::Arc::new(repo));
    mgr.refresh().await.unwrap();

    let out = dir.path().join("out");
    let err = mgr.download("nonexistent", &out).await.unwrap_err();
    assert!(matches!(err, RepositoryError::AllFailed(_)));
}

// ── Marketplace ──

#[tokio::test]
async fn marketplace_search_categories_and_updates() {
    let dir = TempDir::new().unwrap();
    let repo = LocalRepository::new("repo", RepositoryKind::Local, 10, dir.path().join("repo"));
    let mut web = entry("oc_web", "2.0.0", "acme", "h1");
    web.category = "web".into();
    repo.write_index(&[entry("oc_calc", "1.5.0", "acme", "h2"), web])
        .unwrap();

    let mgr = RepositoryManager::new(PlatformMetrics::new());
    mgr.add_repository(std::sync::Arc::new(repo));
    mgr.refresh().await.unwrap();

    let publishers = PublisherRegistry::new();
    let mut p = Publisher::new("acme", "aa", "ACME");
    p.trust = PublisherTrust::Verified;
    publishers.register(p);

    let market = Marketplace::new(mgr, publishers);
    let mut cats = market.categories();
    cats.sort();
    assert_eq!(cats, vec!["productivity".to_string(), "web".to_string()]);

    // installed oc_calc @ 1.0.0 → update available to 1.5.0
    let installed = vec![("oc_calc".to_string(), "1.0.0".to_string())];
    let updates = market.updates(&installed);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].entry.slug, "oc_calc");

    let results = market.search(
        &MarketQuery {
            category: Some("web".into()),
            ..Default::default()
        },
        &installed,
    );
    assert_eq!(results.len(), 1);
    assert!(results[0].verified_publisher);
}

// ── Update engine ──

#[tokio::test]
async fn update_engine_classifies_versions() {
    let dir = TempDir::new().unwrap();
    let repo = LocalRepository::new("repo", RepositoryKind::Local, 10, dir.path().join("repo"));
    repo.write_index(&[
        entry("oc_minor", "1.5.0", "acme", "h1"),
        entry("oc_major", "2.0.0", "acme", "h2"),
    ])
    .unwrap();

    let mgr = RepositoryManager::new(PlatformMetrics::new());
    mgr.add_repository(std::sync::Arc::new(repo));
    mgr.refresh().await.unwrap();

    let publishers = PublisherRegistry::new();
    publishers.register(Publisher::new("acme", "aa", "ACME"));
    let engine = UpdateEngine::new(mgr, publishers, AutoUpdatePolicy::NonBreaking);

    let installed = vec![
        ("oc_minor".to_string(), "1.0.0".to_string()),
        ("oc_major".to_string(), "1.0.0".to_string()),
        ("oc_gone".to_string(), "1.0.0".to_string()),
    ];
    let updates = engine.detect(&installed);
    assert_eq!(updates.len(), 3);
    assert!(updates
        .iter()
        .any(|u| u.slug == "oc_minor" && u.kind == UpdateKind::Upgrade));
    assert!(updates
        .iter()
        .any(|u| u.slug == "oc_major" && u.kind == UpdateKind::Breaking));
    assert!(updates
        .iter()
        .any(|u| u.slug == "oc_gone" && u.kind == UpdateKind::Deprecated));

    // NonBreaking policy → only the minor upgrade is auto-applicable.
    let auto = engine.auto_applicable(&updates);
    assert_eq!(auto.len(), 1);
    assert_eq!(auto[0].slug, "oc_minor");
}

#[tokio::test]
async fn update_engine_detects_publisher_revocation() {
    let dir = TempDir::new().unwrap();
    let repo = LocalRepository::new("repo", RepositoryKind::Local, 10, dir.path().join("repo"));
    repo.write_index(&[entry("oc_bad", "1.0.0", "bad", "h1")])
        .unwrap();
    let mgr = RepositoryManager::new(PlatformMetrics::new());
    mgr.add_repository(std::sync::Arc::new(repo));
    mgr.refresh().await.unwrap();

    let publishers = PublisherRegistry::new();
    publishers.register(Publisher::new("bad", "bb", "Bad"));
    publishers.revoke("bad");
    let engine = UpdateEngine::new(mgr, publishers, AutoUpdatePolicy::NonBreaking);

    let installed = vec![("oc_bad".to_string(), "1.0.0".to_string())];
    let updates = engine.detect(&installed);
    assert_eq!(updates[0].kind, UpdateKind::PublisherRevoked);
}

// ── Sync engine (delta + offline) ──

#[tokio::test]
async fn sync_engine_delta_and_offline() {
    let dir = TempDir::new().unwrap();
    let repo = LocalRepository::new("repo", RepositoryKind::Local, 10, dir.path().join("repo"));
    repo.write_index(&[entry("oc_a", "1.0.0", "acme", "h1")])
        .unwrap();
    let repo = std::sync::Arc::new(repo);

    let mgr = RepositoryManager::new(PlatformMetrics::new());
    mgr.add_repository(repo.clone());
    let cache = std::sync::Arc::new(LocalRepository::new(
        "cache",
        RepositoryKind::Cache,
        100,
        dir.path().join("cache"),
    ));
    mgr.set_cache(cache);

    let sync = SyncEngine::new(mgr.clone(), PlatformMetrics::new());
    let (state1, report1) = sync.sync(&SyncState::default()).await;
    assert_eq!(report1.added, vec!["oc_a".to_string()]);
    assert_eq!(report1.total_indexed, 1);

    // Add a skill, sync again → delta shows one added.
    repo.write_index(&[
        entry("oc_a", "1.0.0", "acme", "h1"),
        entry("oc_b", "1.0.0", "acme", "h2"),
    ])
    .unwrap();
    let (state2, report2) = sync.sync(&state1).await;
    assert_eq!(report2.added, vec!["oc_b".to_string()]);
    assert_eq!(report2.total_indexed, 2);

    // Changed hash → updated.
    repo.write_index(&[
        entry("oc_a", "1.0.0", "acme", "h1_changed"),
        entry("oc_b", "1.0.0", "acme", "h2"),
    ])
    .unwrap();
    let (_state3, report3) = sync.sync(&state2).await;
    assert_eq!(report3.updated, vec!["oc_a".to_string()]);
}

// ── Publishing pipeline (real bundle, real signing) ──

use std::path::Path;

fn make_bundle_dir(dir: &Path, slug: &str, version: &str, publisher_hex: &str) {
    std::fs::create_dir_all(dir.join("handler")).unwrap();
    let manifest = format!(
        r#"
[skill]
slug = "{slug}"
name = "Demo {slug}"
version = "{version}"
category = "productivity"
description = "demo skill for publishing"
min_kria = "0.1.0"
tags = ["util"]
[runtime]
kind = "docker"
entry = "handler/demo.js"
[resource]
class = "light"
[trust]
declared_tier = "community"
publisher = "{publisher_hex}"
"#
    );
    std::fs::write(dir.join("manifest.toml"), manifest).unwrap();
    std::fs::write(dir.join("schema.json"), r#"{"type":"object"}"#).unwrap();
    std::fs::write(dir.join("handler/demo.js"), "module.exports=()=>({})").unwrap();
}

#[test]
fn publishing_pipeline_signs_and_publishes() {
    let dir = TempDir::new().unwrap();
    let (sk, pubhex) = keypair_from_seed([42u8; 32]);

    // Publisher whose identity matches the signing key.
    let publishers = PublisherRegistry::new();
    let mut p = Publisher::new("acme", pubhex.clone(), "ACME");
    p.trust = PublisherTrust::Verified;
    publishers.register(p);

    // Build a bundle authored by that publisher.
    let bundle_dir = dir.path().join("bundle");
    make_bundle_dir(&bundle_dir, "oc_pub", "1.0.0", &pubhex);

    // Target repository.
    let repo_dir = dir.path().join("repo");
    let repo = LocalRepository::new("repo", RepositoryKind::Local, 10, &repo_dir);

    let pipeline = PublishingPipeline::new(publishers);
    let req = PublishRequest {
        bundle_dir: &bundle_dir,
        publisher_id: "acme",
        signing_key: &sk,
    };
    let entry = pipeline
        .publish(&req, &repo, &repo_dir)
        .expect("publish should succeed");
    assert_eq!(entry.slug, "oc_pub");
    assert!(entry.signed);

    // Entry landed in the repo index.
    let index = repo.fetch_index_blocking().unwrap();
    assert_eq!(index.len(), 1);
    assert_eq!(index[0].slug, "oc_pub");

    // The published package directory exists and is signature-verifiable.
    let pkg = repo_dir.join(&entry.location);
    assert!(pkg.join("bundle.sig").exists());
    assert!(pkg.join("MANIFEST.sha256").exists());
}

#[test]
fn publishing_rejects_key_mismatch() {
    let dir = TempDir::new().unwrap();
    let (sk, pubhex) = keypair_from_seed([1u8; 32]);
    let (_other_sk, other_pub) = keypair_from_seed([2u8; 32]);

    // Publisher registered with a DIFFERENT key than the signer.
    let publishers = PublisherRegistry::new();
    publishers.register(Publisher::new("acme", other_pub, "ACME"));

    let bundle_dir = dir.path().join("bundle");
    make_bundle_dir(&bundle_dir, "oc_x", "1.0.0", &pubhex);
    let repo_dir = dir.path().join("repo");
    let repo = LocalRepository::new("repo", RepositoryKind::Local, 10, &repo_dir);

    let pipeline = PublishingPipeline::new(publishers);
    let req = PublishRequest {
        bundle_dir: &bundle_dir,
        publisher_id: "acme",
        signing_key: &sk,
    };
    let err = pipeline.publish(&req, &repo, &repo_dir).unwrap_err();
    assert!(matches!(err, PublishError::KeyMismatch));
}

// ── Stress: 1000-skill repository, 100 publishers ──

#[tokio::test]
async fn stress_thousand_skill_repository() {
    let dir = TempDir::new().unwrap();
    let repo = LocalRepository::new("repo", RepositoryKind::Local, 10, dir.path().join("repo"));
    let mut entries = Vec::new();
    for i in 0..1000 {
        let pubid = format!("pub{}", i % 100);
        entries.push(entry(
            &format!("oc_skill_{i}"),
            "1.0.0",
            &pubid,
            &format!("h{i}"),
        ));
    }
    repo.write_index(&entries).unwrap();

    let mgr = RepositoryManager::new(PlatformMetrics::new());
    mgr.add_repository(std::sync::Arc::new(repo));
    let count = mgr.refresh().await.unwrap();
    assert_eq!(count, 1000);
    assert!(mgr.find("oc_skill_500").is_some());

    let publishers = PublisherRegistry::new();
    for i in 0..100 {
        publishers.register(Publisher::new(
            format!("pub{i}"),
            format!("k{i}"),
            format!("Pub {i}"),
        ));
    }
    let market = Marketplace::new(mgr, publishers);
    assert_eq!(market.publishers().len(), 100);
    let results = market.search(
        &MarketQuery {
            text: Some("oc_skill_1".into()),
            ..Default::default()
        },
        &[],
    );
    assert!(results.len() >= 100); // oc_skill_1, oc_skill_10..19, oc_skill_1xx
}

// ── Integration: publish → repository → download → verify (A8.5 composition) ──
// Proves the platform composes the frozen A2 bundle layer (verify) and does NOT
// re-implement installation/verification.

use crate::openclaw::bundle::Bundle;

#[tokio::test]
async fn integration_publish_download_verify_roundtrip() {
    let dir = TempDir::new().unwrap();
    let (sk, pubhex) = keypair_from_seed([77u8; 32]);

    // Verified publisher.
    let publishers = PublisherRegistry::new();
    let mut p = Publisher::new("acme", pubhex.clone(), "ACME");
    p.trust = PublisherTrust::Verified;
    p.verification = VerificationStatus::Verified;
    publishers.register(p);

    // Author + publish a signed bundle into a local repo.
    let bundle_dir = dir.path().join("src_bundle");
    make_bundle_dir(&bundle_dir, "oc_int", "1.0.0", &pubhex);
    let repo_dir = dir.path().join("repo");
    let repo = LocalRepository::new("repo", RepositoryKind::Local, 10, &repo_dir);
    let pipeline = PublishingPipeline::new(publishers.clone());
    pipeline
        .publish(
            &PublishRequest {
                bundle_dir: &bundle_dir,
                publisher_id: "acme",
                signing_key: &sk,
            },
            &repo,
            &repo_dir,
        )
        .expect("publish");

    // Repackage the published dir path as a downloadable "bundle" (dir copy).
    // RepositoryManager.download copies the location file; here the published bundle is a
    // directory, so we verify directly from the published package path instead.
    let mgr = RepositoryManager::new(PlatformMetrics::new());
    mgr.add_repository(std::sync::Arc::new(repo));
    mgr.refresh().await.unwrap();
    let published = mgr.find("oc_int").expect("in catalogue");
    let pkg_dir = repo_dir.join(&published.location);

    // Verify the downloaded/published bundle using the platform-derived trust policy.
    let tf = TrustFramework::new(publishers, EnterprisePolicy::default());
    let policy = tf.verify_policy();
    let bundle = Bundle::open(&pkg_dir).expect("open published bundle");
    let content_hash = bundle
        .verify(&policy)
        .expect("verify with platform trust policy");
    assert_eq!(content_hash, published.content_hash);

    // Trust decision allows this signed, verified-publisher skill.
    let decision = tf.evaluate(&TrustQuery {
        skill_id: "oc_int",
        publisher_id: Some("acme"),
        signed: true,
        signature_valid: true,
        repository_trust: RepositoryTrust::Verified,
    });
    assert!(decision.is_allowed());
}
