use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use uuid::Uuid;

pub const DEFAULT_DRIFT_BUFFER_MS: i64 = 5_000;
pub const MAX_DRIFT_BUFFER_MS: i64 = 15_000;
const NONCE_RETAIN_BACKSTOP_MS: i64 = 60_000;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedEnvelope {
    pub version: u8,
    pub key_id: String,
    pub target_id: Uuid,
    pub lease_id: Uuid,
    pub nonce: String,
    pub sequence: u64,
    pub issued_at_wall_unix_ms: i64,
    pub issued_at_mono_ms: i64,
    pub ttl_ms: i64,
    pub drift_buffer_ms: i64,
    pub op: String,
    pub payload_hash_sha256_b64: String,
    pub payload: serde_json::Value,
    pub signature_hmac_sha256_b64: String,
}

#[derive(Clone, Debug)]
pub struct SignedEnvelopeInput {
    pub target_id: Uuid,
    pub lease_id: Uuid,
    pub nonce: String,
    pub sequence: u64,
    pub op: String,
    pub payload: serde_json::Value,
    pub ttl: Duration,
    pub drift_buffer_ms: i64,
}

#[derive(Clone, Debug)]
pub struct VerificationMetadata {
    pub used_key_id: String,
    pub valid_until_mono_ms: i64,
    pub effective_drift_buffer_ms: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    #[error("invalid ttl")]
    InvalidTtl,
    #[error("invalid drift buffer")]
    InvalidDriftBuffer,
    #[error("envelope expired")]
    EnvelopeExpired,
    #[error("envelope issued in unsupported future window")]
    IssuedInFutureWindow,
    #[error("payload hash mismatch")]
    PayloadHashMismatch,
    #[error("signature verification failed")]
    SignatureVerificationFailed,
    #[error("replay nonce detected")]
    ReplayNonceDetected,
    #[error("non-monotonic sequence: observed={observed}, expected_min={expected_min}")]
    NonMonotonicSequence { observed: u64, expected_min: u64 },
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("base64 decode error: {0}")]
    Base64Decode(#[from] base64::DecodeError),
    #[error("invalid signing key length: {0}")]
    InvalidSigningKey(#[from] hmac::digest::InvalidLength),
}

#[derive(Clone)]
pub struct KeyMaterial {
    pub key_id: String,
    pub secret: Vec<u8>,
}

#[derive(Clone)]
struct KeyRing {
    current: KeyMaterial,
    previous: Option<KeyMaterial>,
    previous_accept_until_mono_ms: i64,
}

pub struct DualKeyHmacEnvelopeSigner {
    key_ring: RwLock<KeyRing>,
    replay_nonces: RwLock<HashMap<(Uuid, Uuid, String), i64>>,
    sequence_watermarks: RwLock<HashMap<(Uuid, Uuid), u64>>,
    target_drift_override_ms: RwLock<HashMap<Uuid, i64>>,
}

impl DualKeyHmacEnvelopeSigner {
    pub fn new(current: KeyMaterial, previous: Option<KeyMaterial>, grace: Duration) -> Self {
        let now_mono = now_mono_ms();
        Self {
            key_ring: RwLock::new(KeyRing {
                current,
                previous,
                previous_accept_until_mono_ms: now_mono + grace.as_millis() as i64,
            }),
            replay_nonces: RwLock::new(HashMap::new()),
            sequence_watermarks: RwLock::new(HashMap::new()),
            target_drift_override_ms: RwLock::new(HashMap::new()),
        }
    }

    pub async fn rotate(&self, next_key: KeyMaterial, grace: Duration) {
        let mut ring = self.key_ring.write().await;
        let old_current = ring.current.clone();
        ring.previous = Some(old_current);
        ring.current = next_key;
        ring.previous_accept_until_mono_ms = now_mono_ms() + grace.as_millis() as i64;
    }

    pub async fn set_target_drift_buffer_ms(&self, target_id: Uuid, drift_buffer_ms: i64) {
        let mut overrides = self.target_drift_override_ms.write().await;
        overrides.insert(
            target_id,
            drift_buffer_ms.clamp(DEFAULT_DRIFT_BUFFER_MS, MAX_DRIFT_BUFFER_MS),
        );
    }

    pub async fn target_drift_buffer_ms(&self, target_id: Uuid) -> i64 {
        let overrides = self.target_drift_override_ms.read().await;
        overrides
            .get(&target_id)
            .copied()
            .unwrap_or(DEFAULT_DRIFT_BUFFER_MS)
    }

    pub async fn sign(&self, input: SignedEnvelopeInput) -> Result<SignedEnvelope, SignerError> {
        let ttl_ms = input.ttl.as_millis() as i64;
        if ttl_ms <= 0 {
            return Err(SignerError::InvalidTtl);
        }

        let drift_buffer_ms = input
            .drift_buffer_ms
            .clamp(DEFAULT_DRIFT_BUFFER_MS, MAX_DRIFT_BUFFER_MS);

        let payload_bytes = serde_json::to_vec(&input.payload)?;
        let payload_hash_sha256_b64 = sha256_b64(payload_bytes.as_slice());
        let issued_at_wall_unix_ms = now_unix_ms();
        let issued_at_mono_ms = now_mono_ms();

        let ring = self.key_ring.read().await;
        let canonical = CanonicalEnvelope {
            version: 1,
            target_id: input.target_id,
            lease_id: input.lease_id,
            nonce: input.nonce.clone(),
            sequence: input.sequence,
            issued_at_wall_unix_ms,
            issued_at_mono_ms,
            ttl_ms,
            drift_buffer_ms,
            op: input.op.clone(),
            payload_hash_sha256_b64: payload_hash_sha256_b64.clone(),
        };

        let signature_hmac_sha256_b64 = sign_with_key(&ring.current.secret, &canonical)?;

        Ok(SignedEnvelope {
            version: 1,
            key_id: ring.current.key_id.clone(),
            target_id: input.target_id,
            lease_id: input.lease_id,
            nonce: input.nonce,
            sequence: input.sequence,
            issued_at_wall_unix_ms,
            issued_at_mono_ms,
            ttl_ms,
            drift_buffer_ms,
            op: input.op,
            payload_hash_sha256_b64,
            payload: input.payload,
            signature_hmac_sha256_b64,
        })
    }

    pub async fn verify(
        &self,
        envelope: &SignedEnvelope,
    ) -> Result<VerificationMetadata, SignerError> {
        if envelope.ttl_ms <= 0 {
            return Err(SignerError::InvalidTtl);
        }

        if envelope.drift_buffer_ms < 0 || envelope.drift_buffer_ms > MAX_DRIFT_BUFFER_MS {
            return Err(SignerError::InvalidDriftBuffer);
        }

        let now_mono = now_mono_ms();
        let override_drift = {
            let overrides = self.target_drift_override_ms.read().await;
            overrides.get(&envelope.target_id).copied()
        };

        let effective_drift_buffer_ms = override_drift
            .unwrap_or(envelope.drift_buffer_ms)
            .clamp(DEFAULT_DRIFT_BUFFER_MS, MAX_DRIFT_BUFFER_MS);

        let valid_until_mono_ms =
            envelope.issued_at_mono_ms + envelope.ttl_ms + effective_drift_buffer_ms;

        if now_mono > valid_until_mono_ms {
            return Err(SignerError::EnvelopeExpired);
        }

        if envelope.issued_at_mono_ms > now_mono + effective_drift_buffer_ms {
            return Err(SignerError::IssuedInFutureWindow);
        }

        let observed_hash = sha256_b64(serde_json::to_vec(&envelope.payload)?.as_slice());
        if observed_hash != envelope.payload_hash_sha256_b64 {
            return Err(SignerError::PayloadHashMismatch);
        }

        let canonical = CanonicalEnvelope {
            version: envelope.version,
            target_id: envelope.target_id,
            lease_id: envelope.lease_id,
            nonce: envelope.nonce.clone(),
            sequence: envelope.sequence,
            issued_at_wall_unix_ms: envelope.issued_at_wall_unix_ms,
            issued_at_mono_ms: envelope.issued_at_mono_ms,
            ttl_ms: envelope.ttl_ms,
            drift_buffer_ms: envelope.drift_buffer_ms,
            op: envelope.op.clone(),
            payload_hash_sha256_b64: envelope.payload_hash_sha256_b64.clone(),
        };

        let ring = self.key_ring.read().await;
        let mut accepted_key_id: Option<String> = None;

        if envelope.key_id == ring.current.key_id
            && verify_with_key(
                &ring.current.secret,
                &canonical,
                envelope.signature_hmac_sha256_b64.as_str(),
            )?
        {
            accepted_key_id = Some(ring.current.key_id.clone());
        }

        if accepted_key_id.is_none() {
            if let Some(previous) = &ring.previous {
                if now_mono <= ring.previous_accept_until_mono_ms
                    && envelope.key_id == previous.key_id
                    && verify_with_key(
                        &previous.secret,
                        &canonical,
                        envelope.signature_hmac_sha256_b64.as_str(),
                    )?
                {
                    accepted_key_id = Some(previous.key_id.clone());
                }
            }
        }

        if accepted_key_id.is_none() {
            return Err(SignerError::SignatureVerificationFailed);
        }

        drop(ring);

        {
            let mut watermarks = self.sequence_watermarks.write().await;
            let key = (envelope.target_id, envelope.lease_id);
            if let Some(last_seen) = watermarks.get(&key).copied() {
                if envelope.sequence <= last_seen {
                    return Err(SignerError::NonMonotonicSequence {
                        observed: envelope.sequence,
                        expected_min: last_seen + 1,
                    });
                }
            }
            watermarks.insert(key, envelope.sequence);
        }

        {
            let replay_key = (
                envelope.target_id,
                envelope.lease_id,
                envelope.nonce.clone(),
            );
            let mut replay = self.replay_nonces.write().await;
            if replay.contains_key(&replay_key) {
                return Err(SignerError::ReplayNonceDetected);
            }
            replay.insert(replay_key, valid_until_mono_ms);

            let cutoff = now_mono - NONCE_RETAIN_BACKSTOP_MS;
            replay.retain(|_, expires_at| *expires_at > cutoff);
        }

        Ok(VerificationMetadata {
            used_key_id: accepted_key_id.expect("accepted key id must be present"),
            valid_until_mono_ms,
            effective_drift_buffer_ms,
        })
    }
}

#[derive(Serialize)]
struct CanonicalEnvelope {
    version: u8,
    target_id: Uuid,
    lease_id: Uuid,
    nonce: String,
    sequence: u64,
    issued_at_wall_unix_ms: i64,
    issued_at_mono_ms: i64,
    ttl_ms: i64,
    drift_buffer_ms: i64,
    op: String,
    payload_hash_sha256_b64: String,
}

fn sign_with_key(secret: &[u8], canonical: &CanonicalEnvelope) -> Result<String, SignerError> {
    let data = serde_json::to_vec(canonical)?;
    let mut mac = HmacSha256::new_from_slice(secret)?;
    mac.update(data.as_slice());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn verify_with_key(
    secret: &[u8],
    canonical: &CanonicalEnvelope,
    sig_b64: &str,
) -> Result<bool, SignerError> {
    let data = serde_json::to_vec(canonical)?;
    let sig = URL_SAFE_NO_PAD.decode(sig_b64.as_bytes())?;
    let mut mac = HmacSha256::new_from_slice(secret)?;
    mac.update(data.as_slice());
    Ok(mac.verify_slice(sig.as_slice()).is_ok())
}

fn sha256_b64(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

pub fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn now_mono_ms() -> i64 {
    static MONO_BASE: OnceLock<Instant> = OnceLock::new();
    MONO_BASE.get_or_init(Instant::now).elapsed().as_millis() as i64
}
