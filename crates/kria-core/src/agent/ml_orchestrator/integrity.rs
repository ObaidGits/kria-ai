// crates/kria-core/src/agent/ml_orchestrator/integrity.rs
//
// Fast integrity checks: xxhash64 for datasets, SHA-256 for model weights.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use sha2::{Sha256, Digest};

use super::types::{HashAlgorithm, ContentHash};

/// Compute SHA-256 hex digest of a file.
pub fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Compute xxhash64 of a file.
pub fn xxhash64_file(path: &Path) -> anyhow::Result<u64> {
    use std::io::BufReader;
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = xxhash_rust::xxh64::Xxh64::new(0);
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.digest())
}

/// Compute the appropriate hash for an artifact based on its algorithm.
pub fn compute_hash(path: &Path, algo: HashAlgorithm) -> anyhow::Result<ContentHash> {
    match algo {
        HashAlgorithm::Sha256 => Ok(ContentHash::Sha256(sha256_file(path)?)),
        HashAlgorithm::Xxhash64 => Ok(ContentHash::Xxhash64(xxhash64_file(path)?)),
    }
}

/// Verify a file's hash matches the expected value.
pub fn verify_hash(path: &Path, expected: &ContentHash) -> anyhow::Result<bool> {
    let actual = match expected {
        ContentHash::Sha256(_) => {
            let h = sha256_file(path)?;
            ContentHash::Sha256(h)
        }
        ContentHash::Xxhash64(_) => {
            let h = xxhash64_file(path)?;
            ContentHash::Xxhash64(h)
        }
    };
    Ok(match (expected, &actual) {
        (ContentHash::Sha256(a), ContentHash::Sha256(b)) => a == b,
        (ContentHash::Xxhash64(a), ContentHash::Xxhash64(b)) => a == b,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn sha256_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"hello world").unwrap();
        let h1 = sha256_file(&path).unwrap();
        let h2 = sha256_file(&path).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex is 64 chars
    }

    #[test]
    fn xxhash64_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"hello world").unwrap();
        let h1 = xxhash64_file(&path).unwrap();
        let h2 = xxhash64_file(&path).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn verify_hash_matches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.pth");
        std::fs::write(&path, b"fake model weights").unwrap();
        let hash = compute_hash(&path, HashAlgorithm::Sha256).unwrap();
        assert!(verify_hash(&path, &hash).unwrap());
    }

    #[test]
    fn verify_hash_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.pth");
        std::fs::write(&path, b"fake model weights").unwrap();
        let hash = ContentHash::Sha256("0000000000000000000000000000000000000000000000000000000000000000".into());
        assert!(!verify_hash(&path, &hash).unwrap());
    }
}
