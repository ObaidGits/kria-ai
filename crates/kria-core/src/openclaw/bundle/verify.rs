//! Bundle verification (A2.3): content-hash tree, ed25519 signature, integrity.
//!
//! Frozen (security-contract): signature = ed25519 over the content-hash tree; identity =
//! (slug, publisher); publisher is a stable ed25519 public key. Algorithm *parameters* (hex
//! encoding here) are ⚠ evolvable; the *presence* of signing is frozen.
//!
//! Bundle layout hashed (skill-package-contract §1): every file except `MANIFEST.sha256`
//! (the tree itself) and `bundle.sig` (the detached signature).

use super::manifest::Manifest;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

pub const MANIFEST_HASH_FILE: &str = "MANIFEST.sha256";
pub const SIGNATURE_FILE: &str = "bundle.sig";

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("io error: {0}")]
    Io(String),
    #[error("missing required file: {0}")]
    MissingFile(String),
    #[error("hash mismatch for '{path}' (bundle tampered or repackaged)")]
    HashMismatch { path: String },
    #[error("MANIFEST.sha256 does not cover file: {0}")]
    UnlistedFile(String),
    #[error("invalid publisher key: {0}")]
    InvalidPublisherKey(String),
    #[error("invalid signature encoding: {0}")]
    InvalidSignature(String),
    #[error("signature verification failed (not signed by the declared publisher)")]
    SignatureRejected,
    #[error("publisher is not in the trusted key set (required for tier '{0}')")]
    UntrustedPublisher(String),
    #[error("bundle is unsigned but signing is required")]
    UnsignedNotAllowed,
}

/// Trust policy for verification.
#[derive(Clone, Default)]
pub struct TrustPolicy {
    /// KRIA-trusted publisher keys (hex). A `verified`-tier bundle must be signed by one of these.
    pub trusted_keys: Vec<String>,
    /// If true, an unsigned bundle is rejected outright (default production posture).
    pub require_signature: bool,
}

impl TrustPolicy {
    pub fn strict() -> Self {
        Self {
            trusted_keys: Vec::new(),
            require_signature: true,
        }
    }
}

/// Compute the canonical content-hash tree for a bundle directory: sorted `sha256hex  relpath`.
pub fn compute_hash_tree(root: &Path) -> Result<String, VerifyError> {
    let mut tree: BTreeMap<String, String> = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).sort_by_file_name() {
        let entry = entry.map_err(|e| VerifyError::Io(e.to_string()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .map_err(|e| VerifyError::Io(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        if rel == MANIFEST_HASH_FILE || rel == SIGNATURE_FILE {
            continue;
        }
        let bytes = std::fs::read(entry.path()).map_err(|e| VerifyError::Io(e.to_string()))?;
        let mut h = Sha256::new();
        h.update(&bytes);
        tree.insert(rel, hex::encode(h.finalize()));
    }
    let mut out = String::new();
    for (rel, hash) in &tree {
        out.push_str(hash);
        out.push_str("  ");
        out.push_str(rel);
        out.push('\n');
    }
    Ok(out)
}

/// The single content hash of the whole bundle = sha256 of the canonical tree.
pub fn content_hash(tree: &str) -> String {
    let mut h = Sha256::new();
    h.update(tree.as_bytes());
    hex::encode(h.finalize())
}

/// Verify that on-disk files match `MANIFEST.sha256` (integrity + tamper detection).
pub fn verify_hashes(root: &Path) -> Result<String, VerifyError> {
    let manifest_hash_path = root.join(MANIFEST_HASH_FILE);
    let recorded = std::fs::read_to_string(&manifest_hash_path)
        .map_err(|_| VerifyError::MissingFile(MANIFEST_HASH_FILE.to_string()))?;
    let recomputed = compute_hash_tree(root)?;

    // Compare line-by-line so the error names the offending file.
    let recorded_map = parse_tree(&recorded);
    let recomputed_map = parse_tree(&recomputed);

    for (path, hash) in &recomputed_map {
        match recorded_map.get(path) {
            None => return Err(VerifyError::UnlistedFile(path.clone())),
            Some(rec) if rec != hash => {
                return Err(VerifyError::HashMismatch { path: path.clone() })
            }
            _ => {}
        }
    }
    // A listed file that no longer exists is also tampering.
    for path in recorded_map.keys() {
        if !recomputed_map.contains_key(path) {
            return Err(VerifyError::HashMismatch { path: path.clone() });
        }
    }
    Ok(content_hash(&recorded))
}

fn parse_tree(s: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for line in s.lines() {
        if let Some((hash, path)) = line.split_once("  ") {
            m.insert(path.to_string(), hash.to_string());
        }
    }
    m
}

/// Parse a publisher/verifying key from its hex encoding (optionally `ed25519:` prefixed).
fn parse_verifying_key(s: &str) -> Result<VerifyingKey, VerifyError> {
    let hexpart = s.strip_prefix("ed25519:").unwrap_or(s);
    let bytes =
        hex::decode(hexpart).map_err(|e| VerifyError::InvalidPublisherKey(e.to_string()))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| VerifyError::InvalidPublisherKey("expected 32-byte key".into()))?;
    VerifyingKey::from_bytes(&arr).map_err(|e| VerifyError::InvalidPublisherKey(e.to_string()))
}

/// Verify the detached signature over `MANIFEST.sha256` using the manifest's publisher key,
/// and enforce trust-tier rules. Returns the content hash on success.
pub fn verify_signature(
    root: &Path,
    manifest: &Manifest,
    policy: &TrustPolicy,
) -> Result<String, VerifyError> {
    let content = verify_hashes(root)?;

    let sig_path = root.join(SIGNATURE_FILE);
    if !sig_path.exists() {
        if policy.require_signature {
            return Err(VerifyError::UnsignedNotAllowed);
        }
        return Ok(content);
    }

    let sig_hex = std::fs::read_to_string(&sig_path)
        .map_err(|e| VerifyError::Io(e.to_string()))?
        .trim()
        .to_string();
    let sig_bytes =
        hex::decode(&sig_hex).map_err(|e| VerifyError::InvalidSignature(e.to_string()))?;
    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| VerifyError::InvalidSignature("expected 64-byte signature".into()))?;
    let signature = Signature::from_bytes(&sig_arr);

    let key = parse_verifying_key(&manifest.trust.publisher)?;

    // The signed message is the canonical MANIFEST.sha256 content.
    let signed_msg =
        std::fs::read(root.join(MANIFEST_HASH_FILE)).map_err(|e| VerifyError::Io(e.to_string()))?;

    key.verify(&signed_msg, &signature)
        .map_err(|_| VerifyError::SignatureRejected)?;

    // Verified tier must be signed by a KRIA-trusted key.
    if manifest
        .trust
        .declared_tier
        .eq_ignore_ascii_case("verified")
    {
        let pub_hex = hex::encode(key.to_bytes());
        let trusted = policy.trusted_keys.iter().any(|k| {
            k.strip_prefix("ed25519:")
                .unwrap_or(k)
                .eq_ignore_ascii_case(&pub_hex)
        });
        if !trusted {
            return Err(VerifyError::UntrustedPublisher("verified".into()));
        }
    }

    Ok(content)
}

// ── Packaging helpers (used by tests + future `ocskill pack`) ──────────────────

/// Write `MANIFEST.sha256` for a bundle directory.
pub fn write_hash_tree(root: &Path) -> Result<String, VerifyError> {
    let tree = compute_hash_tree(root)?;
    std::fs::write(root.join(MANIFEST_HASH_FILE), tree.as_bytes())
        .map_err(|e| VerifyError::Io(e.to_string()))?;
    Ok(content_hash(&tree))
}

/// Sign a bundle directory's `MANIFEST.sha256` with `signing_key`, writing `bundle.sig`.
pub fn sign_bundle(root: &Path, signing_key: &SigningKey) -> Result<(), VerifyError> {
    let msg = std::fs::read(root.join(MANIFEST_HASH_FILE))
        .map_err(|_| VerifyError::MissingFile(MANIFEST_HASH_FILE.to_string()))?;
    let sig = signing_key.sign(&msg);
    std::fs::write(root.join(SIGNATURE_FILE), hex::encode(sig.to_bytes()))
        .map_err(|e| VerifyError::Io(e.to_string()))?;
    Ok(())
}

/// Deterministic keypair from a 32-byte seed (test/dev convenience; publisher hex + signer).
pub fn keypair_from_seed(seed: [u8; 32]) -> (SigningKey, String) {
    let sk = SigningKey::from_bytes(&seed);
    let pub_hex = hex::encode(sk.verifying_key().to_bytes());
    (sk, pub_hex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_bundle(dir: &Path, publisher_hex: &str) {
        std::fs::create_dir_all(dir.join("handler")).unwrap();
        let manifest = format!(
            r#"
[skill]
slug = "oc_demo"
name = "Demo"
version = "1.0.0"
category = "productivity"
description = "demo skill"
min_kria = "0.1.0"
[runtime]
kind = "docker"
entry = "handler/demo.js"
[resource]
class = "light"
[trust]
declared_tier = "community"
publisher = "{publisher_hex}"
"#
        );
        std::fs::write(dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(dir.join("schema.json"), r#"{"type":"object"}"#).unwrap();
        std::fs::write(dir.join("handler/demo.js"), "module.exports=()=>({})").unwrap();
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let (sk, pub_hex) = keypair_from_seed([7u8; 32]);
        let td = TempDir::new().unwrap();
        make_bundle(td.path(), &pub_hex);
        write_hash_tree(td.path()).unwrap();
        sign_bundle(td.path(), &sk).unwrap();

        let manifest =
            Manifest::parse(&std::fs::read_to_string(td.path().join("manifest.toml")).unwrap())
                .unwrap();
        let policy = TrustPolicy::strict();
        assert!(verify_signature(td.path(), &manifest, &policy).is_ok());
    }

    #[test]
    fn tamper_is_detected() {
        let (sk, pub_hex) = keypair_from_seed([9u8; 32]);
        let td = TempDir::new().unwrap();
        make_bundle(td.path(), &pub_hex);
        write_hash_tree(td.path()).unwrap();
        sign_bundle(td.path(), &sk).unwrap();

        // Tamper with a file after signing.
        std::fs::write(
            td.path().join("handler/demo.js"),
            "module.exports=()=>({evil:1})",
        )
        .unwrap();
        let manifest =
            Manifest::parse(&std::fs::read_to_string(td.path().join("manifest.toml")).unwrap())
                .unwrap();
        let err = verify_signature(td.path(), &manifest, &TrustPolicy::strict()).unwrap_err();
        assert!(matches!(err, VerifyError::HashMismatch { .. }));
    }

    #[test]
    fn wrong_publisher_key_rejected() {
        let (sk, _pub_hex) = keypair_from_seed([1u8; 32]);
        let (_sk2, other_pub) = keypair_from_seed([2u8; 32]);
        let td = TempDir::new().unwrap();
        // Manifest claims a DIFFERENT publisher than the signer.
        make_bundle(td.path(), &other_pub);
        write_hash_tree(td.path()).unwrap();
        sign_bundle(td.path(), &sk).unwrap();
        let manifest =
            Manifest::parse(&std::fs::read_to_string(td.path().join("manifest.toml")).unwrap())
                .unwrap();
        let err = verify_signature(td.path(), &manifest, &TrustPolicy::strict()).unwrap_err();
        assert!(matches!(err, VerifyError::SignatureRejected));
    }

    #[test]
    fn unsigned_rejected_under_strict() {
        let (_sk, pub_hex) = keypair_from_seed([3u8; 32]);
        let td = TempDir::new().unwrap();
        make_bundle(td.path(), &pub_hex);
        write_hash_tree(td.path()).unwrap();
        let manifest =
            Manifest::parse(&std::fs::read_to_string(td.path().join("manifest.toml")).unwrap())
                .unwrap();
        let err = verify_signature(td.path(), &manifest, &TrustPolicy::strict()).unwrap_err();
        assert!(matches!(err, VerifyError::UnsignedNotAllowed));
    }
}
