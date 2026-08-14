//! OpenClaw provider `execute` → `LaunchSpec` wiring regression.
//!
//! Proves the correctness/security fix in `OpenClawProvider::execute`
//! (`capability/acl/openclaw.rs`): the `LaunchSpec` handed to the runtime is
//! built from the authoritative registry + the per-run permission grant, NOT
//! hardcoded. Specifically:
//!   1. `resource_class` = the skill's declared class (not always `Light`).
//!   2. `grants` = the skill's authoritative `granted_capabilities`, FILTERED to
//!      only the effect classes the permission engine granted for THIS run
//!      (`req.granted_effects`) — the provider must never exceed the run grant.
//!   3. `mounted_skill_dir` = the installed bundle's `<bundle_path>/.bridge`
//!      handler dir (only when it exists on disk); `None` for baked skills.
//!
//! No Docker required: a capturing `SkillRuntime` records the exact `LaunchSpec`
//! the provider assembled, so the wiring is asserted directly and deterministically.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use kria_core::capability::acl::openclaw::OpenClawProvider;
use kria_core::capability::provider::{CapabilityProvider, CapabilityRequest, RequestContext};
use kria_core::infra::isolation::ToolResult;
use kria_core::openclaw::capability::{
    grant_all, Capability, CapabilityKind, CapabilityMode, CapabilityScope, GrantSource,
};
use kria_core::openclaw::registry::{
    DiscoverySource, ProductionSkillRegistry, SkillMetadata, SkillState,
};
use kria_core::openclaw::runtime::{LaunchSpec, RuntimeContext, RuntimeKind, SkillRuntime};
use kria_core::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};
use kria_core::safety::RiskLevel;

/// Runtime that captures every `LaunchSpec` it is asked to run and reports
/// success, so the provider's spec assembly can be asserted without Docker.
struct CapturingRuntime {
    seen: Arc<Mutex<Vec<LaunchSpec>>>,
}

#[async_trait]
impl SkillRuntime for CapturingRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Docker
    }
    async fn execute(&self, spec: LaunchSpec, _ctx: RuntimeContext) -> ToolResult {
        self.seen.lock().unwrap().push(spec);
        ToolResult::ok(serde_json::json!({"ok": true}))
    }
}

fn install_skill(
    registry: &ProductionSkillRegistry,
    skill_id: &str,
    resource_class: ResourceClass,
    grants: Vec<kria_core::openclaw::capability::CapabilityGrant>,
    bundle_path: Option<String>,
) {
    let now = chrono::Utc::now();
    let meta = SkillMetadata {
        skill_id: skill_id.to_string(),
        name: skill_id.to_string(),
        description: "wiring test skill".to_string(),
        publisher: "test".to_string(),
        version: "1.0.0".to_string(),
        category: "general".to_string(),
        discovery_source: DiscoverySource::Bundled {
            path: "test".to_string(),
        },
        discovered_at: now,
        capabilities: SkillCapabilities::default(),
        runtime_requirements: "docker".to_string(),
        risk_level: RiskLevel::Yellow,
        resource_class,
        tags: Vec::new(),
        categories: Vec::new(),
        semantic_version: "1.0.0".to_string(),
        dependencies: Vec::new(),
        compatibility_requirements: Vec::new(),
        trust_tier: TrustTier::Community,
        content_hash: "hash".to_string(),
        signature: None,
        granted_capabilities: grants,
        bundle_path,
        manifest_toml: None,
        input_schema: None,
        state: SkillState::Enabled,
        state_changed_at: now,
    };
    registry.install_skill(&meta).expect("install skill");
}

fn cap(kind: CapabilityKind, mode: CapabilityMode, scope: CapabilityScope) -> Capability {
    Capability { kind, mode, scope }
}

#[test]
fn launchspec_threads_resource_class_grants_filtered_and_mount() {
    let tmp = std::env::temp_dir().join(format!("kria_ocwire_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let registry =
        Arc::new(ProductionSkillRegistry::new(&tmp.join("skills.db")).expect("registry"));

    // Installed skill: Heavy, with a ReadWrite-filesystem grant + a Network
    // grant, and a real bundle dir whose `.bridge/` exists on disk.
    let bundle_dir = tmp.join("store").join("installed_skill").join("1.0.0");
    std::fs::create_dir_all(bundle_dir.join(".bridge")).unwrap();
    let caps = vec![
        cap(
            CapabilityKind::Filesystem,
            CapabilityMode::ReadWrite,
            CapabilityScope::Workspace,
        ),
        cap(
            CapabilityKind::Network,
            CapabilityMode::Egress,
            CapabilityScope::Domains(vec!["api.example.com".into()]),
        ),
    ];
    let grants = grant_all(&caps, GrantSource::UserApproval, true);
    install_skill(
        &registry,
        "installed_skill",
        ResourceClass::Heavy,
        grants,
        Some(bundle_dir.to_string_lossy().to_string()),
    );

    // Baked skill: not present in the registry at all.
    let seen = Arc::new(Mutex::new(Vec::<LaunchSpec>::new()));
    let runtime: Arc<dyn SkillRuntime> = Arc::new(CapturingRuntime { seen: seen.clone() });
    let provider = OpenClawProvider::new(registry.clone(), runtime);

    let rt = tokio::runtime::Runtime::new().unwrap();

    // ── Case 1: run grants only "read" + "network". The ReadWrite fs grant
    //    surfaces BOTH "read" and "write" → "write" ungranted → filtered OUT.
    //    The Network grant ("network") is granted → kept.
    rt.block_on(async {
        provider
            .execute(CapabilityRequest {
                provider_id: "openclaw".into(),
                capability_id: "installed_skill".into(),
                args: serde_json::json!({}),
                context: RequestContext::new(),
                granted_effects: vec!["read".into(), "network".into()],
            })
            .await
            .expect("execute ok");
    });
    {
        let specs = seen.lock().unwrap();
        let spec = specs.last().expect("a spec was captured");
        assert_eq!(
            spec.resource_class,
            ResourceClass::Heavy,
            "resource_class must come from the skill, not hardcoded Light"
        );
        assert_eq!(
            spec.grants.len(),
            1,
            "only the network grant survives run-grant filtering (write not granted)"
        );
        assert_eq!(spec.grants[0].capability.kind, CapabilityKind::Network);
        let bridge = bundle_dir.join(".bridge");
        assert_eq!(
            spec.mounted_skill_dir.as_deref(),
            Some(bridge.as_path()),
            "installed skill must mount its .bridge handler dir"
        );
    }

    // ── Case 2: run grants "read" + "write" + "network" → the ReadWrite fs
    //    grant now passes too (both classes granted). Both grants present.
    rt.block_on(async {
        provider
            .execute(CapabilityRequest {
                provider_id: "openclaw".into(),
                capability_id: "installed_skill".into(),
                args: serde_json::json!({}),
                context: RequestContext::new(),
                granted_effects: vec!["read".into(), "write".into(), "network".into()],
            })
            .await
            .expect("execute ok");
    });
    {
        let specs = seen.lock().unwrap();
        let spec = specs.last().unwrap();
        assert_eq!(
            spec.grants.len(),
            2,
            "both fs(read+write) and network grants pass when all classes granted"
        );
    }

    // ── Case 3: a baked skill absent from the registry keeps the conservative
    //    legacy defaults: Light, no grants, no mount.
    rt.block_on(async {
        provider
            .execute(CapabilityRequest {
                provider_id: "openclaw".into(),
                capability_id: "baked_only_skill".into(),
                args: serde_json::json!({}),
                context: RequestContext::new(),
                granted_effects: vec!["read".into(), "write".into()],
            })
            .await
            .expect("execute ok");
    });
    {
        let specs = seen.lock().unwrap();
        let spec = specs.last().unwrap();
        assert_eq!(spec.resource_class, ResourceClass::Light);
        assert!(spec.grants.is_empty(), "baked skill has no grants");
        assert!(
            spec.mounted_skill_dir.is_none(),
            "baked skill mounts nothing"
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
}
