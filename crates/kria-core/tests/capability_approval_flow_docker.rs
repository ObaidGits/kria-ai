//! Milestone 4 real approval-flow validation — the live
//! `authorize → approve → execute → revoke → re-prompt` lifecycle that the
//! desktop `cpp_authorize`/`cpp_approve`/`cpp_execute`/`cpp_revoke_grant`
//! commands drive, exercised directly against the same neutral engine + durable
//! grant store the commands use.
//!
//! Two real halves in one test:
//!  - **A (real Docker):** the permission gate admits the GREEN calculator
//!    (`NeverAsk`) and the platform executes it end-to-end on real Docker
//!    (`3+3 → 6`). Proves the gate → execute wiring.
//!  - **B (durable grants, real file):** the full approval state machine on an
//!    elevated (network) descriptor using an on-disk [`GrantStore`]: first use
//!    prompts, an approval grant is persisted, reuse is silent, the decision
//!    survives a store *reopen* (desktop-restart durability, R6.4), revoke forces
//!    a fresh prompt. Proves approve → reuse → persist → revoke → re-prompt.
//!
//! Gated on `KRIA_CPP_DOCKER=1` (needs Docker + `kria/openclaw-substrate:latest`
//! + a populated `~/.kria/skills.db`). Run:
//!
//! ```bash
//! KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_approval_flow_docker -- --nocapture
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use kria_core::capability::descriptor::{CapabilityDescriptor, Effects, Reversibility};
use kria_core::capability::grants::{GrantDecision, GrantStore, ScopeKind};
use kria_core::capability::index::{InMemoryFederatedIndex, MemoryEmbedder};
use kria_core::capability::permission::{
    approval_grant, AuthorizeRequest, DefaultPermissionEngine, PermissionDecision,
    PermissionEngine, PermissionTier,
};
use kria_core::capability::platform::CapabilityPlatform;
use kria_core::capability::provider::{CapabilityRequest, RequestContext};
use kria_core::capability::registry::ProviderRegistry;
use kria_core::capability::{CapabilityOutcome, OpenClawProvider};
use kria_core::openclaw::config::OpenClawConfig;
use kria_core::openclaw::pool::ContainerPool;
use kria_core::openclaw::registry::ProductionSkillRegistry;
use kria_core::openclaw::runtime::{DockerRuntime, SkillRuntime};

/// Build an elevated (network) descriptor: reversible + idempotent, so the
/// engine assigns a context (session) tier with grant reuse — the branch the
/// approve/reuse/revoke lifecycle exercises. Named as a synthetic capability so
/// it is never confused with a real installed skill.
fn elevated_descriptor() -> CapabilityDescriptor {
    let mut d = CapabilityDescriptor::minimal(
        "openclaw",
        "cpp_test_network_fetch",
        "Test Network Fetch",
        "Synthetic elevated capability (network) for approval-flow validation.",
        serde_json::json!({ "type": "object" }),
    );
    d.effects = Effects {
        classes: vec!["network".to_string()],
        reversible: Reversibility::Reversible,
        idempotent: true,
        resource_class: Default::default(),
    };
    d
}

#[tokio::test]
async fn approval_lifecycle_authorize_approve_execute_revoke_reprompt() {
    if std::env::var("KRIA_CPP_DOCKER").is_err() {
        eprintln!("skipping: set KRIA_CPP_DOCKER=1 (needs Docker + substrate image + skills.db)");
        return;
    }
    let real_db = dirs::home_dir().unwrap().join(".kria/skills.db");
    if !real_db.exists() {
        eprintln!("skipping: ~/.kria/skills.db not found");
        return;
    }

    // ── Half A: real Docker gate → execute (GREEN calculator, NeverAsk) ──────
    let tmp_db: PathBuf =
        std::env::temp_dir().join(format!("kria_cpp_approval_{}.db", std::process::id()));
    std::fs::copy(&real_db, &tmp_db).expect("copy skills.db");

    let registry = Arc::new(ProductionSkillRegistry::new(&tmp_db).expect("open skills registry"));
    let mut cfg = OpenClawConfig::default();
    cfg.enabled = true;
    cfg.image = "kria/openclaw-substrate:latest".to_string();
    let pool = Arc::new(ContainerPool::new(cfg).await.expect("container pool"));
    let runtime: Arc<dyn SkillRuntime> = Arc::new(DockerRuntime::new(pool.clone()));

    let provider = OpenClawProvider::new(registry, runtime);
    let embedder = Arc::new(MemoryEmbedder::load().expect("embedder"));
    let index = Arc::new(InMemoryFederatedIndex::new(embedder));
    let preg = Arc::new(ProviderRegistry::new(index));
    preg.register(Arc::new(provider));
    let platform = CapabilityPlatform::new(preg);
    let report = platform.refresh().await;
    assert!(report.total_descriptors > 0, "must describe skills");

    let engine = DefaultPermissionEngine;
    // Durable on-disk grant store (simulates the desktop's cpp_grants.db).
    let grant_path: PathBuf =
        std::env::temp_dir().join(format!("kria_cpp_grants_{}.db", std::process::id()));
    let _ = std::fs::remove_file(&grant_path);
    let grants = GrantStore::open(&grant_path).expect("open grant store");

    // Gate the calculator: GREEN/reversible ⇒ NeverAsk ⇒ execute immediately.
    if let Some(calc) = platform
        .descriptor("openclaw", "oc_calculator")
        .expect("descriptor lookup")
    {
        let req = AuthorizeRequest::from_descriptor(&calc, Some("sess-A".into()), None);
        let decision = engine.authorize(&req, &grants);
        assert!(
            matches!(
                decision,
                PermissionDecision::Allow {
                    tier: PermissionTier::NeverAsk,
                    ..
                }
            ),
            "calculator must be NeverAsk; got {decision:?}"
        );
        let out = platform
            .execute(CapabilityRequest {
                provider_id: "openclaw".into(),
                capability_id: "oc_calculator".into(),
                args: serde_json::json!({ "expression": "3+3" }),
                context: RequestContext::new(),
                granted_effects: calc.effects.classes.clone(),
            })
            .await
            .expect("execute calculator");
        match out {
            CapabilityOutcome::Value(v) => {
                assert!(v.to_string().contains('6'), "3+3 must be 6, got {v}");
                eprintln!("[A] gate→execute calculator = {v}");
            }
            other => panic!("expected Value, got {other:?}"),
        }
    } else {
        eprintln!("[A] oc_calculator not installed; skipping execute half");
    }

    // ── Half B: full approval lifecycle on an elevated descriptor ────────────
    let elevated = elevated_descriptor();
    let session = Some("sess-B".to_string());
    let auth = || AuthorizeRequest::from_descriptor(&elevated, session.clone(), None);

    // 1) First use ⇒ Prompt (no covering grant yet).
    let d1 = engine.authorize(&auth(), &grants);
    assert!(d1.is_prompt(), "elevated first-use must prompt; got {d1:?}");
    eprintln!("[B] first authorize = {d1:?}");

    // 2) User approves for the session ⇒ persist an Allow grant.
    let grant = approval_grant(&auth(), ScopeKind::Session, GrantDecision::Allow);
    let grant_id = grant.grant_id.clone();
    grants.insert(&grant).expect("persist grant");

    // 3) Reuse ⇒ Allow (no prompt), backed by the grant.
    let d2 = engine.authorize(&auth(), &grants);
    match &d2 {
        PermissionDecision::Allow { grant_id: gid, .. } => {
            assert_eq!(gid.as_deref(), Some(grant_id.as_str()), "reuse same grant");
            eprintln!("[B] reuse authorize = Allow (grant {grant_id})");
        }
        other => panic!("expected reuse Allow, got {other:?}"),
    }

    // 4) Durability: reopen the store from disk (simulate desktop restart).
    drop(grants);
    let grants2 = GrantStore::open(&grant_path).expect("reopen grant store");
    let d3 = engine.authorize(&auth(), &grants2);
    assert!(d3.is_allow(), "grant must survive reopen; got {d3:?}");
    eprintln!("[B] post-restart authorize = Allow (durable)");

    // 5) Revoke ⇒ fresh prompt (re-prompt).
    engine.revoke(&grant_id, &grants2).expect("revoke grant");
    let d4 = engine.authorize(&auth(), &grants2);
    assert!(d4.is_prompt(), "revoke must re-prompt; got {d4:?}");
    eprintln!("[B] post-revoke authorize = Prompt (re-prompt)");

    // Cleanup + leak baseline.
    pool.shutdown().await.expect("pool shutdown");
    let _ = std::fs::remove_file(&tmp_db);
    let _ = std::fs::remove_file(&grant_path);
}
