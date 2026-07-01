//! Device pairing + signed device tokens for the mobile prompt-control path.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine;
use hmac::{Hmac, Mac};
use rand::RngCore;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

use crate::auth::SecretsVault;

type HmacSha256 = Hmac<Sha256>;

/// Vault key under which the per-install device-token signing key is stored.
const SIGNING_KEY_VAULT_ID: &str = "mobile/device_signing_key";
const TOKEN_VERSION: &str = "v1";
const DEFAULT_TOKEN_TTL_SECS: i64 = 24 * 3600;
const DEFAULT_PAIRING_TTL_SECS: i64 = 5 * 60;

/// Errors from the mobile pairing subsystem.
#[derive(Debug, thiserror::Error)]
pub enum MobileError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("vault error: {0}")]
    Vault(#[from] crate::auth::AuthError),
    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("invalid token format")]
    InvalidTokenFormat,
    #[error("token signature mismatch")]
    BadSignature,
    #[error("token expired")]
    Expired,
    #[error("device revoked or unknown")]
    Revoked,
    #[error("pairing code invalid or expired")]
    BadPairingCode,
    #[error("crypto error: {0}")]
    Crypto(String),
}

pub type Result<T> = std::result::Result<T, MobileError>;

/// Public-facing record for a paired device (never carries secrets).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub last_seen: i64,
    pub revoked: bool,
}

/// A pending pairing handshake returned by [`DeviceRegistry::begin_pairing`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingChallenge {
    /// Short single-use pairing code the phone submits to complete pairing.
    pub code: String,
    /// QR payload string the phone scans (`kria-pair://<host>/<code>`).
    pub qr_payload: String,
    /// Unix seconds when this pairing code expires.
    pub expires_at: i64,
}

struct PendingPairing {
    expires_at: i64,
}

/// Per-device pairing + token registry backed by SQLite, signed via the vault.
pub struct DeviceRegistry {
    conn: Mutex<Connection>,
    signing_key: Vec<u8>,
    token_ttl_secs: i64,
    pairing_ttl_secs: i64,
    pending: Mutex<HashMap<String, PendingPairing>>,
}

impl DeviceRegistry {
    /// Open (or create) the device registry at `db_path`, resolving the signing
    /// key from `vault` (generating + persisting one on first use).
    pub fn open(db_path: impl Into<PathBuf>, vault: &Arc<SecretsVault>) -> Result<Self> {
        let path = db_path.into();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS devices (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_seen INTEGER NOT NULL,
                revoked INTEGER NOT NULL DEFAULT 0
            );",
        )?;

        let signing_key = resolve_signing_key(vault)?;

        Ok(Self {
            conn: Mutex::new(conn),
            signing_key,
            token_ttl_secs: DEFAULT_TOKEN_TTL_SECS,
            pairing_ttl_secs: DEFAULT_PAIRING_TTL_SECS,
            pending: Mutex::new(HashMap::new()),
        })
    }

    /// Override the device-token TTL (seconds).
    pub fn with_token_ttl(mut self, secs: i64) -> Self {
        if secs > 0 {
            self.token_ttl_secs = secs;
        }
        self
    }

    /// Override the pairing-code TTL (seconds).
    pub fn with_pairing_ttl(mut self, secs: i64) -> Self {
        if secs > 0 {
            self.pairing_ttl_secs = secs;
        }
        self
    }

    /// Step 1 (on the laptop): create a single-use pairing code + QR payload.
    pub fn begin_pairing(&self, host: &str) -> PairingChallenge {
        let code = random_code();
        let expires_at = now() + self.pairing_ttl_secs;
        self.pending
            .lock()
            .unwrap()
            .insert(code.clone(), PendingPairing { expires_at });
        let qr_payload = format!("kria-pair://{host}/{code}");
        PairingChallenge {
            code,
            qr_payload,
            expires_at,
        }
    }

    /// Step 2 (from the phone): redeem a pairing code, register the device, and
    /// return a freshly signed device token. The code is consumed (single-use).
    pub fn complete_pairing(&self, code: &str, device_name: &str) -> Result<(DeviceInfo, String)> {
        {
            let mut pending = self.pending.lock().unwrap();
            let now_ts = now();
            // Drop expired entries opportunistically.
            pending.retain(|_, p| p.expires_at > now_ts);
            match pending.remove(code) {
                Some(p) if p.expires_at > now_ts => {}
                _ => return Err(MobileError::BadPairingCode),
            }
        }

        let id = Uuid::new_v4().to_string();
        let now_ts = now();
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO devices (id, name, created_at, last_seen, revoked)
                 VALUES (?1, ?2, ?3, ?3, 0)",
                rusqlite::params![id, device_name, now_ts],
            )?;
        }
        let token = self.issue_token(&id);
        let info = DeviceInfo {
            id,
            name: device_name.to_string(),
            created_at: now_ts,
            last_seen: now_ts,
            revoked: false,
        };
        Ok((info, token))
    }

    /// Issue a signed token for an existing (non-revoked) device.
    pub fn issue_token(&self, device_id: &str) -> String {
        let exp = now() + self.token_ttl_secs;
        let payload = format!("{device_id}.{exp}");
        let sig = self.sign(&payload);
        format!("{TOKEN_VERSION}.{payload}.{sig}")
    }

    /// Renew a token for a known device, returning a new token (or error if the
    /// device is unknown/revoked).
    pub fn renew(&self, device_id: &str) -> Result<String> {
        if !self.is_active(device_id)? {
            return Err(MobileError::Revoked);
        }
        self.touch_last_seen(device_id)?;
        Ok(self.issue_token(device_id))
    }

    /// Verify a device token: signature, expiry, and revocation. On success
    /// returns the device id and updates `last_seen`.
    pub fn verify_token(&self, token: &str) -> Result<String> {
        let parts: Vec<&str> = token.split('.').collect();
        // v1.<device_id>.<exp>.<sig>
        if parts.len() != 4 || parts[0] != TOKEN_VERSION {
            return Err(MobileError::InvalidTokenFormat);
        }
        let device_id = parts[1];
        let exp: i64 = parts[2]
            .parse()
            .map_err(|_| MobileError::InvalidTokenFormat)?;
        let payload = format!("{device_id}.{exp}");
        let expected = self.sign(&payload);
        // Constant-time-ish compare via HMAC verify.
        if !self.verify_sig(&payload, parts[3]) {
            let _ = expected; // keep behaviour explicit
            return Err(MobileError::BadSignature);
        }
        if now() >= exp {
            return Err(MobileError::Expired);
        }
        if !self.is_active(device_id)? {
            return Err(MobileError::Revoked);
        }
        self.touch_last_seen(device_id)?;
        Ok(device_id.to_string())
    }

    /// Revoke a device (instant access withdrawal). Returns true if it existed.
    pub fn revoke(&self, device_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "UPDATE devices SET revoked = 1 WHERE id = ?1",
            rusqlite::params![device_id],
        )?;
        Ok(n > 0)
    }

    /// List all registered devices (never includes tokens or keys).
    pub fn list_devices(&self) -> Result<Vec<DeviceInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, created_at, last_seen, revoked FROM devices ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(DeviceInfo {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
                last_seen: row.get(3)?,
                revoked: row.get::<_, i64>(4)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn is_active(&self, device_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let revoked: Option<i64> = conn
            .query_row(
                "SELECT revoked FROM devices WHERE id = ?1",
                rusqlite::params![device_id],
                |row| row.get(0),
            )
            .ok();
        Ok(matches!(revoked, Some(0)))
    }

    fn touch_last_seen(&self, device_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE devices SET last_seen = ?2 WHERE id = ?1",
            rusqlite::params![device_id, now()],
        )?;
        Ok(())
    }

    fn sign(&self, payload: &str) -> String {
        let mut mac =
            HmacSha256::new_from_slice(&self.signing_key).expect("HMAC accepts any key length");
        mac.update(payload.as_bytes());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    fn verify_sig(&self, payload: &str, sig_b64: &str) -> bool {
        let sig = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(sig_b64) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let mut mac =
            HmacSha256::new_from_slice(&self.signing_key).expect("HMAC accepts any key length");
        mac.update(payload.as_bytes());
        mac.verify_slice(&sig).is_ok()
    }
}

/// Resolve the signing key from the vault, generating + persisting one on first use.
fn resolve_signing_key(vault: &Arc<SecretsVault>) -> Result<Vec<u8>> {
    if let Some(b64) = vault.get(SIGNING_KEY_VAULT_ID) {
        let key = base64::engine::general_purpose::STANDARD.decode(b64.as_bytes())?;
        if key.len() == 32 {
            return Ok(key);
        }
    }
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    let b64 = base64::engine::general_purpose::STANDARD.encode(key);
    vault.set(SIGNING_KEY_VAULT_ID, b64)?;
    Ok(key.to_vec())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Generate a short, URL-safe single-use pairing code.
fn random_code() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> (tempfile::TempDir, DeviceRegistry) {
        std::env::set_var("KRIA_VAULT_PASSPHRASE", "mobile-test-pass-000000");
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(SecretsVault::open(dir.path().join("vault.enc"), dir.path()).unwrap());
        let reg = DeviceRegistry::open(dir.path().join("devices.db"), &vault).unwrap();
        (dir, reg)
    }

    #[test]
    #[serial_test::serial]
    fn pair_token_verify_roundtrip() {
        let (_d, reg) = registry();
        let challenge = reg.begin_pairing("100.64.0.1:8787");
        assert!(challenge.qr_payload.starts_with("kria-pair://"));
        let (info, token) = reg.complete_pairing(&challenge.code, "Pixel 8").unwrap();
        let verified = reg.verify_token(&token).unwrap();
        assert_eq!(verified, info.id);
    }

    #[test]
    #[serial_test::serial]
    fn pairing_code_is_single_use() {
        let (_d, reg) = registry();
        let challenge = reg.begin_pairing("h");
        reg.complete_pairing(&challenge.code, "dev").unwrap();
        let second = reg.complete_pairing(&challenge.code, "dev2");
        assert!(matches!(second, Err(MobileError::BadPairingCode)));
    }

    #[test]
    #[serial_test::serial]
    fn revoked_device_token_rejected() {
        let (_d, reg) = registry();
        let challenge = reg.begin_pairing("h");
        let (info, token) = reg.complete_pairing(&challenge.code, "dev").unwrap();
        assert!(reg.revoke(&info.id).unwrap());
        assert!(matches!(
            reg.verify_token(&token),
            Err(MobileError::Revoked)
        ));
    }

    #[test]
    #[serial_test::serial]
    fn tampered_token_rejected() {
        let (_d, reg) = registry();
        let challenge = reg.begin_pairing("h");
        let (_info, token) = reg.complete_pairing(&challenge.code, "dev").unwrap();
        let mut parts: Vec<&str> = token.split('.').collect();
        parts[3] = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let tampered = parts.join(".");
        assert!(matches!(
            reg.verify_token(&tampered),
            Err(MobileError::BadSignature)
        ));
    }

    #[test]
    #[serial_test::serial]
    fn expired_token_rejected() {
        let (_d, reg) = {
            std::env::set_var("KRIA_VAULT_PASSPHRASE", "mobile-test-pass-111111");
            let dir = tempfile::tempdir().unwrap();
            let vault =
                Arc::new(SecretsVault::open(dir.path().join("vault.enc"), dir.path()).unwrap());
            let reg = DeviceRegistry::open(dir.path().join("d.db"), &vault)
                .unwrap()
                .with_token_ttl(1);
            (dir, reg)
        };
        let challenge = reg.begin_pairing("h");
        let (_info, _token) = reg.complete_pairing(&challenge.code, "dev").unwrap();
        // Issue a token that is already expired by signing a past expiry.
        let device = reg.list_devices().unwrap()[0].id.clone();
        let exp = now() - 5;
        let payload = format!("{device}.{exp}");
        let sig = reg.sign(&payload);
        let token = format!("v1.{payload}.{sig}");
        assert!(matches!(
            reg.verify_token(&token),
            Err(MobileError::Expired)
        ));
    }
}
