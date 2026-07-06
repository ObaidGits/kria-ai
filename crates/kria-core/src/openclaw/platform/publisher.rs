//! A8.2 Publisher Identity — every publisher owns a stable ed25519 identity.
//!
//! Publisher identity is the (publisher_id, public_key) pair. All published skills
//! are signed by the publisher key (reusing `bundle::verify`). Profiles, trust level,
//! verification status and reputation are metadata the platform tracks and persists.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Verification status of a publisher (A8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationStatus {
    /// Unverified — community/unknown.
    Unverified,
    /// Identity verified by KRIA.
    Verified,
    /// Publisher key revoked — skills must not execute.
    Revoked,
}

impl VerificationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unverified => "unverified",
            Self::Verified => "verified",
            Self::Revoked => "revoked",
        }
    }
}

/// Trust level assigned to a publisher (A8.2 + A8.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PublisherTrust {
    /// Explicitly distrusted.
    Untrusted,
    /// Default community level.
    Community,
    /// KRIA-curated / verified.
    Verified,
    /// First-party (KRIA itself).
    FirstParty,
}

impl PublisherTrust {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::Community => "community",
            Self::Verified => "verified",
            Self::FirstParty => "first_party",
        }
    }
}

/// A publisher identity record (A8.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Publisher {
    /// Stable publisher id (e.g. "kria", "acme-corp").
    pub publisher_id: String,
    /// ed25519 public key (hex, optionally `ed25519:` prefixed) — the signing identity.
    pub public_key: String,
    /// Display name.
    pub display_name: String,
    /// Optional organization.
    pub organization: Option<String>,
    /// Optional website.
    pub website: Option<String>,
    /// Optional contact.
    pub contact: Option<String>,
    /// Trust level.
    pub trust: PublisherTrust,
    /// Verification status.
    pub verification: VerificationStatus,
    /// Additional signing certificate keys (rotation / multi-key).
    pub signing_certificates: Vec<String>,
    /// Reputation score in [0.0, 1.0].
    pub reputation: f64,
    /// Count of successfully published skills (history summary).
    pub published_count: u64,
    /// When first registered.
    pub registered_at: chrono::DateTime<chrono::Utc>,
}

impl Publisher {
    pub fn new(
        publisher_id: impl Into<String>,
        public_key: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Self {
        Self {
            publisher_id: publisher_id.into(),
            public_key: public_key.into(),
            display_name: display_name.into(),
            organization: None,
            website: None,
            contact: None,
            trust: PublisherTrust::Community,
            verification: VerificationStatus::Unverified,
            signing_certificates: Vec::new(),
            reputation: 0.5,
            published_count: 0,
            registered_at: chrono::Utc::now(),
        }
    }

    /// All keys this publisher may sign with (primary + certificates), normalized (no prefix).
    pub fn all_keys(&self) -> Vec<String> {
        let mut keys = vec![normalize_key(&self.public_key)];
        for c in &self.signing_certificates {
            keys.push(normalize_key(c));
        }
        keys
    }

    /// Whether this publisher is allowed to have executing skills.
    pub fn is_active(&self) -> bool {
        self.verification != VerificationStatus::Revoked && self.trust != PublisherTrust::Untrusted
    }
}

/// Normalize a key: strip `ed25519:` prefix and lowercase hex.
pub fn normalize_key(k: &str) -> String {
    k.strip_prefix("ed25519:").unwrap_or(k).to_lowercase()
}

/// In-memory publisher registry with a single authoritative map (A8.2).
///
/// Persistence is delegated to the platform store (JSON on disk) so publisher
/// metadata survives offline restarts (A8.10).
#[derive(Clone, Default)]
pub struct PublisherRegistry {
    inner: Arc<RwLock<HashMap<String, Publisher>>>,
}

/// Process-wide, singly-owned `PublisherRegistry` (publisher-revocation
/// enforcement fix, product gap 7/8). `PublisherRegistry` was previously
/// only ever constructed ad-hoc inside unit tests — no real install path
/// referenced ANY instance of it, so `revoke()` had zero effect on
/// anything real. This is the ONE shared instance both real install paths
/// (`BundleInstaller::install_inner`, `clawhub_install_skill`) now consult —
/// same single-authority pattern `openclaw::trust_runtime` already
/// established for the sibling Settings-knob-wiring fix.
static GLOBAL_PUBLISHER_REGISTRY: std::sync::OnceLock<PublisherRegistry> =
    std::sync::OnceLock::new();

/// The single, process-wide `PublisherRegistry` instance. Every real install
/// path and every admin/Settings command that registers/revokes/verifies a
/// publisher MUST go through this instance — never construct a second,
/// disconnected `PublisherRegistry::new()` in production code (tests
/// legitimately construct isolated instances to test the type in
/// isolation, as `platform/tests.rs` already does).
pub fn global() -> &'static PublisherRegistry {
    GLOBAL_PUBLISHER_REGISTRY.get_or_init(PublisherRegistry::new)
}

impl PublisherRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or replace a publisher.
    pub fn register(&self, publisher: Publisher) {
        self.inner
            .write()
            .unwrap()
            .insert(publisher.publisher_id.clone(), publisher);
    }

    pub fn get(&self, publisher_id: &str) -> Option<Publisher> {
        self.inner.read().unwrap().get(publisher_id).cloned()
    }

    /// Find the publisher owning a given signing key (primary or certificate).
    pub fn find_by_key(&self, key: &str) -> Option<Publisher> {
        let norm = normalize_key(key);
        self.inner
            .read()
            .unwrap()
            .values()
            .find(|p| p.all_keys().contains(&norm))
            .cloned()
    }

    /// Revoke a publisher (A8.6 publisher revocation). Returns true if found.
    pub fn revoke(&self, publisher_id: &str) -> bool {
        let mut map = self.inner.write().unwrap();
        if let Some(p) = map.get_mut(publisher_id) {
            p.verification = VerificationStatus::Revoked;
            p.trust = PublisherTrust::Untrusted;
            true
        } else {
            false
        }
    }

    /// Mark a publisher verified and set trust.
    pub fn verify(&self, publisher_id: &str, trust: PublisherTrust) -> bool {
        let mut map = self.inner.write().unwrap();
        if let Some(p) = map.get_mut(publisher_id) {
            p.verification = VerificationStatus::Verified;
            p.trust = trust;
            true
        } else {
            false
        }
    }

    /// Update reputation after an install/execution outcome.
    pub fn adjust_reputation(&self, publisher_id: &str, delta: f64) {
        let mut map = self.inner.write().unwrap();
        if let Some(p) = map.get_mut(publisher_id) {
            p.reputation = (p.reputation + delta).clamp(0.0, 1.0);
        }
    }

    pub fn all(&self) -> Vec<Publisher> {
        self.inner.read().unwrap().values().cloned().collect()
    }

    /// Trusted signing keys (hex) for all verified/first-party publishers — feeds `TrustPolicy`.
    pub fn trusted_keys(&self) -> Vec<String> {
        self.inner
            .read()
            .unwrap()
            .values()
            .filter(|p| {
                p.verification == VerificationStatus::Verified
                    && matches!(
                        p.trust,
                        PublisherTrust::Verified | PublisherTrust::FirstParty
                    )
            })
            .flat_map(|p| p.all_keys())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.inner.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().unwrap().is_empty()
    }
}
