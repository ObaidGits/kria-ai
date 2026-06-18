//! Encrypted on-device secrets vault.
//!
//! Stores all credentials (OAuth tokens, API keys) in a single AES-256-GCM
//! encrypted file. The master key is derived with Argon2id from
//! `KRIA_VAULT_PASSPHRASE` when set; otherwise a random key is persisted to a
//! `0600` keyfile with a security warning.
//!
//! File layout (`~/.kria/vault.enc`):
//! ```text
//! magic "KRV1" (4) | salt (16) | nonce (12) | ciphertext (AES-256-GCM)
//! ```
//! The ciphertext is `serde_json` of `HashMap<String, SecretEntry>`.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::{AuthError, Result};

const MAGIC: &[u8; 4] = b"KRV1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

/// A single stored secret with bookkeeping metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretEntry {
    /// The secret value (e.g. token JSON, API key).
    pub value: String,
    /// Unix seconds of last update.
    pub updated_at: i64,
    /// Free-form metadata (e.g. provider, scopes). Defaults to null.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl SecretEntry {
    fn new(value: String) -> Self {
        Self {
            value,
            updated_at: chrono::Utc::now().timestamp(),
            metadata: serde_json::Value::Null,
        }
    }
}

/// Encrypted credential store. Cheap to clone the `Arc`; not the struct.
pub struct SecretsVault {
    path: PathBuf,
    key: Zeroizing<[u8; KEY_LEN]>,
    salt: [u8; SALT_LEN],
    entries: RwLock<HashMap<String, SecretEntry>>,
}

impl SecretsVault {
    /// Open (or create) the vault at the default location
    /// (`~/.kria/vault.enc`), resolving the master key from the environment.
    pub fn open_default() -> Result<Self> {
        let paths = crate::platform::paths::KriaPaths::resolve();
        Self::open(paths.config_dir.join("vault.enc"), &paths.config_dir)
    }

    /// Open (or create) a vault at `path`. `key_dir` is where the fallback
    /// keyfile is stored when no passphrase is configured.
    pub fn open(path: impl Into<PathBuf>, key_dir: impl AsRef<Path>) -> Result<Self> {
        let path = path.into();
        let key_dir = key_dir.as_ref();

        // Determine salt: read from existing header, else generate.
        let (salt, existing) = match std::fs::read(&path) {
            Ok(bytes) => {
                let (salt, nonce, ct) = parse_file(&bytes)?;
                (salt, Some((nonce, ct)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut salt = [0u8; SALT_LEN];
                rand::thread_rng().fill_bytes(&mut salt);
                (salt, None)
            }
            Err(e) => return Err(AuthError::Io(e)),
        };

        let key = resolve_master_key(key_dir, &salt)?;

        let entries = match existing {
            Some((nonce, ct)) => {
                let plaintext = open_aead(&key, &nonce, &ct)?;
                serde_json::from_slice::<HashMap<String, SecretEntry>>(&plaintext)?
            }
            None => HashMap::new(),
        };

        Ok(Self {
            path,
            key,
            salt,
            entries: RwLock::new(entries),
        })
    }

    /// Get a secret value by key.
    pub fn get(&self, key: &str) -> Option<String> {
        self.entries
            .read()
            .ok()
            .and_then(|m| m.get(key).map(|e| e.value.clone()))
    }

    /// Get the full entry (value + metadata) by key.
    pub fn get_entry(&self, key: &str) -> Option<SecretEntry> {
        self.entries.read().ok().and_then(|m| m.get(key).cloned())
    }

    /// Set a secret value, persisting immediately.
    pub fn set(&self, key: &str, value: impl Into<String>) -> Result<()> {
        self.set_entry(key, SecretEntry::new(value.into()))
    }

    /// Set a full entry (value + metadata), persisting immediately.
    pub fn set_entry(&self, key: &str, mut entry: SecretEntry) -> Result<()> {
        entry.updated_at = chrono::Utc::now().timestamp();
        {
            let mut map = self
                .entries
                .write()
                .map_err(|_| AuthError::Crypto("vault lock poisoned".into()))?;
            map.insert(key.to_string(), entry);
        }
        self.persist()
    }

    /// Delete a secret. Returns true if it existed.
    pub fn delete(&self, key: &str) -> Result<bool> {
        let existed = {
            let mut map = self
                .entries
                .write()
                .map_err(|_| AuthError::Crypto("vault lock poisoned".into()))?;
            map.remove(key).is_some()
        };
        if existed {
            self.persist()?;
        }
        Ok(existed)
    }

    /// List all secret keys (never values).
    pub fn list(&self) -> Vec<String> {
        self.entries
            .read()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Encrypt and atomically write the vault to disk (`0600`).
    pub fn persist(&self) -> Result<()> {
        let plaintext = {
            let map = self
                .entries
                .read()
                .map_err(|_| AuthError::Crypto("vault lock poisoned".into()))?;
            serde_json::to_vec(&*map)?
        };

        let mut nonce = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ciphertext = seal_aead(&self.key, &nonce, &plaintext)?;

        let mut buf = Vec::with_capacity(4 + SALT_LEN + NONCE_LEN + ciphertext.len());
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&self.salt);
        buf.extend_from_slice(&nonce);
        buf.extend_from_slice(&ciphertext);

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("enc.tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&buf)?;
            f.flush()?;
            f.sync_all()?;
        }
        set_owner_only(&tmp)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

/// Parse the vault file into (salt, nonce, ciphertext).
fn parse_file(bytes: &[u8]) -> Result<([u8; SALT_LEN], [u8; NONCE_LEN], Vec<u8>)> {
    let min = 4 + SALT_LEN + NONCE_LEN;
    if bytes.len() < min {
        return Err(AuthError::Format("vault file too short".into()));
    }
    if &bytes[0..4] != MAGIC {
        return Err(AuthError::Format("bad magic header".into()));
    }
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&bytes[4..4 + SALT_LEN]);
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&bytes[4 + SALT_LEN..min]);
    let ciphertext = bytes[min..].to_vec();
    Ok((salt, nonce, ciphertext))
}

/// Resolve the 32-byte master key.
///
/// 1. `KRIA_VAULT_PASSPHRASE` → Argon2id(passphrase, salt).
/// 2. Fallback: random key persisted to `<key_dir>/vault.key` (mode 0600).
fn resolve_master_key(key_dir: &Path, salt: &[u8; SALT_LEN]) -> Result<Zeroizing<[u8; KEY_LEN]>> {
    if let Ok(pass) = std::env::var("KRIA_VAULT_PASSPHRASE") {
        if !pass.trim().is_empty() {
            let mut key = Zeroizing::new([0u8; KEY_LEN]);
            Argon2::default()
                .hash_password_into(pass.as_bytes(), salt, key.as_mut_slice())
                .map_err(|e| AuthError::Crypto(format!("argon2: {e}")))?;
            return Ok(key);
        }
    }

    let keyfile = key_dir.join("vault.key");
    if let Ok(bytes) = std::fs::read(&keyfile) {
        if bytes.len() == KEY_LEN {
            let mut key = Zeroizing::new([0u8; KEY_LEN]);
            key.copy_from_slice(&bytes);
            warn_keyfile();
            return Ok(key);
        }
    }

    // Generate a fresh random key and persist it (0600).
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    rand::thread_rng().fill_bytes(key.as_mut_slice());
    std::fs::create_dir_all(key_dir)?;
    {
        let mut f = std::fs::File::create(&keyfile)?;
        f.write_all(key.as_slice())?;
        f.flush()?;
        f.sync_all()?;
    }
    set_owner_only(&keyfile)?;
    warn_keyfile();
    Ok(key)
}

fn warn_keyfile() {
    tracing::warn!(
        target: "auth::vault",
        "secrets vault is using a local keyfile master key; set KRIA_VAULT_PASSPHRASE \
         for stronger at-rest protection"
    );
}

fn seal_aead(key: &[u8; KEY_LEN], nonce: &[u8; NONCE_LEN], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| AuthError::Crypto(format!("aes init: {e}")))?;
    cipher
        .encrypt(Nonce::from_slice(nonce), plaintext)
        .map_err(|_| AuthError::Crypto("aes-gcm encrypt failed".into()))
}

fn open_aead(key: &[u8; KEY_LEN], nonce: &[u8; NONCE_LEN], ciphertext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| AuthError::Crypto(format!("aes init: {e}")))?;
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| AuthError::Decrypt)
}

/// Restrict file permissions to the owner (mode 0600 on Unix; no-op elsewhere).
fn set_owner_only(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault() -> (tempfile::TempDir, SecretsVault) {
        std::env::set_var("KRIA_VAULT_PASSPHRASE", "test-passphrase-123456");
        let dir = tempfile::tempdir().unwrap();
        let vault = SecretsVault::open(dir.path().join("vault.enc"), dir.path()).unwrap();
        (dir, vault)
    }

    #[test]
    #[serial_test::serial]
    fn set_get_delete_roundtrip() {
        let (_dir, vault) = temp_vault();
        vault.set("api/foo", "secret-value").unwrap();
        assert_eq!(vault.get("api/foo").as_deref(), Some("secret-value"));
        assert!(vault.list().contains(&"api/foo".to_string()));
        assert!(vault.delete("api/foo").unwrap());
        assert_eq!(vault.get("api/foo"), None);
    }

    #[test]
    #[serial_test::serial]
    fn persists_across_reload() {
        std::env::set_var("KRIA_VAULT_PASSPHRASE", "reload-pass-abcdef");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        {
            let vault = SecretsVault::open(&path, dir.path()).unwrap();
            vault.set("k", "v").unwrap();
        }
        let reopened = SecretsVault::open(&path, dir.path()).unwrap();
        assert_eq!(reopened.get("k").as_deref(), Some("v"));
    }

    #[test]
    #[serial_test::serial]
    fn wrong_passphrase_fails_to_decrypt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        {
            std::env::set_var("KRIA_VAULT_PASSPHRASE", "correct-horse-battery");
            let vault = SecretsVault::open(&path, dir.path()).unwrap();
            vault.set("k", "v").unwrap();
        }
        std::env::set_var("KRIA_VAULT_PASSPHRASE", "totally-wrong-passphrase");
        let result = SecretsVault::open(&path, dir.path());
        assert!(matches!(result, Err(AuthError::Decrypt)));
    }
}
