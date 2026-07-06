//! Phase A2 — Production Skill Bundle System integration tests.
//!
//! Exercises the full lifecycle with real signed `.ocskill` bundles and a mock activation sink
//! (standing in for the router / runtime / tool-index refresh): install, update, rollback,
//! corrupt bundle, invalid signature, duplicate, version/publisher conflict, hot reload,
//! enable/disable, uninstall, and no-leak guarantees. No Docker required.

use kria_core::openclaw::audit::AuditLedger;
use kria_core::openclaw::bundle::verify::{
    keypair_from_seed, sign_bundle, write_hash_tree, TrustPolicy,
};
use kria_core::openclaw::bundle::version::VersionRelation;
use kria_core::openclaw::bundle::{installer::InstallError, BundleInstaller, SkillActivation};
use kria_core::openclaw::registry::SkillRegistry;
use kria_core::openclaw::types::SkillDescriptor;
use semver::Version;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

// ── Mock activation sink (router + runtime + tool-index refresh) ───────────────

struct MockActivation {
    active: Mutex<HashSet<String>>,
    reindex_count: AtomicUsize,
    fail_activate: bool,
}

impl MockActivation {
    fn new(fail_activate: bool) -> Arc<Self> {
        Arc::new(Self {
            active: Mutex::new(HashSet::new()),
            reindex_count: AtomicUsize::new(0),
            fail_activate,
        })
    }
    fn is_active(&self, slug: &str) -> bool {
        self.active.lock().unwrap().contains(slug)
    }
    fn reindexes(&self) -> usize {
        self.reindex_count.load(Ordering::SeqCst)
    }
}

impl SkillActivation for MockActivation {
    fn activate(&self, skill: &SkillDescriptor) -> Result<(), String> {
        if self.fail_activate {
            return Err("simulated activation failure".into());
        }
        self.active.lock().unwrap().insert(skill.skill_id.clone());
        Ok(())
    }
    fn deactivate(&self, skill_id: &str) -> Result<(), String> {
        self.active.lock().unwrap().remove(skill_id);
        Ok(())
    }
    fn reindex(&self) {
        self.reindex_count.fetch_add(1, Ordering::SeqCst);
    }
}

// ── Bundle authoring helper ────────────────────────────────────────────────────

struct BundleSpec {
    slug: String,
    version: String,
    publisher_hex: String,
    tier: String,
    extra_caps: String,
    result_value: i64,
}

fn author_bundle(dir: &Path, spec: &BundleSpec, signing_seed: [u8; 32]) -> PathBuf {
    let (sk, _pub_hex) = keypair_from_seed(signing_seed);
    let root = dir.join(format!("{}-{}", spec.slug, spec.version));
    std::fs::create_dir_all(root.join("handler")).unwrap();

    let manifest = format!(
        r#"
[skill]
slug = "{slug}"
name = "Test Skill"
version = "{version}"
category = "productivity"
tags = ["test"]
intent = "Return a fixed number."
description = "Returns a fixed number for testing."
min_kria = "0.1.0"
license = "MIT"

[runtime]
kind = "docker"
entry = "handler/entry.js"
mcp = true

[resource]
class = "light"
memory_mb = 256
timeout_secs = 30

[trust]
declared_tier = "{tier}"
publisher = "{publisher}"
{extra_caps}
"#,
        slug = spec.slug,
        version = spec.version,
        tier = spec.tier,
        publisher = spec.publisher_hex,
        extra_caps = spec.extra_caps,
    );
    std::fs::write(root.join("manifest.toml"), manifest).unwrap();
    std::fs::write(
        root.join("schema.json"),
        r#"{"type":"object","properties":{}}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("handler/entry.js"),
        format!("module.exports=()=>({{result:{}}})", spec.result_value),
    )
    .unwrap();

    write_hash_tree(&root).unwrap();
    sign_bundle(&root, &sk).unwrap();
    root
}

// ── Test fixture ───────────────────────────────────────────────────────────────

struct Fixture {
    _tmp: TempDir,
    store: PathBuf,
    author_dir: PathBuf,
    registry: Arc<SkillRegistry>,
    audit: Arc<AuditLedger>,
    activation: Arc<MockActivation>,
}

fn fixture(fail_activate: bool) -> Fixture {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("skills.db");
    let registry = Arc::new(SkillRegistry::open(&db).unwrap());
    let audit = Arc::new(AuditLedger::open(&db, b"test-key".to_vec()).unwrap());
    let store = tmp.path().join("store");
    std::fs::create_dir_all(&store).unwrap();
    let author_dir = tmp.path().join("authored");
    std::fs::create_dir_all(&author_dir).unwrap();
    let activation = MockActivation::new(fail_activate);
    Fixture {
        _tmp: tmp,
        store,
        author_dir,
        registry,
        audit,
        activation,
    }
}

impl Fixture {
    fn installer(&self) -> BundleInstaller {
        BundleInstaller::new(
            self.registry.clone(),
            self.audit.clone(),
            self.store.clone(),
        )
        .with_activation(self.activation.clone())
        .with_kria_version(Version::new(1, 0, 0))
        .with_trust_policy(TrustPolicy {
            trusted_keys: Vec::new(),
            require_signature: true,
        })
    }
}

const SEED_A: [u8; 32] = [11u8; 32];
const SEED_B: [u8; 32] = [22u8; 32];

fn pub_hex(seed: [u8; 32]) -> String {
    keypair_from_seed(seed).1
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn install_makes_skill_hot_available() {
    let fx = fixture(false);
    let spec = BundleSpec {
        slug: "oc_test".into(),
        version: "1.0.0".into(),
        publisher_hex: pub_hex(SEED_A),
        tier: "community".into(),
        extra_caps: String::new(),
        result_value: 42,
    };
    let bundle = author_bundle(&fx.author_dir, &spec, SEED_A);

    let outcome = fx.installer().install(&bundle).expect("install ok");
    assert_eq!(outcome.relation, VersionRelation::Fresh);
    assert!(fx.registry.get("oc_test").is_ok(), "registry has skill");
    assert!(
        fx.activation.is_active("oc_test"),
        "router/runtime activated"
    );
    assert!(fx.activation.reindexes() >= 1, "tool index reindexed");
    let prov = fx.registry.get_provenance("oc_test").unwrap().unwrap();
    assert_eq!(prov.version, "1.0.0");
    assert!(!prov.content_hash.is_empty());
}

#[test]
fn update_replaces_with_new_version() {
    let fx = fixture(false);
    let publisher = pub_hex(SEED_A);
    let v1 = author_bundle(
        &fx.author_dir,
        &BundleSpec {
            slug: "oc_test".into(),
            version: "1.0.0".into(),
            publisher_hex: publisher.clone(),
            tier: "community".into(),
            extra_caps: String::new(),
            result_value: 1,
        },
        SEED_A,
    );
    let v2 = author_bundle(
        &fx.author_dir,
        &BundleSpec {
            slug: "oc_test".into(),
            version: "1.1.0".into(),
            publisher_hex: publisher,
            tier: "community".into(),
            extra_caps: String::new(),
            result_value: 2,
        },
        SEED_A,
    );
    let inst = fx.installer();
    inst.install(&v1).unwrap();
    let outcome = inst.install(&v2).expect("update ok");
    assert_eq!(outcome.relation, VersionRelation::Upgrade);
    let prov = fx.registry.get_provenance("oc_test").unwrap().unwrap();
    assert_eq!(prov.version, "1.1.0");
    assert!(fx.activation.is_active("oc_test"));
}

#[test]
fn downgrade_is_blocked() {
    let fx = fixture(false);
    let publisher = pub_hex(SEED_A);
    let v2 = author_bundle(
        &fx.author_dir,
        &BundleSpec {
            slug: "oc_test".into(),
            version: "1.1.0".into(),
            publisher_hex: publisher.clone(),
            tier: "community".into(),
            extra_caps: String::new(),
            result_value: 2,
        },
        SEED_A,
    );
    let v1 = author_bundle(
        &fx.author_dir,
        &BundleSpec {
            slug: "oc_test".into(),
            version: "1.0.0".into(),
            publisher_hex: publisher,
            tier: "community".into(),
            extra_caps: String::new(),
            result_value: 1,
        },
        SEED_A,
    );
    let inst = fx.installer();
    inst.install(&v2).unwrap();
    let err = inst.install(&v1).unwrap_err();
    assert!(matches!(err, InstallError::DowngradeBlocked { .. }));
    // Still on 1.1.0.
    assert_eq!(
        fx.registry
            .get_provenance("oc_test")
            .unwrap()
            .unwrap()
            .version,
        "1.1.0"
    );
}

#[test]
fn duplicate_install_is_idempotent() {
    let fx = fixture(false);
    let spec = BundleSpec {
        slug: "oc_test".into(),
        version: "1.0.0".into(),
        publisher_hex: pub_hex(SEED_A),
        tier: "community".into(),
        extra_caps: String::new(),
        result_value: 7,
    };
    let bundle = author_bundle(&fx.author_dir, &spec, SEED_A);
    let inst = fx.installer();
    inst.install(&bundle).unwrap();
    let outcome = inst.install(&bundle).expect("idempotent reinstall");
    assert_eq!(outcome.relation, VersionRelation::Same);
}

#[test]
fn corrupt_bundle_is_rejected_and_prior_intact() {
    let fx = fixture(false);
    let publisher = pub_hex(SEED_A);
    let v1 = author_bundle(
        &fx.author_dir,
        &BundleSpec {
            slug: "oc_test".into(),
            version: "1.0.0".into(),
            publisher_hex: publisher.clone(),
            tier: "community".into(),
            extra_caps: String::new(),
            result_value: 1,
        },
        SEED_A,
    );
    let inst = fx.installer();
    inst.install(&v1).unwrap();

    // Author v2 then tamper AFTER signing → hash mismatch.
    let v2 = author_bundle(
        &fx.author_dir,
        &BundleSpec {
            slug: "oc_test".into(),
            version: "1.1.0".into(),
            publisher_hex: publisher,
            tier: "community".into(),
            extra_caps: String::new(),
            result_value: 2,
        },
        SEED_A,
    );
    std::fs::write(
        v2.join("handler/entry.js"),
        "module.exports=()=>({result:999})",
    )
    .unwrap();

    let err = inst.install(&v2).unwrap_err();
    assert!(matches!(err, InstallError::Bundle(_)), "got {err:?}");
    // Prior install untouched.
    assert_eq!(
        fx.registry
            .get_provenance("oc_test")
            .unwrap()
            .unwrap()
            .version,
        "1.0.0"
    );
    assert!(fx.activation.is_active("oc_test"));
}

#[test]
fn invalid_signature_is_rejected() {
    let fx = fixture(false);
    // Manifest declares publisher A, but the bundle is signed by key B → signature rejected.
    let spec = BundleSpec {
        slug: "oc_test".into(),
        version: "1.0.0".into(),
        publisher_hex: pub_hex(SEED_A),
        tier: "community".into(),
        extra_caps: String::new(),
        result_value: 1,
    };
    let bundle = author_bundle(&fx.author_dir, &spec, SEED_B); // signed with wrong key
    let err = fx.installer().install(&bundle).unwrap_err();
    assert!(matches!(err, InstallError::Bundle(_)), "got {err:?}");
    assert!(fx.registry.get("oc_test").is_err(), "nothing installed");
}

#[test]
fn publisher_conflict_is_detected() {
    let fx = fixture(false);
    let a = author_bundle(
        &fx.author_dir,
        &BundleSpec {
            slug: "oc_test".into(),
            version: "1.0.0".into(),
            publisher_hex: pub_hex(SEED_A),
            tier: "community".into(),
            extra_caps: String::new(),
            result_value: 1,
        },
        SEED_A,
    );
    let inst = fx.installer();
    inst.install(&a).unwrap();
    // Same slug, DIFFERENT publisher (signed by B, declares B).
    let b = author_bundle(
        &fx.author_dir,
        &BundleSpec {
            slug: "oc_test".into(),
            version: "1.2.0".into(),
            publisher_hex: pub_hex(SEED_B),
            tier: "community".into(),
            extra_caps: String::new(),
            result_value: 2,
        },
        SEED_B,
    );
    let err = inst.install(&b).unwrap_err();
    assert!(matches!(err, InstallError::Deps(_)), "got {err:?}");
}

#[test]
fn activation_failure_triggers_rollback() {
    let fx = fixture(true); // activation always fails
    let spec = BundleSpec {
        slug: "oc_test".into(),
        version: "1.0.0".into(),
        publisher_hex: pub_hex(SEED_A),
        tier: "community".into(),
        extra_caps: String::new(),
        result_value: 1,
    };
    let bundle = author_bundle(&fx.author_dir, &spec, SEED_A);
    let err = fx.installer().install(&bundle).unwrap_err();
    assert!(matches!(err, InstallError::RolledBack(_)), "got {err:?}");
    // Registry restored (fresh → removed) and no leaked store dir.
    assert!(fx.registry.get("oc_test").is_err(), "registry rolled back");
    assert!(
        !fx.store.join("oc_test").join("1.0.0").exists(),
        "no leaked bundle files"
    );
}

#[test]
fn uninstall_removes_everything() {
    let fx = fixture(false);
    let spec = BundleSpec {
        slug: "oc_test".into(),
        version: "1.0.0".into(),
        publisher_hex: pub_hex(SEED_A),
        tier: "community".into(),
        extra_caps: String::new(),
        result_value: 1,
    };
    let bundle = author_bundle(&fx.author_dir, &spec, SEED_A);
    let inst = fx.installer();
    inst.install(&bundle).unwrap();
    assert!(fx.store.join("oc_test").exists());

    inst.uninstall("oc_test").unwrap();
    assert!(fx.registry.get("oc_test").is_err(), "registry cleared");
    assert!(!fx.activation.is_active("oc_test"), "deactivated");
    assert!(
        !fx.store.join("oc_test").exists(),
        "store files removed (no leak)"
    );
    assert!(matches!(
        inst.uninstall("oc_test"),
        Err(InstallError::NotFound(_))
    ));
}

#[test]
fn enable_disable_roundtrip() {
    let fx = fixture(false);
    let spec = BundleSpec {
        slug: "oc_test".into(),
        version: "1.0.0".into(),
        publisher_hex: pub_hex(SEED_A),
        tier: "community".into(),
        extra_caps: String::new(),
        result_value: 1,
    };
    let bundle = author_bundle(&fx.author_dir, &spec, SEED_A);
    let inst = fx.installer();
    inst.install(&bundle).unwrap();

    inst.disable("oc_test").unwrap();
    assert!(
        !fx.activation.is_active("oc_test"),
        "disabled → deactivated"
    );

    inst.enable("oc_test").unwrap();
    assert!(fx.activation.is_active("oc_test"), "enabled → reactivated");
}

#[test]
fn install_from_tar_archive() {
    // Prove the drop-a-file UX: author a bundle dir, tar it into `example.ocskill`, install.
    let fx = fixture(false);
    let spec = BundleSpec {
        slug: "oc_test".into(),
        version: "1.0.0".into(),
        publisher_hex: pub_hex(SEED_A),
        tier: "community".into(),
        extra_caps: String::new(),
        result_value: 5,
    };
    let root = author_bundle(&fx.author_dir, &spec, SEED_A);

    let archive_path = fx.author_dir.join("example.ocskill");
    {
        let file = std::fs::File::create(&archive_path).unwrap();
        let mut builder = tar::Builder::new(file);
        // Store bundle contents at the archive root.
        builder.append_dir_all(".", &root).unwrap();
        builder.finish().unwrap();
    }

    let outcome = fx
        .installer()
        .install(&archive_path)
        .expect("install from .ocskill");
    assert_eq!(outcome.relation, VersionRelation::Fresh);
    assert!(fx.registry.get("oc_test").is_ok());
}

#[tokio::test]
async fn lifecycle_events_are_emitted() {
    use kria_core::openclaw::bundle::events::{self, BundleLifecycleEvent};
    let fx = fixture(false);
    let mut rx = events::subscribe();
    let spec = BundleSpec {
        slug: "oc_evt".into(),
        version: "1.0.0".into(),
        publisher_hex: pub_hex(SEED_A),
        tier: "community".into(),
        extra_caps: String::new(),
        result_value: 1,
    };
    let bundle = author_bundle(&fx.author_dir, &spec, SEED_A);
    fx.installer().install(&bundle).unwrap();

    let mut saw_installing = false;
    let mut saw_installed = false;
    while let Ok(ev) = rx.try_recv() {
        if ev.slug() != "oc_evt" {
            continue;
        }
        match ev {
            BundleLifecycleEvent::Installing { .. } => saw_installing = true,
            BundleLifecycleEvent::Installed { .. } => saw_installed = true,
            _ => {}
        }
    }
    assert!(saw_installing && saw_installed);
}

// ── Real activation adapter (A6 registry-driven hot-reload) ────────────────────
//
// REGRESSION (task 5, R11/R6/R3 validation): this test previously constructed
// `ToolRegistryActivation` with a `ToolRegistry` + `RuntimeRegistry` and asserted
// the installed skill became callable as an INDIVIDUAL `oc_<slug>` tool
// (`tool_registry.get_def("oc_test")`). That path was ALREADY DEAD: the legacy
// `register_skill` it depended on is fully disabled under A6 (always returns
// `false`), so `activate()` ALWAYS returned `Err(...)`, which made
// `installer.install(&bundle)` ALWAYS roll back — reproduced directly by reverting
// `activation.rs` to its prior content and re-running this exact test, which failed
// with `RolledBack("activation: no runtime backend available for skill 'oc_test'")`.
//
// Fixed `ToolRegistryActivation` to match the ACTUAL A6 contract: activation always
// succeeds (a skill becomes routable the instant it is `Enabled` in
// `ProductionSkillRegistry` — `SemanticSkillRouter::route` reads `get_enabled_skills()`
// fresh every call, no per-skill tool registration exists or is needed) and just
// triggers the reindex callback. This test now asserts THAT real contract: install
// succeeds, the skill is `Enabled` in the registry (hence routable), reindex fires;
// uninstall removes it from the registry and fires reindex again.

use kria_core::openclaw::registry::SkillState;
use kria_core::openclaw::ToolRegistryActivation;

#[test]
fn real_activation_makes_skill_routable_then_removes_it() {
    let fx = fixture(false);

    let reindexes = Arc::new(AtomicUsize::new(0));
    let reindexes_cb = reindexes.clone();
    let activation = Arc::new(
        ToolRegistryActivation::new().with_reindex(Arc::new(move || {
            reindexes_cb.fetch_add(1, Ordering::SeqCst);
        })),
    );

    let installer = BundleInstaller::new(fx.registry.clone(), fx.audit.clone(), fx.store.clone())
        .with_activation(activation)
        .with_kria_version(Version::new(1, 0, 0))
        .with_trust_policy(TrustPolicy {
            trusted_keys: Vec::new(),
            require_signature: true,
        });

    let spec = BundleSpec {
        slug: "oc_test".into(),
        version: "1.0.0".into(),
        publisher_hex: pub_hex(SEED_A),
        tier: "community".into(),
        extra_caps: String::new(),
        result_value: 3,
    };
    let bundle = author_bundle(&fx.author_dir, &spec, SEED_A);

    installer
        .install(&bundle)
        .expect("install must succeed (real bug: previously always rolled back)");

    // Real A6 contract: the skill is discoverable via the registry the semantic
    // router reads fresh — not via an individual ToolRegistry tool entry.
    let installed = fx
        .registry
        .get("oc_test")
        .expect("skill must be in the registry after install");
    assert!(
        matches!(
            installed.status,
            kria_core::openclaw::types::SkillStatus::Active
        ),
        "installed skill must be active/routable, got status={:?}",
        installed.status
    );
    assert!(
        reindexes.load(Ordering::SeqCst) >= 1,
        "tool index reindexed on install"
    );

    installer.uninstall("oc_test").unwrap();
    let after = fx.registry.get_skill("oc_test");
    let removed_or_disabled = after
        .map(|m| matches!(m.state, SkillState::Removed | SkillState::Disabled))
        .unwrap_or(true);
    assert!(
        removed_or_disabled,
        "skill must be removed/disabled after uninstall"
    );
    assert!(
        reindexes.load(Ordering::SeqCst) >= 2,
        "reindex after uninstall"
    );
}
