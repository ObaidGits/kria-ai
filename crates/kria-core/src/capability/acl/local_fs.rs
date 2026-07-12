//! `LocalFsProvider` — a REAL, second lifecycle-capable [`CapabilityProvider`]
//! (Wave 7.2 / spec R9.3, E2). It is deliberately NOT OpenClaw and NOT Docker:
//! it proves the Brain's acquisition + execution + upgrade + removal path is
//! genuinely provider-neutral by exercising the identical neutral lifecycle
//! through a completely different backend — the local filesystem.
//!
//! # What it does (real, not a stub)
//! - **catalog()**: reads capability manifests (`*.json`) from a *source*
//!   directory → neutral descriptors (marked installed per the *store*).
//! - **acquire()**: copies the Brain-selected manifest from source → store
//!   (real install) and returns its descriptor.
//! - **describe()**: lists installed manifests from the store.
//! - **execute()**: runs the manifest's declared, sandboxed **pure text
//!   transform** (built-in, whitelisted ops — never arbitrary code), so
//!   execution is real, deterministic, and safe on the host.
//! - **upgrade()**: re-installs when the source version is newer (semver).
//! - **remove()**: deletes the installed manifest (idempotent).
//! - **health()**: Ready when the store directory is accessible.
//!
//! # Neutrality
//! This adapter lives under `capability::acl::*` (the only place provider-native
//! detail may live). It emits ONLY neutral [`CapabilityDescriptor`] /
//! [`CapabilityOutcome`] types and holds no cognition (no ranking/selection/
//! arg-gen) — the Brain decides; this is pure Hands.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::capability::descriptor::{CapabilityDescriptor, Effects, Reversibility, TrustInfo};
use crate::capability::error::CapError;
use crate::capability::protocol::{
    ClientCapabilities, Feature, FeatureSet, ProtocolSession, ProtocolVersion, ProviderHealth,
};
use crate::capability::provider::{
    AcquireRequest, CapabilityOutcome, CapabilityProvider, CapabilityRequest,
};
use crate::capability::ProviderId;

/// A capability manifest stored on disk (source + installed store use the same
/// shape). Purely declarative — the `operation` names a built-in safe transform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalManifest {
    pub capability_id: String,
    pub name: String,
    pub description: String,
    #[serde(default = "default_version")]
    pub version: String,
    /// Built-in transform: one of `reverse`/`upper`/`lower`/`base64_encode`/
    /// `base64_decode`/`length`. Unknown ⇒ honest execution error.
    pub operation: String,
    #[serde(default)]
    pub trust_tier: Option<String>,
    /// Optional declared dependencies (neutral shape the Brain understands).
    #[serde(default)]
    pub dependencies: Vec<serde_json::Value>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

/// The local filesystem provider.
pub struct LocalFsProvider {
    id: ProviderId,
    source_dir: PathBuf,
    store_dir: PathBuf,
}

impl LocalFsProvider {
    /// Create over a `source_dir` (available catalog) and `store_dir` (installed).
    /// The store is created if missing.
    pub fn new(
        id: impl Into<String>,
        source_dir: impl AsRef<Path>,
        store_dir: impl AsRef<Path>,
    ) -> Result<Self, CapError> {
        let store_dir = store_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&store_dir)
            .map_err(|e| CapError::Io(format!("local_fs store dir: {e}")))?;
        Ok(Self {
            id: id.into(),
            source_dir: source_dir.as_ref().to_path_buf(),
            store_dir,
        })
    }

    fn read_manifests(dir: &Path) -> Vec<LocalManifest> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&p) {
                if let Ok(m) = serde_json::from_str::<LocalManifest>(&text) {
                    out.push(m);
                }
            }
        }
        out
    }

    fn installed_path(&self, capability_id: &str) -> PathBuf {
        self.store_dir.join(format!("{capability_id}.json"))
    }

    fn descriptor_from(&self, m: &LocalManifest, installed: bool) -> CapabilityDescriptor {
        let mut d = CapabilityDescriptor::minimal(
            self.id.clone(),
            &m.capability_id,
            &m.name,
            &m.description,
            serde_json::json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }),
        );
        d.version = m.version.clone();
        d.io_modality = vec!["text".into()];
        d.inputs = vec!["text".into()];
        d.outputs = vec!["text".into()];
        // A pure text transform is read-only + reversible + idempotent.
        d.effects = Effects {
            classes: vec![],
            reversible: Reversibility::Reversible,
            idempotent: true,
            resource_class: Default::default(),
        };
        d.trust = TrustInfo {
            publisher: Some(self.id.clone()),
            signed: false,
            tier: m.trust_tier.clone().or_else(|| Some("local".into())),
        };
        d.extensions
            .insert("installed".into(), serde_json::Value::Bool(installed));
        // Declare substrate: an in-process native transform (Brain reads this).
        d.extensions
            .insert("kind".into(), serde_json::Value::String("native".into()));
        if !m.dependencies.is_empty() {
            d.extensions.insert(
                "dependencies".into(),
                serde_json::Value::Array(m.dependencies.clone()),
            );
        }
        d
    }

    fn load_installed(&self, capability_id: &str) -> Option<LocalManifest> {
        let text = std::fs::read_to_string(self.installed_path(capability_id)).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn apply_operation(op: &str, text: &str) -> Result<serde_json::Value, CapError> {
        // Single source of truth: the neutral audited primitive vocabulary
        // (shared with the synthesis provider — no duplicated transform logic).
        match crate::capability::intelligence::primitives::apply_primitive(op, text) {
            Ok(Some(result)) => Ok(serde_json::json!({ "result": result })),
            Ok(None) => Err(CapError::Execute(format!(
                "local_fs: unknown operation '{op}'"
            ))),
            Err(e) => Err(CapError::Execute(e)),
        }
    }
}

#[async_trait]
impl CapabilityProvider for LocalFsProvider {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }

    async fn negotiate(&self, client: &ClientCapabilities) -> Result<ProtocolSession, CapError> {
        // Advertises the LIFECYCLE facet — this is a lifecycle-capable provider.
        Ok(client.negotiate(
            self.id.clone(),
            ProtocolVersion::CURRENT,
            FeatureSet::mandatory().with(Feature::Lifecycle),
            serde_json::Map::new(),
        ))
    }

    async fn describe(
        &self,
        _session: &ProtocolSession,
    ) -> Result<Vec<CapabilityDescriptor>, CapError> {
        Ok(Self::read_manifests(&self.store_dir)
            .iter()
            .map(|m| self.descriptor_from(m, true))
            .collect())
    }

    async fn catalog(&self) -> Result<Vec<CapabilityDescriptor>, CapError> {
        Ok(Self::read_manifests(&self.source_dir)
            .iter()
            .map(|m| {
                let installed = self.installed_path(&m.capability_id).exists();
                self.descriptor_from(m, installed)
            })
            .collect())
    }

    async fn acquire(&self, req: &AcquireRequest) -> Result<CapabilityDescriptor, CapError> {
        let source = Self::read_manifests(&self.source_dir);
        // Honor the Brain-selected capability id; fall back to a tag/hint match
        // ONLY if the Brain did not choose (thin-caller path). No cognition here.
        let manifest = if let Some(chosen) = req.capability_id.as_deref() {
            source
                .into_iter()
                .find(|m| m.capability_id == chosen)
                .ok_or_else(|| {
                    CapError::Acquire(format!("local_fs: '{chosen}' not in source catalog"))
                })?
        } else {
            let needle = req
                .hint
                .clone()
                .unwrap_or_else(|| req.capability_tag.clone())
                .to_lowercase();
            source
                .into_iter()
                .find(|m| {
                    let hay =
                        format!("{} {} {}", m.capability_id, m.name, m.description).to_lowercase();
                    needle.split_whitespace().any(|t| hay.contains(t))
                })
                .ok_or_else(|| {
                    CapError::Acquire(format!("local_fs: no catalog match for '{needle}'"))
                })?
        };
        let json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| CapError::Acquire(format!("serialize manifest: {e}")))?;
        std::fs::write(self.installed_path(&manifest.capability_id), json)
            .map_err(|e| CapError::Acquire(format!("local_fs install write: {e}")))?;
        Ok(self.descriptor_from(&manifest, true))
    }

    async fn execute(&self, req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
        let manifest = self.load_installed(&req.capability_id).ok_or_else(|| {
            CapError::Execute(format!(
                "local_fs: capability '{}' is not installed",
                req.capability_id
            ))
        })?;
        let text = req
            .args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                CapError::Execute("local_fs: missing required 'text' argument".into())
            })?;
        let value = Self::apply_operation(&manifest.operation, text)?;
        Ok(CapabilityOutcome::Value(value))
    }

    async fn remove(&self, capability_id: &str) -> Result<(), CapError> {
        let path = self.installed_path(capability_id);
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| CapError::Acquire(format!("local_fs remove: {e}")))?;
        }
        Ok(()) // idempotent
    }

    async fn health(&self) -> ProviderHealth {
        if self.store_dir.is_dir() {
            ProviderHealth::Ready
        } else {
            ProviderHealth::Offline
        }
    }
}

/// Convenience for tests/tools: build a source catalog on disk from manifests.
pub fn write_source_catalog(dir: &Path, manifests: &[LocalManifest]) -> Result<(), CapError> {
    std::fs::create_dir_all(dir).map_err(|e| CapError::Io(e.to_string()))?;
    for m in manifests {
        let json = serde_json::to_string_pretty(m).map_err(|e| CapError::Io(e.to_string()))?;
        std::fs::write(dir.join(format!("{}.json", m.capability_id)), json)
            .map_err(|e| CapError::Io(e.to_string()))?;
    }
    Ok(())
}
