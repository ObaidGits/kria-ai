//! A8.3 Skill Publishing — the publishing pipeline.
//!
//! Skill → validation → signing → package → metadata → repository → published.
//! Reuses the frozen A2 bundle layer for manifest validation, hash-tree + ed25519
//! signing and verification. Nothing published bypasses validation.

use super::publisher::{normalize_key, Publisher, PublisherRegistry};
use super::repository::{LocalRepository, RepositoryEntry};
use crate::openclaw::bundle::verify::{self, TrustPolicy};
use crate::openclaw::bundle::Bundle;
use ed25519_dalek::SigningKey;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("unknown publisher: {0}")]
    UnknownPublisher(String),
    #[error("signing key does not match publisher identity")]
    KeyMismatch,
    #[error("signing failed: {0}")]
    Signing(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("bundle error: {0}")]
    Bundle(String),
}

/// A request to publish a skill bundle (A8.3).
pub struct PublishRequest<'a> {
    /// Path to the bundle directory to publish.
    pub bundle_dir: &'a Path,
    /// Publisher id performing the publish.
    pub publisher_id: &'a str,
    /// Signing key for the publisher.
    pub signing_key: &'a SigningKey,
}

/// The single publishing pipeline (A8.3).
pub struct PublishingPipeline {
    publishers: PublisherRegistry,
}

impl PublishingPipeline {
    pub fn new(publishers: PublisherRegistry) -> Self {
        Self { publishers }
    }

    /// Run the full pipeline: validate → sign → package → metadata → publish into
    /// the target local repository. Returns the produced `RepositoryEntry`.
    pub fn publish(
        &self,
        req: &PublishRequest,
        target_repo: &LocalRepository,
        repo_dir: &Path,
    ) -> Result<RepositoryEntry, PublishError> {
        // 1. Publisher must be known.
        let publisher = self
            .publishers
            .get(req.publisher_id)
            .ok_or_else(|| PublishError::UnknownPublisher(req.publisher_id.to_string()))?;

        // 2. The signing key must match the publisher's declared identity.
        let signer_pub = hex::encode(req.signing_key.verifying_key().to_bytes());
        if !publisher.all_keys().contains(&normalize_key(&signer_pub)) {
            return Err(PublishError::KeyMismatch);
        }

        // 3. Validation: open + validate manifest, capabilities, schema, runtime, deps.
        let bundle =
            Bundle::open(req.bundle_dir).map_err(|e| PublishError::Bundle(e.to_string()))?;
        let manifest = bundle.manifest();

        // Manifest publisher must equal the signer identity.
        if normalize_key(&manifest.trust.publisher) != normalize_key(&signer_pub) {
            return Err(PublishError::Validation(
                "manifest publisher key does not match signing key".into(),
            ));
        }

        let slug = manifest.skill.slug.clone();
        let name = manifest.skill.name.clone();
        let description = manifest.skill.description.clone();
        let category = manifest.skill.category.clone();
        let version = manifest.skill.version.clone();

        // 4. Signing: write hash-tree + detached signature into the bundle dir.
        let content_hash = verify::write_hash_tree(req.bundle_dir)
            .map_err(|e| PublishError::Signing(e.to_string()))?;
        verify::sign_bundle(req.bundle_dir, req.signing_key)
            .map_err(|e| PublishError::Signing(e.to_string()))?;

        // 5. Verify the freshly-signed bundle (defense in depth: nothing unverified ships).
        verify::verify_signature(req.bundle_dir, manifest, &TrustPolicy::strict())
            .map_err(|e| PublishError::Validation(format!("post-sign verification failed: {e}")))?;

        // 6. Package: copy the bundle dir into the repository as `<slug>-<version>/`.
        let pkg_rel = format!("{}-{}", slug, version);
        let pkg_dir = repo_dir.join(&pkg_rel);
        copy_dir_recursive(req.bundle_dir, &pkg_dir).map_err(|e| PublishError::Io(e))?;

        // 7. Metadata: build the repository entry.
        let entry = RepositoryEntry {
            slug,
            name,
            description,
            category,
            version,
            publisher_id: req.publisher_id.to_string(),
            content_hash,
            location: pkg_rel,
            tags: manifest.skill.tags.clone(),
            signed: true,
        };

        // 8. Repository: append/replace entry in the target repo index.
        let mut index = target_repo
            .fetch_index_blocking()
            .map_err(|e| PublishError::Io(e))?;
        index.retain(|e| !(e.slug == entry.slug && e.version == entry.version));
        index.push(entry.clone());
        target_repo
            .write_index(&index)
            .map_err(|e| PublishError::Io(e.to_string()))?;

        // 9. Update publisher history.
        self.publishers.adjust_reputation(req.publisher_id, 0.01);
        self.bump_published(&publisher);

        Ok(entry)
    }

    fn bump_published(&self, publisher: &Publisher) {
        let mut p = publisher.clone();
        p.published_count += 1;
        self.publishers.register(p);
    }
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let dest = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest)?;
        } else {
            std::fs::copy(&path, &dest).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
