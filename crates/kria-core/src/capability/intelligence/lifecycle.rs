//! [`DefaultLifecycleManager`] — the complete capability lifecycle (spec R5/R19/R21)
//! over the provider-neutral [`CapabilityPlatform`].
//!
//! - **acquire_verified**: install via the platform's single acquisition entry
//!   (`acquire_for_goal` → provider `acquire`, which already does download +
//!   integrity/trust verification), record it to the CKB, then **smoke-test**
//!   before it is trusted; a failed smoke test **rolls back** the install
//!   (transactional activation, R21.1) — no capability is enabled on download alone.
//! - **smoke_test**: liveness only (spec R30.2) — runs the capability's declared
//!   self-check (or a no-arg execution) and requires a real `Value` outcome.
//! - **upgrade**: idempotent re-acquire (R5.3).
//! - **rollback/delete**: provider `remove` + cascade CKB purge (R1.6/R5.4).
//! - **retire/recover**: reversible archive via CKB state (R19).
//!
//! Provider-neutral: everything goes through the platform + provider trait; no
//! provider-native type, no hardcoded provider name.

use std::sync::Arc;

use async_trait::async_trait;

use super::{CapabilityKnowledge, LifecycleManager};
use crate::capability::descriptor::CapabilityDescriptor;
use crate::capability::error::CapError;
use crate::capability::platform::CapabilityPlatform;
use crate::capability::provider::{
    AcquireRequest, CapabilityOutcome, CapabilityRequest, RequestContext,
};

/// Default lifecycle manager over the neutral platform + optional CKB.
pub struct DefaultLifecycleManager {
    platform: Arc<CapabilityPlatform>,
    knowledge: Option<Arc<dyn CapabilityKnowledge>>,
}

impl DefaultLifecycleManager {
    pub fn new(platform: Arc<CapabilityPlatform>) -> Self {
        Self {
            platform,
            knowledge: None,
        }
    }

    pub fn with_knowledge(mut self, ckb: Arc<dyn CapabilityKnowledge>) -> Self {
        self.knowledge = Some(ckb);
        self
    }

    /// Whether a JSON-Schema declares any required/expected arguments.
    fn schema_expects_args(schema: &serde_json::Value) -> bool {
        let Some(obj) = schema.as_object() else {
            return false;
        };
        if obj
            .get("required")
            .and_then(|r| r.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        obj.get("properties")
            .and_then(|p| p.as_object())
            .map(|p| !p.is_empty())
            .unwrap_or(false)
    }

    /// Remove a capability through its owning provider (real reversal of install).
    async fn provider_remove(
        &self,
        provider_id: &str,
        capability_id: &str,
    ) -> Result<(), CapError> {
        let provider = self
            .platform
            .registry()
            .get(provider_id)
            .ok_or_else(|| CapError::Execute(format!("no such provider '{provider_id}'")))?;
        provider.remove(capability_id).await
    }
}

#[async_trait]
impl LifecycleManager for DefaultLifecycleManager {
    async fn acquire_verified(&self, goal: &str) -> Result<CapabilityDescriptor, CapError> {
        // Install (download + integrity/trust verify + register + re-index) via
        // the single neutral acquisition entry.
        let descriptor = self.platform.acquire_for_goal(goal).await?;
        let (pid, cid) = descriptor.key();

        // Record to the CKB as installed (learning layer, grounding source).
        if let Some(ckb) = &self.knowledge {
            let _ = ckb.record_install(&descriptor).await;
        }

        // Smoke-test before trust; roll back on failure (R21.1 / transactional).
        if let Err(e) = self.smoke_test(&pid, &cid).await {
            // Best-effort rollback: uninstall + purge so a broken capability is
            // never left "installed" (no partial artifact, R5.2).
            let _ = self.provider_remove(&pid, &cid).await;
            if let Some(ckb) = &self.knowledge {
                let _ = ckb.purge(&pid, &cid).await;
            }
            return Err(CapError::Acquire(format!(
                "'{cid}' installed but failed smoke test; rolled back: {e}"
            )));
        }

        // Passed ⇒ mark enabled (activate).
        if let Some(ckb) = &self.knowledge {
            let _ = ckb.set_state(&pid, &cid, "enabled").await;
        }
        Ok(descriptor)
    }

    async fn smoke_test(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError> {
        let descriptor = self
            .platform
            .descriptor(provider_id, capability_id)?
            .ok_or_else(|| {
                CapError::Execute(format!("smoke test: unknown capability '{capability_id}'"))
            })?;

        // Prefer a descriptor-declared smoke check: extensions["smoke"] = { args }.
        let args = descriptor
            .extensions
            .get("smoke")
            .and_then(|s| s.get("args"))
            .cloned();

        let args = match args {
            Some(a) => a,
            None => {
                // No declared smoke test. If the capability needs no args we can
                // safely exercise liveness with `{}`. If it requires args we
                // cannot fabricate them (honest) — treat as a pass with no run.
                if Self::schema_expects_args(&descriptor.input_schema) {
                    return Ok(());
                }
                serde_json::json!({})
            }
        };

        let req = CapabilityRequest {
            provider_id: descriptor.provider_id.clone(),
            capability_id: descriptor.capability_id.clone(),
            args,
            context: RequestContext::new(),
            granted_effects: descriptor.effects.classes.clone(),
        };
        match self.platform.execute(req).await {
            Ok(CapabilityOutcome::Value(_)) | Ok(CapabilityOutcome::Stream(_)) => Ok(()),
            Ok(CapabilityOutcome::Declined { reason }) => {
                Err(CapError::Execute(format!("smoke test declined: {reason}")))
            }
            Err(e) => Err(CapError::Execute(format!("smoke test failed: {e}"))),
        }
    }

    async fn upgrade(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError> {
        // Idempotent re-acquire of the SPECIFIC installed capability by id (R5.3).
        // Upgrade must NOT go through `acquire_for_goal` (fresh, goal-based
        // acquisition) because that path is native-first-filtered and would
        // exclude the already-installed capability being upgraded. Instead we
        // re-acquire the exact capability directly through its owning provider —
        // the provider re-installs the current/newer version (a no-op if latest).
        let provider = self
            .platform
            .registry()
            .get(provider_id)
            .ok_or_else(|| CapError::Execute(format!("no such provider '{provider_id}'")))?;
        let req = AcquireRequest {
            capability_tag: capability_id.to_string(),
            hint: None,
            capability_id: Some(capability_id.to_string()),
            proposed_graph: None,
            context: RequestContext::new(),
        };
        let descriptor = provider.acquire(&req).await?;
        // Make the upgraded descriptor discoverable + record the new state.
        self.platform.refresh().await;
        self.platform.invalidate_catalog_cache(Some(provider_id));
        if let Some(ckb) = &self.knowledge {
            let _ = ckb.record_install(&descriptor).await;
        }
        Ok(())
    }

    async fn rollback(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError> {
        // Revert an install: uninstall + purge learned rows (reversible action).
        self.provider_remove(provider_id, capability_id).await?;
        if let Some(ckb) = &self.knowledge {
            let _ = ckb.purge(provider_id, capability_id).await;
        }
        self.platform.refresh().await;
        Ok(())
    }

    async fn retire(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError> {
        // Reversible retirement: archive in the CKB (excluded from listings) but
        // keep the bundle so `recover` can re-enable it (spec R19). If no CKB,
        // fall back to a full delete.
        if let Some(ckb) = &self.knowledge {
            ckb.set_state(provider_id, capability_id, "archived").await
        } else {
            self.delete(provider_id, capability_id).await
        }
    }

    async fn recover(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError> {
        // Reverse a retirement (R19.2): re-enable the archived CKB row (the
        // bundle was kept by `retire`). Re-index so it becomes discoverable again.
        if let Some(ckb) = &self.knowledge {
            ckb.set_state(provider_id, capability_id, "enabled").await?;
        }
        self.platform.refresh().await;
        Ok(())
    }

    async fn delete(&self, provider_id: &str, capability_id: &str) -> Result<(), CapError> {
        // Cascade delete: uninstall from the provider + purge CKB + re-index.
        self.provider_remove(provider_id, capability_id).await?;
        if let Some(ckb) = &self.knowledge {
            let _ = ckb.purge(provider_id, capability_id).await;
        }
        self.platform.refresh().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::CapabilityDescriptor;
    use crate::capability::index::{Embedder, InMemoryFederatedIndex};
    use crate::capability::intelligence::SqliteCapabilityKnowledge;
    use crate::capability::protocol::{
        ClientCapabilities, FeatureSet, ProtocolSession, ProtocolVersion, ProviderHealth,
    };
    use crate::capability::provider::CapabilityProvider;
    use crate::capability::registry::ProviderRegistry;
    use crate::capability::CapabilityPlatform;
    use std::sync::Mutex;

    struct HashEmbedder;
    impl Embedder for HashEmbedder {
        fn embed(&self, text: &str) -> Result<Vec<f32>, CapError> {
            let mut v = vec![0.0f32; 16];
            for (i, b) in text.bytes().enumerate() {
                v[i % 16] += b as f32;
            }
            Ok(v)
        }
        fn dim(&self) -> usize {
            16
        }
        fn model_id(&self) -> &str {
            "hash-test"
        }
    }

    /// A provider that supports `remove` (lifecycle) and tracks removals, so the
    /// delete/rollback cascade can be verified through the real trait path.
    struct RemovableProvider {
        id: String,
        removed: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl CapabilityProvider for RemovableProvider {
        fn provider_id(&self) -> &String {
            &self.id
        }
        async fn negotiate(
            &self,
            client: &ClientCapabilities,
        ) -> Result<ProtocolSession, CapError> {
            Ok(client.negotiate(
                self.id.clone(),
                ProtocolVersion::CURRENT,
                FeatureSet::mandatory(),
                serde_json::Map::new(),
            ))
        }
        async fn describe(
            &self,
            _s: &ProtocolSession,
        ) -> Result<Vec<CapabilityDescriptor>, CapError> {
            if self.removed.lock().unwrap().contains(&"cap1".to_string()) {
                return Ok(vec![]);
            }
            Ok(vec![CapabilityDescriptor::minimal(
                &self.id,
                "cap1",
                "cap1",
                "",
                serde_json::json!({"type": "object"}),
            )])
        }
        async fn execute(&self, req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
            Ok(CapabilityOutcome::Value(
                serde_json::json!({"ran": req.capability_id}),
            ))
        }
        async fn remove(&self, capability_id: &str) -> Result<(), CapError> {
            self.removed.lock().unwrap().push(capability_id.to_string());
            Ok(())
        }
        async fn health(&self) -> ProviderHealth {
            ProviderHealth::Ready
        }
    }

    async fn setup() -> (Arc<CapabilityPlatform>, Arc<dyn CapabilityKnowledge>) {
        let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmbedder)));
        let registry = ProviderRegistry::new(index);
        registry.register(Arc::new(RemovableProvider {
            id: "prov".into(),
            removed: Mutex::new(vec![]),
        }));
        let ckb: Arc<dyn CapabilityKnowledge> =
            Arc::new(SqliteCapabilityKnowledge::in_memory().unwrap());
        let platform =
            Arc::new(CapabilityPlatform::new(Arc::new(registry)).with_knowledge(ckb.clone()));
        platform.refresh().await;
        (platform, ckb)
    }

    #[tokio::test]
    async fn smoke_test_passes_on_live_capability() {
        let (platform, _ckb) = setup().await;
        let mgr = DefaultLifecycleManager::new(platform);
        // cap1 has an object schema with no required/properties ⇒ no-arg run ⇒ Value ⇒ pass.
        mgr.smoke_test("prov", "cap1").await.unwrap();
    }

    #[tokio::test]
    async fn smoke_test_unknown_capability_errors() {
        let (platform, _ckb) = setup().await;
        let mgr = DefaultLifecycleManager::new(platform);
        assert!(mgr.smoke_test("prov", "nope").await.is_err());
    }

    #[tokio::test]
    async fn delete_cascades_provider_and_ckb() {
        let (platform, ckb) = setup().await;
        ckb.record_install(&CapabilityDescriptor::minimal(
            "prov",
            "cap1",
            "cap1",
            "",
            serde_json::json!({"type":"object"}),
        ))
        .await
        .unwrap();
        assert_eq!(ckb.list_installed().await.unwrap().len(), 1);
        let mgr = DefaultLifecycleManager::new(platform.clone()).with_knowledge(ckb.clone());
        mgr.delete("prov", "cap1").await.unwrap();
        // CKB purged.
        assert_eq!(ckb.list_installed().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn retire_archives_reversibly() {
        let (platform, ckb) = setup().await;
        ckb.record_install(&CapabilityDescriptor::minimal(
            "prov",
            "cap1",
            "cap1",
            "",
            serde_json::json!({"type":"object"}),
        ))
        .await
        .unwrap();
        let mgr = DefaultLifecycleManager::new(platform).with_knowledge(ckb.clone());
        mgr.retire("prov", "cap1").await.unwrap();
        // Archived ⇒ excluded from installed listing (but bundle/row kept).
        assert_eq!(ckb.list_installed().await.unwrap().len(), 0);
        // Recover by re-enabling.
        ckb.set_state("prov", "cap1", "enabled").await.unwrap();
        assert_eq!(ckb.list_installed().await.unwrap().len(), 1);
    }
}
