//! The single bundle installer (A2.4/A2.5/A2.8). One installer, one registry, one source of
//! truth. Installs/updates/uninstalls `.ocskill` bundles atomically with rollback, drives hot
//! reload through a `SkillActivation` sink, and emits lifecycle events + audit entries.

use super::events::{self, BundleLifecycleEvent};
use super::manifest::Manifest;
use super::version::{self, VersionRelation};
use super::{deps, Bundle, BundleError};
use crate::openclaw::audit::AuditLedger;
use crate::openclaw::registry::{BundleProvenance, RegistryError, SkillRegistry};
use crate::openclaw::types::SkillDescriptor;
use semver::Version;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Hot-reload sink (router-contract §5). Implemented by the host (desktop/server) to update the
/// `ToolRegistry`, runtime registry, and semantic tool index without a restart.
pub trait SkillActivation: Send + Sync {
    /// Register/replace a skill so it is immediately callable.
    fn activate(&self, skill: &SkillDescriptor) -> Result<(), String>;
    /// Remove a skill so it is immediately gone.
    fn deactivate(&self, skill_id: &str) -> Result<(), String>;
    /// Rebuild the semantic tool index (idempotent; called after activate/deactivate).
    fn reindex(&self);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    pub skill_id: String,
    pub version: String,
    pub relation: VersionRelation,
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error(transparent)]
    Bundle(#[from] BundleError),
    #[error("dependency error: {0}")]
    Deps(#[from] deps::DepError),
    #[error("downgrade blocked: installed {installed}, bundle {candidate} (uninstall first)")]
    DowngradeBlocked {
        installed: String,
        candidate: String,
    },
    #[error("registry error: {0}")]
    Registry(#[from] RegistryError),
    #[error("io error: {0}")]
    Io(String),
    #[error("activation failed: {0}")]
    Activation(String),
    #[error("rolled back after failure: {0}")]
    RolledBack(String),
    #[error("skill not found: {0}")]
    NotFound(String),
}

pub struct BundleInstaller {
    registry: Arc<SkillRegistry>,
    audit: Arc<AuditLedger>,
    store_dir: PathBuf,
    kria_version: Version,
    policy: super::verify::TrustPolicy,
    activation: Option<Arc<dyn SkillActivation>>,
}

impl BundleInstaller {
    pub fn new(registry: Arc<SkillRegistry>, audit: Arc<AuditLedger>, store_dir: PathBuf) -> Self {
        let kria_version =
            Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 0, 0));
        Self {
            registry,
            audit,
            store_dir,
            kria_version,
            policy: super::verify::TrustPolicy::strict(),
            activation: None,
        }
    }

    pub fn with_activation(mut self, activation: Arc<dyn SkillActivation>) -> Self {
        self.activation = Some(activation);
        self
    }

    pub fn with_trust_policy(mut self, policy: super::verify::TrustPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_kria_version(mut self, v: Version) -> Self {
        self.kria_version = v;
        self
    }

    /// Install or update a bundle. Atomic: on any failure after mutation begins, the registry,
    /// filesystem, and activation state are restored.
    pub fn install(&self, bundle_path: &Path) -> Result<InstallOutcome, InstallError> {
        // ── Phase 1: open + validate (no side effects) ─────────────────────────
        let bundle = Bundle::open(bundle_path)?;
        let manifest = bundle.manifest().clone();
        let slug = manifest.skill.slug.clone();
        let version_str = manifest.skill.version.clone();

        events::emit(BundleLifecycleEvent::Installing {
            slug: slug.clone(),
            version: version_str.clone(),
        });

        let result = self.install_inner(&bundle, &manifest);
        if let Err(ref e) = result {
            events::emit(BundleLifecycleEvent::Failed {
                slug: slug.clone(),
                reason: e.to_string(),
            });
        }
        result
    }

    fn install_inner(
        &self,
        bundle: &Bundle,
        manifest: &Manifest,
    ) -> Result<InstallOutcome, InstallError> {
        let slug = manifest.skill.slug.clone();
        let candidate_ver = manifest.semver();

        // Verify signature + hashes.
        let content_hash = bundle.verify(&self.policy)?;

        // Publisher revocation enforcement fix (product gap 7/8): a
        // publisher known to the global registry (by manifest-declared
        // signing key) that has been revoked must not be able to install a
        // NEW skill, even though its signature is cryptographically valid.
        // An UNKNOWN publisher (not yet registered) is allowed through here
        // unchanged — matches `TrustFramework::evaluate`'s existing
        // "unknown publisher, no verification required" default (this fix
        // enforces revocation, not first-time verification policy, which is
        // a separate, larger enterprise-policy decision out of this fix's
        // scope).
        {
            use crate::openclaw::platform::publisher::VerificationStatus;
            let signing_key = manifest.trust.publisher.trim();
            if let Some(publisher) =
                crate::openclaw::platform::publisher::global().find_by_key(signing_key)
            {
                if publisher.verification == VerificationStatus::Revoked {
                    return Err(InstallError::Bundle(BundleError::Verify(
                        crate::openclaw::bundle::verify::VerifyError::UntrustedPublisher(format!(
                            "publisher '{}' has been revoked",
                            publisher.publisher_id
                        )),
                    )));
                }
            }
        }

        // Dependency + conflict resolution.
        let refs = self.installed_refs()?;
        deps::resolve(
            manifest,
            &refs,
            &self.kria_version,
            &deps::substrate_provides(),
        )?;

        // Version relation vs currently installed.
        let previous_descriptor = self.registry.get(&slug).ok();
        let previous_prov = self.registry.get_provenance(&slug).ok().flatten();
        let installed_ver = previous_prov
            .as_ref()
            .and_then(|p| Version::parse(&p.version).ok());
        let relation = version::relation(&candidate_ver, installed_ver.as_ref());

        // Idempotent no-op: identical content already installed.
        if let Some(prev) = &previous_prov {
            if prev.content_hash == content_hash && relation == VersionRelation::Same {
                events::emit(BundleLifecycleEvent::Installed {
                    slug: slug.clone(),
                    version: manifest.skill.version.clone(),
                });
                return Ok(InstallOutcome {
                    skill_id: slug,
                    version: manifest.skill.version.clone(),
                    relation,
                });
            }
        }

        // Block downgrades (explicit uninstall required — package-contract).
        if relation == VersionRelation::Downgrade {
            return Err(InstallError::DowngradeBlocked {
                installed: installed_ver.map(|v| v.to_string()).unwrap_or_default(),
                candidate: candidate_ver.to_string(),
            });
        }

        // ── Phase 2: mutation with rollback ────────────────────────────────────
        // 2a. Copy bundle into the versioned store dir (non-destructive).
        let dest = self
            .store_dir
            .join(&slug)
            .join(manifest.skill.version.clone());
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(|e| InstallError::Io(e.to_string()))?;
        }
        copy_dir_all(bundle.root(), &dest).map_err(|e| InstallError::Io(e.to_string()))?;

        // 2a-bis. Prepare a bridge-format runtime dir (`<dest>/.bridge/`) so
        // the substrate's MCP bridge can load + execute this skill's handler
        // when the container bind-mounts it (bundle-execution fix). The
        // bridge expects `<slug>.json` (name/description/inputSchema/handler)
        // + the handler JS; the `.ocskill` bundle stores `manifest.toml` +
        // `schema.json` + the entry handler, so we project into the bridge
        // format here, once, at install time. Non-fatal on failure (the skill
        // still installs + routes; only runtime execution needs this).
        if let Err(e) = prepare_bridge_dir(&dest, manifest) {
            tracing::warn!(slug = %slug, error = %e, "[installer] failed to prepare bridge runtime dir (skill will route but not execute until fixed)");
        }

        // 2b. Build descriptor + provenance.
        let mut descriptor = bundle.to_descriptor();
        descriptor.skill_id = slug.clone();
        let signature = std::fs::read_to_string(dest.join(super::verify::SIGNATURE_FILE))
            .unwrap_or_default()
            .trim()
            .to_string();
        let prov = BundleProvenance {
            publisher: manifest.trust.publisher.clone(),
            version: manifest.skill.version.clone(),
            content_hash: content_hash.clone(),
            signature,
            manifest_toml: std::fs::read_to_string(dest.join("manifest.toml")).unwrap_or_default(),
            bundle_path: dest.to_string_lossy().to_string(),
        };

        // 2c. Registry write.
        if let Err(e) = self.registry.install_bundle(&descriptor, &prov) {
            self.rollback(
                &slug,
                &dest,
                &previous_descriptor,
                &previous_prov,
                "registry write failed",
            );
            return Err(InstallError::RolledBack(format!("registry: {e}")));
        }

        // 2c-bis. Auto-enable fix (R6.1/R3.4, product gap 4/8): `install_bundle`
        // always lands a skill in `Installed` state, never `Enabled` — a fresh
        // install used to be silently unroutable until a SEPARATE `enable()`
        // call. Fresh installs now transition straight to `Enabled` (no second
        // step, matching the task requirement). Upgrades PRESERVE whatever the
        // skill's enabled/disabled state was immediately before this install —
        // never silently re-enabling a skill the user explicitly disabled.
        use crate::openclaw::registry::SkillState;
        use crate::openclaw::types::SkillStatus;
        let target_state = match relation {
            VersionRelation::Fresh => Some(SkillState::Enabled),
            _ => match previous_descriptor.as_ref().map(|d| d.status) {
                Some(SkillStatus::Active) => Some(SkillState::Enabled),
                Some(_) => None, // was disabled/quarantined/etc — preserve, don't force-enable
                None => Some(SkillState::Enabled), // no prior descriptor, treat like fresh
            },
        };
        if let Some(state) = target_state {
            if let Err(e) = self.registry.set_skill_state(&slug, state) {
                self.rollback(
                    &slug,
                    &dest,
                    &previous_descriptor,
                    &previous_prov,
                    "auto-enable state transition failed",
                );
                return Err(InstallError::RolledBack(format!("auto-enable: {e}")));
            }
        }

        // 2d. Hot activation + reindex.
        if let Some(act) = &self.activation {
            if let Err(e) = act.activate(&descriptor) {
                self.rollback(
                    &slug,
                    &dest,
                    &previous_descriptor,
                    &previous_prov,
                    "activation failed",
                );
                return Err(InstallError::RolledBack(format!("activation: {e}")));
            }
            act.reindex();
        }

        // 2e. Audit.
        let mut entry = AuditLedger::create_skill_install_entry(
            &descriptor.skill_id,
            &descriptor.name,
            descriptor.trust_tier.as_str(),
            &prov.bundle_path,
        );
        entry.signature = self.audit.sign_entry(&entry);
        let _ = self.audit.append(&entry);

        // Success events.
        match relation {
            VersionRelation::Upgrade => events::emit(BundleLifecycleEvent::Updated {
                slug: slug.clone(),
                from: installed_ver.map(|v| v.to_string()).unwrap_or_default(),
                to: manifest.skill.version.clone(),
            }),
            _ => events::emit(BundleLifecycleEvent::Installed {
                slug: slug.clone(),
                version: manifest.skill.version.clone(),
            }),
        }

        Ok(InstallOutcome {
            skill_id: slug,
            version: manifest.skill.version.clone(),
            relation,
        })
    }

    /// Uninstall a skill: deactivate, remove from registry, delete stored files.
    pub fn uninstall(&self, slug: &str) -> Result<(), InstallError> {
        if self.registry.get(slug).is_err() {
            return Err(InstallError::NotFound(slug.to_string()));
        }
        if let Some(act) = &self.activation {
            let _ = act.deactivate(slug);
            act.reindex();
        }
        self.registry.uninstall(slug)?;
        let slug_dir = self.store_dir.join(slug);
        if slug_dir.exists() {
            let _ = std::fs::remove_dir_all(&slug_dir);
        }
        events::emit(BundleLifecycleEvent::Removed {
            slug: slug.to_string(),
        });
        Ok(())
    }

    /// Enable a previously-disabled skill (re-activate immediately).
    pub fn enable(&self, slug: &str) -> Result<(), InstallError> {
        self.registry.toggle(slug, true)?;
        if let Some(act) = &self.activation {
            let descriptor = self.registry.get(slug)?;
            act.activate(&descriptor)
                .map_err(InstallError::Activation)?;
            act.reindex();
        }
        events::emit(BundleLifecycleEvent::Enabled {
            slug: slug.to_string(),
        });
        Ok(())
    }

    /// Disable a skill (deactivate immediately).
    pub fn disable(&self, slug: &str) -> Result<(), InstallError> {
        self.registry.toggle(slug, false)?;
        if let Some(act) = &self.activation {
            let _ = act.deactivate(slug);
            act.reindex();
        }
        events::emit(BundleLifecycleEvent::Disabled {
            slug: slug.to_string(),
        });
        Ok(())
    }

    fn installed_refs(&self) -> Result<Vec<deps::InstalledRef>, InstallError> {
        let refs = self
            .registry
            .installed_refs()?
            .into_iter()
            .filter_map(|(slug, ver, publisher)| {
                let version = Version::parse(&ver).ok()?;
                Some(deps::InstalledRef {
                    slug,
                    version,
                    publisher,
                    provides_runtime: Vec::new(),
                })
            })
            .collect();
        Ok(refs)
    }

    /// Restore registry + filesystem + activation to the pre-install state.
    fn rollback(
        &self,
        slug: &str,
        new_dir: &Path,
        previous_descriptor: &Option<SkillDescriptor>,
        previous_prov: &Option<BundleProvenance>,
        reason: &str,
    ) {
        // Remove the freshly-copied bundle dir.
        if new_dir.exists() {
            let _ = std::fs::remove_dir_all(new_dir);
        }
        // Restore registry.
        match (previous_descriptor, previous_prov) {
            (Some(desc), Some(prov)) => {
                let _ = self.registry.install_bundle(desc, prov);
                if let Some(act) = &self.activation {
                    let _ = act.activate(desc);
                    act.reindex();
                }
            }
            _ => {
                // Fresh install → remove any partial row + deactivate.
                let _ = self.registry.uninstall(slug);
                if let Some(act) = &self.activation {
                    let _ = act.deactivate(slug);
                    act.reindex();
                }
            }
        }
        events::emit(BundleLifecycleEvent::RolledBack {
            slug: slug.to_string(),
            reason: reason.to_string(),
        });
    }
}

/// Project an installed `.ocskill` bundle into the MCP bridge's runtime
/// skill format at `<dest>/.bridge/` (bundle-execution fix). Writes
/// `.bridge/<slug>.json` (name/description/inputSchema/handler=handler.js)
/// and copies the manifest's entry handler to `.bridge/handler.js`. The
/// `DockerRuntime` bind-mounts this dir into a bespoke execution container.
fn prepare_bridge_dir(dest: &Path, manifest: &Manifest) -> std::io::Result<()> {
    let bridge_dir = dest.join(".bridge");
    std::fs::create_dir_all(&bridge_dir)?;

    // inputSchema from the bundle's schema.json (best-effort; default to a
    // permissive object schema if unreadable).
    let input_schema: serde_json::Value = std::fs::read_to_string(dest.join("schema.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({ "type": "object", "properties": {} }));

    let descriptor = serde_json::json!({
        "name": manifest.skill.slug,
        "description": manifest.skill.description,
        "inputSchema": input_schema,
        "handler": "handler.js",
    });
    std::fs::write(
        bridge_dir.join(format!("{}.json", manifest.skill.slug)),
        serde_json::to_string_pretty(&descriptor).unwrap_or_else(|_| "{}".to_string()),
    )?;

    // Copy the entry handler to a flat `.bridge/handler.js`. The entry may be
    // nested (e.g. `handler/main.js`); we flatten it so the bridge's
    // handler-path resolution (relative to the mounted dir) is trivial.
    let entry_src = dest.join(&manifest.runtime.entry);
    if entry_src.exists() {
        std::fs::copy(&entry_src, bridge_dir.join("handler.js"))?;
    } else {
        // No entry file present (should not happen for a validated bundle) —
        // write an honest stub that reports the missing handler at runtime.
        std::fs::write(
            bridge_dir.join("handler.js"),
            "module.exports = () => ({ error: 'handler_missing', reason: 'bundle entry file was not found at install time' });\n",
        )?;
    }
    Ok(())
}

/// Recursively copy a directory tree.
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
