//! Milestone 5 acquisition-via-LIFECYCLE real validation.
//!
//! Drives the OpenClaw provider's LIFECYCLE facet end-to-end against the REAL
//! marketplace (`ObaidGits/kria-skills`): negotiate advertises Lifecycle →
//! `acquire` finds + downloads + transpiles + installs a real skill via the
//! frozen `BundleInstaller` → returns its refreshed descriptor → `remove`
//! uninstalls it. No Docker needed (install is download+synth+register); no
//! fixtures — the real repo + real installer pipeline.
//!
//! Gated on `KRIA_CPP_NET=1` (needs network to raw.githubusercontent.com). Run:
//! ```bash
//! KRIA_CPP_NET=1 cargo test -p kria-core --test capability_acquire_marketplace -- --nocapture
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use kria_core::capability::acl::openclaw::OpenClawProvider;
use kria_core::capability::protocol::ClientCapabilities;
use kria_core::capability::provider::{AcquireRequest, RequestContext};
use kria_core::capability::CapabilityProvider;
use kria_core::infra::isolation::ToolResult;
use kria_core::openclaw::audit::AuditLedger;
use kria_core::openclaw::registry::ProductionSkillRegistry;
use kria_core::openclaw::runtime::{LaunchSpec, RuntimeContext, RuntimeKind, SkillRuntime};

/// A no-op runtime: acquisition never executes, so execute is unreachable here.
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

#[tokio::test]
async fn openclaw_lifecycle_acquire_and_remove_real_marketplace() {
    if std::env::var("KRIA_CPP_NET").is_err() {
        eprintln!("skipping: set KRIA_CPP_NET=1 (needs network to the skills repo)");
        return;
    }

    let dir = std::env::temp_dir().join(format!("kria_cpp_acq_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("skills.db");
    let store = dir.join("store");
    std::fs::create_dir_all(&store).unwrap();

    let registry = Arc::new(ProductionSkillRegistry::new(&db).expect("registry"));
    let audit =
        Arc::new(AuditLedger::open(&db, b"kria-cpp-test-audit-key-0001".to_vec()).expect("audit"));
    let runtime: Arc<dyn SkillRuntime> = Arc::new(NullRuntime);

    let provider = OpenClawProvider::new(registry.clone(), runtime).with_lifecycle(
        "https://raw.githubusercontent.com/ObaidGits/kria-skills/refs/heads/main/index.json",
        vec!["raw.githubusercontent.com".to_string()],
        audit,
        store,
    );

    // Negotiation must now advertise the LIFECYCLE facet.
    let session = provider
        .negotiate(&ClientCapabilities::default())
        .await
        .expect("negotiate");
    assert!(
        session.supports_lifecycle(),
        "lifecycle must be advertised when wired"
    );

    // Acquire a skill matching a Python-code-execution goal → real install.
    let descriptor = provider
        .acquire(&AcquireRequest {
            capability_tag: "code.execute.python".to_string(),
            hint: Some("execute python code snippet sandbox".to_string()),
            context: RequestContext::new(),
        })
        .await
        .expect("acquire should install a matching marketplace skill");

    eprintln!(
        "acquired: {}/{}",
        descriptor.provider_id, descriptor.capability_id
    );
    assert_eq!(descriptor.provider_id, "openclaw");
    assert_eq!(descriptor.capability_id, "oc_code_sandbox");
    descriptor.validate().expect("acquired descriptor valid");

    // It is now in the registry (descriptor refresh / persistence).
    assert!(
        registry.get_skill("oc_code_sandbox").is_ok(),
        "installed skill present in registry"
    );

    // Remove it (lifecycle removal).
    provider.remove("oc_code_sandbox").await.expect("remove");
    // After uninstall it should no longer be an enabled/usable skill.
    let still_enabled = registry
        .get_enabled_skills()
        .unwrap_or_default()
        .iter()
        .any(|s| s.skill_id == "oc_code_sandbox");
    assert!(!still_enabled, "removed skill must not remain enabled");

    let _ = std::fs::remove_dir_all(&dir);
}
