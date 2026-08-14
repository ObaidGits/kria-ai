//! Task 0.3 — "Reconcile the two existing policy paths" authority invariants.
//!
//! These are code-level, fake-backed tests (no live OS mutation) proving the
//! design §2.1 reconciliation (OSC-001/OSC-002/OSC-004): `ExecutionGate` is the
//! ONE native-OS admission authority, and the extension capability plane
//! (`CapabilityPlatform`, `DefaultPermissionEngine`, `GrantStore`) plus the
//! command-policy defence layer can neither approve, execute, nor broaden a
//! native host-OS operation.

use std::sync::Arc;

use async_trait::async_trait;

use kria_core::agent::os_action_authority::{
    effects_request_native_os, is_native_os_action, NATIVE_OS_EFFECT,
};
use kria_core::capability::descriptor::{CapabilityDescriptor, Effects, Reversibility};
use kria_core::capability::grants::{GrantDecision, GrantStore, ScopeKind, ScopedGrant};
use kria_core::capability::index::{InMemoryFederatedIndex, MemoryEmbedder};
use kria_core::capability::permission::{
    AuthorizeRequest, DefaultPermissionEngine, PermissionDecision, PermissionEngine,
};
use kria_core::capability::platform::CapabilityPlatform;
use kria_core::capability::protocol::{
    ClientCapabilities, FeatureSet, ProtocolSession, ProtocolVersion, ProviderHealth,
};
use kria_core::capability::provider::{
    CapabilityOutcome, CapabilityProvider, CapabilityRequest, RequestContext,
};
use kria_core::capability::registry::ProviderRegistry;
use kria_core::capability::{CapError, ProviderId};
use kria_core::safety::policy_gate::{
    ArgPattern, CapabilityPolicyGate, CustomRule, PolicyDecision, PolicyGate,
};

/// A fake provider that advertises a native host-OS capability (declares the
/// `os.native` effect) alongside a benign one. It must never be executed for the
/// native-OS capability through the plane.
struct RogueOsProvider {
    id: ProviderId,
}

#[async_trait]
impl CapabilityProvider for RogueOsProvider {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }
    async fn negotiate(&self, client: &ClientCapabilities) -> Result<ProtocolSession, CapError> {
        Ok(client.negotiate(
            self.id.clone(),
            ProtocolVersion::CURRENT,
            FeatureSet::mandatory(),
            serde_json::Map::new(),
        ))
    }
    async fn describe(&self, _s: &ProtocolSession) -> Result<Vec<CapabilityDescriptor>, CapError> {
        let mut native = CapabilityDescriptor::minimal(
            self.id.clone(),
            "reboot_the_host",
            "Reboot The Host",
            "Attempts to reboot the machine (native host OS effect).",
            serde_json::json!({ "type": "object", "properties": {} }),
        );
        native.effects = Effects {
            classes: vec![NATIVE_OS_EFFECT.to_string()],
            reversible: Reversibility::Irreversible,
            idempotent: false,
            resource_class: Default::default(),
        };
        Ok(vec![native])
    }
    async fn execute(&self, _req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
        // If this ever runs, the plane exclusion has failed.
        panic!("RogueOsProvider::execute must never be reached for a native-OS capability");
    }
    async fn health(&self) -> ProviderHealth {
        ProviderHealth::Ready
    }
}

fn native_effects() -> Vec<String> {
    vec![NATIVE_OS_EFFECT.to_string()]
}

#[tokio::test]
async fn capability_platform_cannot_execute_a_native_os_operation() {
    let embedder = Arc::new(MemoryEmbedder::load().unwrap());
    let index = Arc::new(InMemoryFederatedIndex::new(embedder));
    let registry = Arc::new(ProviderRegistry::new(index));
    registry.register(Arc::new(RogueOsProvider {
        id: "rogue".to_string(),
    }));
    let platform = CapabilityPlatform::new(registry);
    platform.refresh().await;

    // Discovery excludes the native-OS descriptor entirely.
    let all = platform.discover("", 1000).unwrap_or_default();
    assert!(
        all.iter()
            .all(|s| s.descriptor.capability_id != "reboot_the_host"),
        "native-OS descriptor must be excluded from the capability plane"
    );

    // Even a direct execute request (with the native effect granted) is refused
    // before the provider is touched.
    let req = CapabilityRequest {
        provider_id: "rogue".to_string(),
        capability_id: "reboot_the_host".to_string(),
        args: serde_json::json!({}),
        context: RequestContext::new(),
        granted_effects: native_effects(),
    };
    let result = platform.execute(req).await;
    match result {
        Err(CapError::Permission(msg)) => {
            assert!(msg.contains("native host-OS"), "unexpected message: {msg}");
        }
        other => panic!("expected Permission refusal, got {other:?}"),
    }
}

#[test]
fn extension_permission_engine_denies_native_os_effects() {
    let grants = GrantStore::in_memory().unwrap();
    let engine = DefaultPermissionEngine;
    let req = AuthorizeRequest {
        provider_id: "rogue".to_string(),
        capability_id: "reboot_the_host".to_string(),
        effects: Effects {
            classes: native_effects(),
            reversible: Reversibility::Irreversible,
            idempotent: false,
            resource_class: Default::default(),
        },
        session_id: None,
        workspace_id: Some("default".to_string()),
    };
    match engine.authorize(&req, &grants) {
        PermissionDecision::Deny { reason } => {
            assert!(
                reason.contains("native host-OS"),
                "unexpected reason: {reason}"
            );
        }
        other => panic!("expected Deny for native-OS effects, got {other:?}"),
    }
}

#[test]
fn extension_grant_store_refuses_to_persist_native_os_authority() {
    let grants = GrantStore::in_memory().unwrap();
    let grant = ScopedGrant {
        grant_id: "g-native".to_string(),
        provider_id: "rogue".to_string(),
        capability_id: "reboot_the_host".to_string(),
        scope_kind: ScopeKind::Persistent,
        scope_key: None,
        effects: native_effects(),
        decision: GrantDecision::Allow,
        granted_at: chrono::Utc::now(),
        expires_at: None,
        revoked: false,
    };
    assert!(
        grants.insert(&grant).is_err(),
        "grant store must refuse to persist native host-OS authority"
    );
}

#[test]
fn command_policy_is_subordinate_and_cannot_be_unblocked_by_a_custom_rule() {
    let gate = CapabilityPolicyGate::new();

    // Generic `reboot` binary stays blocked (it is not a typed OS action).
    assert!(
        matches!(gate.evaluate("reboot", &[]), PolicyDecision::Blocked { .. }),
        "generic `reboot` must remain blocked"
    );

    // BLACK-scope administration stays blocked.
    assert!(matches!(
        gate.evaluate("mkfs.ext4", &["/dev/sdb1".to_string()]),
        PolicyDecision::Blocked { .. }
    ));

    // A runtime custom rule cannot un-block a blocked binary or BLACK-scope op:
    // hard denials run first, so the rule never takes effect.
    gate.add_custom_rule(CustomRule {
        binary: "reboot".to_string(),
        arg_pattern: ArgPattern::Any,
        decision: PolicyDecision::AutoApproved {
            risk_level: kria_core::safety::RiskLevel::Green,
            capabilities: Default::default(),
        },
        description: "attempt to broaden authority".to_string(),
        expires_at: None,
    });
    gate.add_custom_rule(CustomRule {
        binary: "mkfs.ext4".to_string(),
        arg_pattern: ArgPattern::Any,
        decision: PolicyDecision::AutoApproved {
            risk_level: kria_core::safety::RiskLevel::Green,
            capabilities: Default::default(),
        },
        description: "attempt to broaden authority".to_string(),
        expires_at: None,
    });

    assert!(
        matches!(gate.evaluate("reboot", &[]), PolicyDecision::Blocked { .. }),
        "custom rule must NOT be able to un-block `reboot`"
    );
    assert!(
        matches!(
            gate.evaluate("mkfs.ext4", &["/dev/sdb1".to_string()]),
            PolicyDecision::Blocked { .. }
        ),
        "custom rule must NOT be able to un-block BLACK-scope administration"
    );
}

#[test]
fn native_os_action_classification_separates_typed_from_generic() {
    // Typed native-OS tools are recognized; generic execution is not.
    assert!(is_native_os_action("reboot_system"));
    assert!(is_native_os_action("toggle_wifi"));
    assert!(!is_native_os_action("reboot"));
    assert!(!is_native_os_action("execute_bash"));
    assert!(!is_native_os_action("openclaw"));

    // The native-OS effect marker is detected; generic extension effects pass.
    assert!(effects_request_native_os(&native_effects()));
    assert!(!effects_request_native_os(&[
        "read".to_string(),
        "write".to_string(),
        "network".to_string(),
    ]));
}
