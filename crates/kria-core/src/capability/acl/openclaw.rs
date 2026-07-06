//! `OpenClawProvider` — OpenClaw expressed as a [`CapabilityProvider`].
//!
//! This adapter is the **sole location** in KRIA where OpenClaw-internal types
//! (`ProductionSkillRegistry`, `SkillMetadata`, `SkillCapabilities`,
//! `LaunchSpec`, `SkillRuntime`, …) are referenced. It translates them into the
//! neutral CPP domain types so the Brain treats OpenClaw exactly like any other
//! provider.
//!
//! # What it wraps
//!
//! - the frozen [`ProductionSkillRegistry`] (OpenClaw's authoritative catalog)
//!   for [`describe`](CapabilityProvider::describe), and
//! - a frozen [`SkillRuntime`] (the Docker runtime built from the container
//!   pool) for [`execute`](CapabilityProvider::execute).
//!
//! The execute mapping mirrors the frozen `execution::executors::OpenClawExecutor`
//! (build a [`LaunchSpec`], call the runtime), so behavior is identical to the
//! proven path — this adapter does not invent a second execution route.
//!
//! # Facets (Milestone 2)
//!
//! Advertises the mandatory facets (describe/discover/execute). The optional
//! **lifecycle** facet (marketplace install + A9 generation via the frozen
//! `BundleInstaller`) is wired in a later milestone; until then the adapter
//! honestly does not advertise it, so acquisition is simply not offered for
//! OpenClaw yet (never a fake install).

use async_trait::async_trait;
use std::sync::Arc;

use crate::capability::descriptor::{
    CapabilityDescriptor, CapabilityTag, Effect, Effects, ResourceClass, Reversibility, TrustInfo,
};
use crate::capability::error::CapError;
use crate::capability::protocol::{
    ClientCapabilities, FeatureSet, ProtocolSession, ProtocolVersion, ProviderHealth,
};
use crate::capability::provider::{CapabilityOutcome, CapabilityProvider, CapabilityRequest};
use crate::capability::ProviderId;

use crate::openclaw::audit::AuditLedger;
use crate::openclaw::registry::{ProductionSkillRegistry, SkillMetadata};
use crate::openclaw::runtime::{LaunchSpec, RuntimeContext, RuntimeKind, SkillRuntime};
use crate::openclaw::types::{ResourceClass as OcResourceClass, SkillCapabilities};

/// The canonical provider id for OpenClaw.
pub const OPENCLAW_PROVIDER_ID: &str = "openclaw";

/// Dependencies enabling the optional LIFECYCLE facet (marketplace acquisition +
/// removal). Present only when the adapter is built via [`OpenClawProvider::with_lifecycle`];
/// otherwise `acquire`/`remove` honestly report `Unsupported`.
struct LifecycleDeps {
    index_url: String,
    allowed_hosts: Vec<String>,
    audit: Arc<AuditLedger>,
    store_dir: std::path::PathBuf,
}

/// OpenClaw as a capability provider.
pub struct OpenClawProvider {
    id: ProviderId,
    registry: Arc<ProductionSkillRegistry>,
    runtime: Arc<dyn SkillRuntime>,
    default_timeout: std::time::Duration,
    lifecycle: Option<LifecycleDeps>,
}

impl OpenClawProvider {
    /// Build the adapter from the frozen registry + a skill runtime (the Docker
    /// runtime built from the container pool).
    pub fn new(registry: Arc<ProductionSkillRegistry>, runtime: Arc<dyn SkillRuntime>) -> Self {
        Self {
            id: OPENCLAW_PROVIDER_ID.to_string(),
            registry,
            runtime,
            default_timeout: std::time::Duration::from_secs(120),
            lifecycle: None,
        }
    }

    /// Enable the LIFECYCLE facet: marketplace acquisition via the frozen
    /// `ClawHubClient` + `BundleInstaller`, and removal via the registry. When
    /// wired, [`negotiate`](CapabilityProvider::negotiate) advertises
    /// [`Feature::Lifecycle`](crate::capability::protocol::Feature::Lifecycle) so
    /// the platform offers acquisition for this provider.
    pub fn with_lifecycle(
        mut self,
        index_url: impl Into<String>,
        allowed_hosts: Vec<String>,
        audit: Arc<AuditLedger>,
        store_dir: std::path::PathBuf,
    ) -> Self {
        self.lifecycle = Some(LifecycleDeps {
            index_url: index_url.into(),
            allowed_hosts,
            audit,
            store_dir,
        });
        self
    }

    /// Map OpenClaw's per-flag [`SkillCapabilities`] into open, neutral effect
    /// class strings. Open vocabulary: adding a capability flag upstream just
    /// adds a string here, never a new enum in the boundary.
    fn effect_classes(caps: &SkillCapabilities) -> Vec<Effect> {
        let mut classes = Vec::new();
        if caps.filesystem_read {
            classes.push("read".to_string());
        }
        if caps.filesystem_write {
            classes.push("write".to_string());
        }
        if caps.subprocess {
            classes.push("subprocess".to_string());
        }
        if caps.browser {
            classes.push("browser".to_string());
        }
        if caps.network {
            classes.push("network".to_string());
        }
        if caps.image_generation {
            classes.push("image_generation".to_string());
        }
        if caps.media {
            classes.push("media".to_string());
        }
        classes
    }

    /// Map OpenClaw's resource class to the neutral mirror.
    fn map_resource_class(rc: OcResourceClass) -> ResourceClass {
        match rc {
            OcResourceClass::Light => ResourceClass::Light,
            OcResourceClass::Medium => ResourceClass::Medium,
            OcResourceClass::Heavy => ResourceClass::Heavy,
        }
    }

    /// Derive a neutral [`CapabilityDescriptor`] from a skill's metadata, with no
    /// loss of the fields the Brain (CIL/permission/planner) needs.
    fn descriptor_from(&self, m: &SkillMetadata) -> CapabilityDescriptor {
        let classes = Self::effect_classes(&m.capabilities);
        // Reversibility is inferred from declared effects: write/subprocess are
        // treated as irreversible; a pure read/compute skill as reversible.
        let reversible = if m.capabilities.filesystem_write || m.capabilities.subprocess {
            Reversibility::Irreversible
        } else {
            Reversibility::Reversible
        };

        let effects = Effects {
            classes: classes.clone(),
            reversible,
            idempotent: false,
            resource_class: Self::map_resource_class(m.resource_class),
        };

        // Tags: reuse the skill's open-vocabulary categories + tags. These are
        // strings supplied by the skill, never enumerated here.
        let mut tags: Vec<CapabilityTag> = m
            .categories
            .iter()
            .chain(m.tags.iter())
            .map(|t| CapabilityTag::new(t.clone()))
            .collect();
        tags.dedup_by(|a, b| a.id == b.id);

        CapabilityDescriptor {
            schema_version: crate::capability::descriptor::DescriptorVersion::CURRENT,
            provider_id: self.id.clone(),
            capability_id: m.skill_id.clone(),
            version: m.version.clone(),
            name: m.name.clone(),
            description: m.description.clone(),
            tags,
            input_schema: m
                .input_schema
                .clone()
                .unwrap_or_else(|| serde_json::json!({})),
            output_schema: None,
            io_modality: vec!["text".to_string()],
            inputs: Vec::new(),
            outputs: Vec::new(),
            examples: Vec::new(),
            effects,
            permissions: classes,
            trust: TrustInfo {
                publisher: Some(m.publisher.clone()),
                signed: m.signature.is_some(),
                tier: Some(m.trust_tier.as_str().to_string()),
            },
            quality: Default::default(),
            stats: None,
            guidance: None,
            expectations: None,
            extensions: serde_json::Map::new(),
        }
    }
}

#[async_trait]
impl CapabilityProvider for OpenClawProvider {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }

    async fn negotiate(&self, client: &ClientCapabilities) -> Result<ProtocolSession, CapError> {
        // Mandatory facets always; LIFECYCLE only when acquisition deps are wired
        // (honest: never advertise a facet the adapter cannot fulfil).
        let mut provider_features = FeatureSet::mandatory();
        if self.lifecycle.is_some() {
            provider_features.insert(crate::capability::protocol::Feature::Lifecycle);
        }
        Ok(client.negotiate(
            self.id.clone(),
            ProtocolVersion::CURRENT,
            provider_features,
            serde_json::Map::new(),
        ))
    }

    async fn describe(
        &self,
        _session: &ProtocolSession,
    ) -> Result<Vec<CapabilityDescriptor>, CapError> {
        let skills = self
            .registry
            .get_enabled_skills()
            .map_err(|e| CapError::Discovery(format!("registry read failed: {e}")))?;
        Ok(skills.iter().map(|m| self.descriptor_from(m)).collect())
    }

    async fn catalog(&self) -> Result<Vec<CapabilityDescriptor>, CapError> {
        // Marketplace federation: fetch the ClawHub index and expose each entry
        // as an installable (not-yet-installed) descriptor for recommendations.
        // Requires the LIFECYCLE deps (index url + allowed hosts); otherwise no
        // catalog.
        let Some(lc) = self.lifecycle.as_ref() else {
            return Ok(Vec::new());
        };
        let client =
            crate::openclaw::clawhub::ClawHubClient::new(&lc.index_url, lc.allowed_hosts.clone());
        let index = client
            .fetch_remote_index()
            .await
            .map_err(|e| CapError::Discovery(format!("marketplace index fetch failed: {e}")))?;
        Ok(index
            .iter()
            .map(|e| {
                let mut d = CapabilityDescriptor::minimal(
                    self.id.clone(),
                    e.slug.clone(),
                    e.name.clone(),
                    e.description.clone(),
                    serde_json::json!({}),
                );
                d.version = e.version.clone();
                d.trust.tier = Some("community".to_string());
                // Mark as installable-but-not-installed so the UI/planner can
                // distinguish a recommendation from an installed capability.
                d.extensions
                    .insert("installed".to_string(), serde_json::Value::Bool(false));
                d.extensions.insert(
                    "manifest_url".to_string(),
                    serde_json::Value::String(e.manifest_url.clone()),
                );
                d
            })
            .collect())
    }

    async fn execute(&self, req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
        // Mirror the frozen OpenClawExecutor: assemble a LaunchSpec and run it on
        // the Docker runtime. Grant materialization (from the permission engine)
        // and installed-bundle mounting are layered on in later milestones; for
        // baked/GREEN skills this is the exact proven execution path.
        let spec = LaunchSpec {
            skill_id: req.capability_id.clone(),
            params: req.args.clone(),
            resource_class: OcResourceClass::Light,
            timeout: self.default_timeout,
            correlation_id: req.context.correlation_id.clone(),
            grants: Vec::new(),
            mounted_skill_dir: None,
        };
        let runtime_ctx = RuntimeContext::detached();

        let result = self.runtime.execute(spec, runtime_ctx).await;
        if result.success {
            Ok(CapabilityOutcome::Value(result.data))
        } else {
            // A skill that ran but failed is an execution error (honest), never a
            // fabricated success.
            Err(CapError::Execute(
                result
                    .error
                    .unwrap_or_else(|| "openclaw execution failed".to_string()),
            ))
        }
    }

    async fn acquire(
        &self,
        req: &crate::capability::provider::AcquireRequest,
    ) -> Result<CapabilityDescriptor, CapError> {
        use crate::openclaw::bundle::synth::synth_marketplace_bundle;
        use crate::openclaw::bundle::verify::TrustPolicy;
        use crate::openclaw::bundle::BundleInstaller;
        use crate::openclaw::clawhub::{ClawHubClient, DomainValidator};
        use crate::openclaw::transpiler::transpile_skill;
        use crate::openclaw::types::{SkillSource, TrustTier};

        let lc = self
            .lifecycle
            .as_ref()
            .ok_or_else(|| CapError::Unsupported("lifecycle".into()))?;

        // 1. Fetch the marketplace index and pick the best match for the needed
        //    capability tag / hint (token-overlap over slug/name/description).
        let client = ClawHubClient::new(&lc.index_url, lc.allowed_hosts.clone());
        let index = client
            .fetch_remote_index()
            .await
            .map_err(|e| CapError::Acquire(format!("marketplace index fetch failed: {e}")))?;

        let needle = req
            .hint
            .clone()
            .unwrap_or_else(|| req.capability_tag.clone())
            .to_lowercase();
        let needle_tokens: Vec<&str> = needle
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect();
        let score = |e: &crate::openclaw::clawhub::RemoteSkillEntry| -> usize {
            let hay = format!("{} {} {}", e.slug, e.name, e.description).to_lowercase();
            needle_tokens.iter().filter(|t| hay.contains(**t)).count()
        };
        let entry = index
            .iter()
            .filter(|e| score(e) > 0)
            .max_by_key(|e| score(e))
            .ok_or_else(|| CapError::Acquire(format!("no marketplace skill matches '{needle}'")))?
            .clone();

        // Already installed? Just return its current descriptor (idempotent).
        if let Ok(meta) = self.registry.get_skill(&entry.slug) {
            return Ok(self.descriptor_from(&meta));
        }

        // 2. Validate + download the manifest, transpile, force Community tier.
        let validator = DomainValidator::new(lc.allowed_hosts.clone());
        validator
            .validate(&entry.manifest_url)
            .map_err(|e| CapError::Acquire(format!("manifest URL rejected: {e}")))?;
        let raw = client
            .download_skill_manifest(&entry.manifest_url)
            .await
            .map_err(|e| CapError::Acquire(format!("manifest download failed: {e}")))?;
        let source = SkillSource::ClawHub {
            slug: entry.slug.clone(),
            version: "remote".into(),
        };
        let mut descriptor = transpile_skill(&raw, source, false)
            .map_err(|e| CapError::Acquire(format!("transpile failed: {e}")))?;
        descriptor.trust_tier = TrustTier::Community;

        // 3. Synthesize a bundle + install through the SINGLE frozen BundleInstaller
        //    (same path the desktop marketplace install uses — no second installer).
        let caps: Vec<crate::openclaw::capability::Capability> = descriptor
            .granted
            .iter()
            .map(|g| g.capability.clone())
            .collect();
        let synth_root =
            std::env::temp_dir().join(format!("kria-cpp-acq-{}", uuid::Uuid::new_v4()));
        let bundle_dir = synth_root.join(&descriptor.skill_id);
        synth_marketplace_bundle(&descriptor, &caps, &bundle_dir)
            .map_err(|e| CapError::Acquire(format!("bundle synthesis failed: {e}")))?;

        let installer = BundleInstaller::new(
            self.registry.clone(),
            lc.audit.clone(),
            lc.store_dir.clone(),
        )
        .with_trust_policy(TrustPolicy {
            trusted_keys: Vec::new(),
            require_signature: true,
        });
        let outcome = installer.install(&bundle_dir).map_err(|e| {
            let _ = std::fs::remove_dir_all(&synth_root);
            CapError::Acquire(format!("install failed: {e}"))
        })?;
        let _ = std::fs::remove_dir_all(&synth_root);

        // 4. Return the freshly-installed capability's descriptor (descriptor refresh).
        let meta = self
            .registry
            .get_skill(&outcome.skill_id)
            .map_err(|e| CapError::Acquire(format!("post-install registry read failed: {e}")))?;
        Ok(self.descriptor_from(&meta))
    }

    async fn remove(&self, capability_id: &str) -> Result<(), CapError> {
        if self.lifecycle.is_none() {
            return Err(CapError::Unsupported("lifecycle".into()));
        }
        self.registry
            .uninstall(capability_id)
            .map_err(|e| CapError::Acquire(format!("uninstall failed: {e}")))?;
        Ok(())
    }

    async fn health(&self) -> ProviderHealth {
        match self.runtime.kind() {
            RuntimeKind::Docker | RuntimeKind::Gpu => ProviderHealth::Ready,
            _ => ProviderHealth::Degraded,
        }
    }
}
