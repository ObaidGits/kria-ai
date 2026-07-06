//! Production skill bundle (`.ocskill`) — the single skill artifact (INV-1).
//!
//! A bundle is a directory (canonical) or a `.ocskill` tar archive (distribution). Its
//! `manifest.toml` is the single source of truth; the `SkillDescriptor` is a derived projection.

pub mod deps;
pub mod events;
pub mod installer;
pub mod manifest;
pub mod synth;
pub mod verify;
pub mod version;

use self::manifest::{Manifest, ManifestError};
use self::verify::{TrustPolicy, VerifyError};
use crate::openclaw::capability::{self, Capability};
use crate::openclaw::types::*;
use crate::safety::RiskLevel;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("io error: {0}")]
    Io(String),
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
    #[error("verification error: {0}")]
    Verify(#[from] VerifyError),
    #[error("bundle missing required file: {0}")]
    MissingFile(String),
    #[error("archive error: {0}")]
    Archive(String),
}

/// A loaded (not yet verified) skill bundle.
pub struct Bundle {
    root: PathBuf,
    /// Kept alive so an extracted temp dir is not dropped while the bundle is in use.
    _tmp: Option<TempDir>,
    manifest: Manifest,
    capabilities: Vec<Capability>,
}

impl Bundle {
    /// Open a bundle from a directory or a `.ocskill`/`.tar` archive, parsing + validating the
    /// manifest. Does NOT verify signatures — call [`Bundle::verify`] for that.
    pub fn open(path: &Path) -> Result<Self, BundleError> {
        let (root, tmp) = if path.is_dir() {
            (path.to_path_buf(), None)
        } else {
            let tmp = TempDir::new().map_err(|e| BundleError::Io(e.to_string()))?;
            extract_archive(path, tmp.path())?;
            let root = locate_manifest_root(tmp.path())?;
            (root, Some(tmp))
        };

        let manifest_path = root.join("manifest.toml");
        let toml_str = std::fs::read_to_string(&manifest_path)
            .map_err(|_| BundleError::MissingFile("manifest.toml".into()))?;
        let manifest = Manifest::parse(&toml_str)?;
        let capabilities = manifest.validate()?;

        // Integrity: required files present.
        if !root.join("schema.json").exists() {
            return Err(BundleError::MissingFile("schema.json".into()));
        }
        let entry = root.join(&manifest.runtime.entry);
        if !entry.exists() {
            return Err(BundleError::MissingFile(manifest.runtime.entry.clone()));
        }

        Ok(Self {
            root,
            _tmp: tmp,
            manifest,
            capabilities,
        })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Verify hashes + signature under `policy`. Returns the content hash.
    pub fn verify(&self, policy: &TrustPolicy) -> Result<String, BundleError> {
        Ok(verify::verify_signature(
            &self.root,
            &self.manifest,
            policy,
        )?)
    }

    /// Read the parameter schema (`schema.json`) as JSON.
    fn schema(&self) -> serde_json::Value {
        std::fs::read_to_string(self.root.join("schema.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}))
    }

    /// Project the bundle into a `SkillDescriptor` (the LLM/router-facing derived view).
    pub fn to_descriptor(&self) -> SkillDescriptor {
        let m = &self.manifest;
        let legacy_caps = capability::to_legacy(&self.capabilities);
        let risk = capability::classify_risk(&self.capabilities);
        let network_policy = legacy_caps.to_network_policy();
        let resource_class = m
            .resource
            .class
            .parse::<ResourceClass>()
            .unwrap_or(ResourceClass::Light);
        let resource_profile = ResourceProfile {
            memory_limit: format!("{}m", m.resource.memory_mb.max(1)),
            cpu_limit: format!("{:.1}", (m.resource.cpu_millis as f64 / 1000.0).max(0.1)),
            timeout_secs: m.resource.timeout_secs,
            max_output_bytes: m.resource.max_output_bytes,
            requires_approval: !matches!(risk, RiskLevel::Green),
            resource_class,
        };
        let trust_tier = m
            .trust
            .declared_tier
            .parse::<TrustTier>()
            .unwrap_or(TrustTier::Community);

        SkillDescriptor {
            skill_id: m.skill.slug.clone(),
            name: m.skill.name.clone(),
            description: m.skill.description.clone(),
            category: m.skill.category.clone(),
            parameters: self.schema(),
            risk_level: risk,
            network_policy,
            resource_profile,
            capabilities: legacy_caps,
            granted: capability::grant_all(
                &self.capabilities,
                capability::GrantSource::Manifest,
                true,
            ),
            trust_tier,
            source: SkillSource::ClawHub {
                slug: m.skill.slug.clone(),
                version: m.skill.version.clone(),
            },
            installed_at: chrono::Utc::now(),
            last_used_at: None,
            use_count: 0,
            status: SkillStatus::Active,
        }
    }
}

/// Extract a tar (optionally the `.ocskill`) archive into `dest`.
fn extract_archive(path: &Path, dest: &Path) -> Result<(), BundleError> {
    let file = std::fs::File::open(path).map_err(|e| BundleError::Io(e.to_string()))?;
    let mut archive = tar::Archive::new(file);
    archive
        .unpack(dest)
        .map_err(|e| BundleError::Archive(e.to_string()))?;
    Ok(())
}

/// Find the directory containing `manifest.toml` (archive root or a single top-level subdir).
fn locate_manifest_root(extracted: &Path) -> Result<PathBuf, BundleError> {
    if extracted.join("manifest.toml").exists() {
        return Ok(extracted.to_path_buf());
    }
    // Single top-level directory case.
    let mut entries = std::fs::read_dir(extracted)
        .map_err(|e| BundleError::Io(e.to_string()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect::<Vec<_>>();
    entries.retain(|p| p.is_dir());
    for dir in &entries {
        if dir.join("manifest.toml").exists() {
            return Ok(dir.clone());
        }
    }
    Err(BundleError::MissingFile("manifest.toml".into()))
}

pub use installer::{BundleInstaller, InstallError, InstallOutcome, SkillActivation};
