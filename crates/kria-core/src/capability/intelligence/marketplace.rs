//! Wave 6 — Marketplace Intelligence (neutral, provider-agnostic).
//!
//! This module is the Brain's marketplace *reasoning* layer. It never touches a
//! provider-native type or a provider name: it operates purely over
//! [`CapabilityDescriptor`]s produced by any provider's `catalog()` and over
//! neutral integrity/versioning primitives. That keeps the Brain/Hands invariant
//! (spec R9/R23) — a new marketplace backend (ClawHub, an MCP registry, a git
//! catalog) needs zero changes here.
//!
//! Contents:
//! - **6.1** [`CatalogRanker`] — neutral catalog ranking from trust/quality/cost/
//!   adoption signals ([`CatalogRankingPolicy`]).
//! - **6.2** [`ArtifactVerifier`] + [`Quarantine`] — artifact hash + signature
//!   verification with quarantine on failure.
//! - **6.3** [`CatalogCache`] — TTL cache with explicit invalidation.
//! - **6.4** [`CapabilityCoordinate`], [`DependencySpec`], [`version_satisfies`]
//!   — namespacing + semver versioning + dependency metadata.
//! - **6.5** [`ClawHubListing`] — the neutral ClawHub schema (publishing,
//!   reviews, ratings, signatures, deps, update channels, breaking-version,
//!   compatibility) that a future ClawHub backend serializes to/from.
//!
//! Everything here is inert until the `capability.intelligence.marketplace_v2`
//! flag wires it into the acquisition path, so flag-off parity holds
//! (spec Property 1).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::super::descriptor::CapabilityDescriptor;
use super::super::error::CapError;

// ─────────────────────────────────────────────────────────────────────────────
// 6.1 — Neutral catalog ranking
// ─────────────────────────────────────────────────────────────────────────────

/// Tunable weights for catalog ranking (data, not code). Signals are all derived
/// from the neutral [`CapabilityDescriptor`], never from a provider type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CatalogRankingPolicy {
    /// Weight for the trust signal (publisher/signed/tier).
    pub weight_trust: f32,
    /// Weight for the quality signal (stars + validator score).
    pub weight_quality: f32,
    /// Weight for the cost signal (cheaper/local ⇒ higher).
    pub weight_cost: f32,
    /// Weight for the adoption signal (usage count, log-scaled).
    pub weight_adoption: f32,
    /// Weight for the semantic relevance signal supplied by the caller.
    pub weight_relevance: f32,
    /// Usage count at/above which the adoption signal saturates to 1.0.
    pub adoption_saturation: f64,
}

impl Default for CatalogRankingPolicy {
    fn default() -> Self {
        Self {
            weight_trust: 0.25,
            weight_quality: 0.25,
            weight_cost: 0.15,
            weight_adoption: 0.15,
            weight_relevance: 0.20,
            adoption_saturation: 10_000.0,
        }
    }
}

/// The per-signal breakdown for one ranked catalog entry, kept for transparency
/// and the reasoning trace (never hidden).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankingSignals {
    pub trust: f32,
    pub quality: f32,
    pub cost: f32,
    pub adoption: f32,
    pub relevance: f32,
}

/// A catalog entry with its fused ranking score and signal breakdown.
#[derive(Debug, Clone)]
pub struct RankedCatalogEntry {
    pub descriptor: CapabilityDescriptor,
    /// Fused score 0.0..=1.0.
    pub score: f32,
    pub signals: RankingSignals,
}

/// Ranks marketplace catalog candidates using only neutral descriptor signals.
#[derive(Debug, Clone, Default)]
pub struct CatalogRanker {
    policy: CatalogRankingPolicy,
}

impl CatalogRanker {
    pub fn new(policy: CatalogRankingPolicy) -> Self {
        Self { policy }
    }

    /// Map an open trust-tier string to a neutral 0.0..=1.0 prior. Unknown tiers
    /// get a conservative-low prior (never rejected — open vocabulary).
    fn tier_prior(tier: Option<&str>) -> f32 {
        match tier.map(|t| t.to_ascii_lowercase()) {
            // NOTE: check "untrusted"/"unknown" first — "untrusted" contains the
            // substring "trusted", which would otherwise mis-match as trusted.
            Some(t) if t.contains("untrusted") || t.contains("unknown") => 0.2,
            Some(t) if t.contains("verified") || t.contains("official") => 1.0,
            Some(t) if t.contains("trusted") => 0.85,
            Some(t) if t.contains("community") => 0.6,
            Some(t) if t.contains("local") => 0.7,
            Some(_) => 0.4, // named but unrecognized tier → conservative-low
            None => 0.3,    // undeclared → conservative
        }
    }

    fn trust_signal(d: &CapabilityDescriptor) -> f32 {
        let mut s = Self::tier_prior(d.trust.tier.as_deref());
        if d.trust.signed {
            s = (s + 1.0) / 2.0; // a verified signature pulls toward trusted
        }
        if d.trust.publisher.is_some() {
            s = (s + s.max(0.5)) / 2.0;
        }
        s.clamp(0.0, 1.0)
    }

    fn quality_signal(d: &CapabilityDescriptor) -> f32 {
        let stars = d.quality.stars.map(|s| (s / 5.0).clamp(0.0, 1.0));
        let validator = d.quality.validator_score.map(|s| s.clamp(0.0, 1.0));
        match (stars, validator) {
            (Some(a), Some(b)) => (a + b) / 2.0,
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => 0.5, // unrated → neutral prior
        }
    }

    /// Cheaper/local ⇒ higher. Free = 1.0; metered scales down with amount;
    /// unknown = neutral 0.5. GPU requirement mildly reduces the signal.
    fn cost_signal(d: &CapabilityDescriptor) -> f32 {
        use super::super::descriptor::CostHint;
        let mut c = match d.expectations.as_ref().and_then(|e| e.cost.as_ref()) {
            Some(CostHint::Free) => 1.0,
            Some(CostHint::Metered { amount, .. }) => {
                // Diminishing: amount 0 → 1.0, larger amounts → smaller signal.
                (1.0 / (1.0 + amount.max(0.0))) as f32
            }
            None => 0.5,
        };
        if d.expectations.as_ref().and_then(|e| e.gpu_required) == Some(true) {
            c *= 0.9;
        }
        c.clamp(0.0, 1.0)
    }

    fn adoption_signal(&self, d: &CapabilityDescriptor) -> f32 {
        let count = d.stats.as_ref().map(|s| s.usage_count).unwrap_or(0) as f64;
        if count <= 0.0 {
            return 0.0;
        }
        // log-scaled and saturating so a few installs don't dominate trust.
        let sat = self.policy.adoption_saturation.max(1.0);
        ((1.0 + count).ln() / (1.0 + sat).ln()).clamp(0.0, 1.0) as f32
    }

    /// Rank the given catalog entries. `relevance` maps `(provider_id,
    /// capability_id)` → semantic relevance 0.0..=1.0 for the goal (supplied by
    /// the reasoner); missing entries default to 0.5 (neutral).
    pub fn rank(
        &self,
        catalog: &[CapabilityDescriptor],
        relevance: &HashMap<(String, String), f32>,
    ) -> Vec<RankedCatalogEntry> {
        let p = &self.policy;
        let wsum = (p.weight_trust
            + p.weight_quality
            + p.weight_cost
            + p.weight_adoption
            + p.weight_relevance)
            .max(f32::EPSILON);

        let mut out: Vec<RankedCatalogEntry> = catalog
            .iter()
            .map(|d| {
                let signals = RankingSignals {
                    trust: Self::trust_signal(d),
                    quality: Self::quality_signal(d),
                    cost: Self::cost_signal(d),
                    adoption: self.adoption_signal(d),
                    relevance: relevance
                        .get(&(d.provider_id.clone(), d.capability_id.clone()))
                        .copied()
                        .unwrap_or(0.5)
                        .clamp(0.0, 1.0),
                };
                let score = (p.weight_trust * signals.trust
                    + p.weight_quality * signals.quality
                    + p.weight_cost * signals.cost
                    + p.weight_adoption * signals.adoption
                    + p.weight_relevance * signals.relevance)
                    / wsum;
                RankedCatalogEntry {
                    descriptor: d.clone(),
                    score: score.clamp(0.0, 1.0),
                    signals,
                }
            })
            .collect();

        out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                // stable tie-break: prefer higher trust, then id for determinism
                .then_with(|| {
                    b.signals
                        .trust
                        .partial_cmp(&a.signals.trust)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| a.descriptor.capability_id.cmp(&b.descriptor.capability_id))
        });
        out
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 6.2 — Artifact integrity: hash + signature verify + quarantine
// ─────────────────────────────────────────────────────────────────────────────

/// A neutral digest algorithm tag (open-ended for future algorithms).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestAlgorithm {
    Sha256,
    Blake3,
}

/// A content digest: algorithm + lowercase hex value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Digest {
    pub algorithm: DigestAlgorithm,
    pub hex: String,
}

impl Digest {
    /// Compute a digest of `bytes` with the given algorithm.
    pub fn compute(algorithm: DigestAlgorithm, bytes: &[u8]) -> Self {
        let hex = match algorithm {
            DigestAlgorithm::Sha256 => {
                use sha2::{Digest as _, Sha256};
                let mut h = Sha256::new();
                h.update(bytes);
                hex::encode(h.finalize())
            }
            DigestAlgorithm::Blake3 => hex::encode(blake3::hash(bytes).as_bytes()),
        };
        Self { algorithm, hex }
    }

    /// Constant-time-ish equality on the hex string (lengths differ ⇒ mismatch).
    fn matches(&self, other_hex: &str) -> bool {
        let a = self.hex.as_bytes();
        let b = other_hex.as_bytes();
        if a.len() != b.len() {
            return false;
        }
        let mut diff = 0u8;
        for (x, y) in a.iter().zip(b.iter()) {
            diff |= x ^ y;
        }
        diff == 0
    }
}

/// The verdict of verifying one artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "verdict")]
pub enum IntegrityVerdict {
    /// Hash matched and (if a key was supplied) the signature verified.
    Verified { signed: bool },
    /// The content hash did not match the expected digest.
    HashMismatch { expected: String, actual: String },
    /// A signature was supplied but did not verify against the public key.
    SignatureInvalid { reason: String },
    /// No expected hash was supplied — cannot verify integrity (honest, not a
    /// silent pass).
    NoExpectedHash,
}

impl IntegrityVerdict {
    /// True only for a positively-verified artifact.
    pub fn is_verified(&self) -> bool {
        matches!(self, IntegrityVerdict::Verified { .. })
    }
}

/// Optional ed25519 signature material for an artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// 32-byte ed25519 public key.
    pub public_key: Vec<u8>,
    /// 64-byte ed25519 signature over the raw artifact bytes.
    pub signature: Vec<u8>,
}

/// Verifies artifact integrity (hash) and authenticity (ed25519 signature).
#[derive(Debug, Clone, Default)]
pub struct ArtifactVerifier;

impl ArtifactVerifier {
    /// Verify `bytes` against an `expected` digest and, if present, an ed25519
    /// signature. Honest: a missing expected hash yields
    /// [`IntegrityVerdict::NoExpectedHash`] rather than a silent pass.
    pub fn verify(
        &self,
        bytes: &[u8],
        expected: Option<&Digest>,
        signature: Option<&Signature>,
    ) -> IntegrityVerdict {
        let Some(expected) = expected else {
            return IntegrityVerdict::NoExpectedHash;
        };
        let actual = Digest::compute(expected.algorithm, bytes);
        if !actual.matches(&expected.hex) {
            return IntegrityVerdict::HashMismatch {
                expected: expected.hex.clone(),
                actual: actual.hex,
            };
        }
        if let Some(sig) = signature {
            match Self::verify_ed25519(bytes, sig) {
                Ok(()) => IntegrityVerdict::Verified { signed: true },
                Err(reason) => IntegrityVerdict::SignatureInvalid { reason },
            }
        } else {
            IntegrityVerdict::Verified { signed: false }
        }
    }

    fn verify_ed25519(bytes: &[u8], sig: &Signature) -> Result<(), String> {
        use ed25519_dalek::{Signature as EdSig, Verifier, VerifyingKey};
        let key_arr: [u8; 32] = sig
            .public_key
            .as_slice()
            .try_into()
            .map_err(|_| "public key must be 32 bytes".to_string())?;
        let vk = VerifyingKey::from_bytes(&key_arr).map_err(|e| e.to_string())?;
        let sig_arr: [u8; 64] = sig
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| "signature must be 64 bytes".to_string())?;
        let ed_sig = EdSig::from_bytes(&sig_arr);
        vk.verify(bytes, &ed_sig).map_err(|e| e.to_string())
    }
}

/// Tracks capabilities whose artifacts failed integrity/signature verification.
/// A quarantined capability must not be activated (spec R8.3). In-memory here;
/// durability is the CKB's job when wired.
#[derive(Debug, Clone, Default)]
pub struct Quarantine {
    entries: HashMap<(String, String), String>,
}

impl Quarantine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Quarantine a capability with a reason (idempotent — reason updated).
    pub fn quarantine(
        &mut self,
        provider_id: &str,
        capability_id: &str,
        reason: impl Into<String>,
    ) {
        self.entries
            .insert((provider_id.into(), capability_id.into()), reason.into());
    }

    /// True when the capability is quarantined.
    pub fn is_quarantined(&self, provider_id: &str, capability_id: &str) -> bool {
        self.entries
            .contains_key(&(provider_id.into(), capability_id.into()))
    }

    /// The quarantine reason, if any.
    pub fn reason(&self, provider_id: &str, capability_id: &str) -> Option<&str> {
        self.entries
            .get(&(provider_id.into(), capability_id.into()))
            .map(|s| s.as_str())
    }

    /// All quarantined capabilities as `(provider_id, capability_id, reason)`,
    /// for the oversight/marketplace UI (spec R8.3 visibility).
    pub fn list(&self) -> Vec<(String, String, String)> {
        self.entries
            .iter()
            .map(|((p, c), r)| (p.clone(), c.clone(), r.clone()))
            .collect()
    }

    /// Release a capability from quarantine (e.g. after re-verify). Returns
    /// whether it was quarantined.
    pub fn release(&mut self, provider_id: &str, capability_id: &str) -> bool {
        self.entries
            .remove(&(provider_id.into(), capability_id.into()))
            .is_some()
    }

    /// Verify then quarantine-on-failure in one step. Returns the verdict; the
    /// caller must not activate unless [`IntegrityVerdict::is_verified`].
    pub fn verify_and_gate(
        &mut self,
        verifier: &ArtifactVerifier,
        provider_id: &str,
        capability_id: &str,
        bytes: &[u8],
        expected: Option<&Digest>,
        signature: Option<&Signature>,
    ) -> IntegrityVerdict {
        let verdict = verifier.verify(bytes, expected, signature);
        if !verdict.is_verified() {
            self.quarantine(
                provider_id,
                capability_id,
                format!("integrity gate failed: {verdict:?}"),
            );
        }
        verdict
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 6.3 — Catalog cache with explicit invalidation
// ─────────────────────────────────────────────────────────────────────────────

struct CacheSlot {
    catalog: Vec<CapabilityDescriptor>,
    stored_at: Instant,
}

/// A per-provider catalog cache with TTL and explicit invalidation. Avoids
/// re-fetching a marketplace catalog on every goal miss while keeping staleness
/// bounded + manually flushable (spec R8.3).
pub struct CatalogCache {
    ttl: Duration,
    slots: HashMap<String, CacheSlot>,
}

impl CatalogCache {
    /// Create a cache with the given time-to-live.
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            slots: HashMap::new(),
        }
    }

    /// Store a provider's catalog snapshot.
    pub fn put(&mut self, provider_id: &str, catalog: Vec<CapabilityDescriptor>) {
        self.slots.insert(
            provider_id.to_string(),
            CacheSlot {
                catalog,
                stored_at: Instant::now(),
            },
        );
    }

    /// Fetch a provider's catalog if present and not past TTL. A stale slot is
    /// left in place (explicit invalidation is the contract) but not returned.
    pub fn get(&self, provider_id: &str) -> Option<&[CapabilityDescriptor]> {
        self.slots.get(provider_id).and_then(|slot| {
            if slot.stored_at.elapsed() <= self.ttl {
                Some(slot.catalog.as_slice())
            } else {
                None
            }
        })
    }

    /// True when a fresh (non-expired) entry exists.
    pub fn is_fresh(&self, provider_id: &str) -> bool {
        self.get(provider_id).is_some()
    }

    /// Explicitly invalidate one provider's cache. Returns whether an entry was
    /// removed.
    pub fn invalidate(&mut self, provider_id: &str) -> bool {
        self.slots.remove(provider_id).is_some()
    }

    /// Explicitly invalidate the entire cache.
    pub fn invalidate_all(&mut self) {
        self.slots.clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 6.4 — Namespacing, semver versioning, dependency metadata
// ─────────────────────────────────────────────────────────────────────────────

/// A namespaced capability coordinate: `namespace/name` (e.g. `acme/pdf-ocr`).
/// Namespacing prevents id collisions across publishers (spec R8.4).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapabilityCoordinate {
    pub namespace: String,
    pub name: String,
}

impl CapabilityCoordinate {
    /// Parse `"namespace/name"`. Both parts must be non-empty and contain no
    /// whitespace or a second `/`. Honest error on malformed input.
    pub fn parse(s: &str) -> Result<Self, CapError> {
        let s = s.trim();
        let (ns, name) = s.split_once('/').ok_or_else(|| {
            CapError::Descriptor(format!(
                "capability coordinate '{s}' must be 'namespace/name'"
            ))
        })?;
        if ns.is_empty() || name.is_empty() {
            return Err(CapError::Descriptor(format!(
                "capability coordinate '{s}' has an empty namespace or name"
            )));
        }
        if name.contains('/') || s.chars().any(|c| c.is_whitespace()) {
            return Err(CapError::Descriptor(format!(
                "capability coordinate '{s}' is malformed"
            )));
        }
        Ok(Self {
            namespace: ns.to_string(),
            name: name.to_string(),
        })
    }
}

impl std::fmt::Display for CapabilityCoordinate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.namespace, self.name)
    }
}

/// A dependency of a capability: a coordinate + a semver requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencySpec {
    pub coordinate: CapabilityCoordinate,
    /// A semver requirement string, e.g. `">=1.2, <2.0"`.
    pub version_req: String,
    /// Whether this dependency is optional (feature-gated).
    #[serde(default)]
    pub optional: bool,
}

/// True when `version` (a semver) satisfies `req` (a semver requirement).
/// Honest error when either is unparsable (never a silent true).
pub fn version_satisfies(version: &str, req: &str) -> Result<bool, CapError> {
    let v = semver::Version::parse(version.trim())
        .map_err(|e| CapError::Descriptor(format!("bad version '{version}': {e}")))?;
    let r = semver::VersionReq::parse(req.trim())
        .map_err(|e| CapError::Descriptor(format!("bad version req '{req}': {e}")))?;
    Ok(r.matches(&v))
}

impl CapabilityCoordinate {
    /// Build a coordinate from a provider id + capability id (the neutral
    /// identity used across ranking, decisions, and provenance).
    pub fn from_ids(provider_id: &str, capability_id: &str) -> Self {
        Self {
            namespace: provider_id.to_string(),
            name: capability_id.to_string(),
        }
    }
}

impl DependencySpec {
    /// Parse the declared dependency list from a descriptor's `extensions`
    /// (`extensions["dependencies"] = [{coordinate, version_req, optional?}]`).
    /// Absent/malformed ⇒ empty (a capability with no declared deps). Neutral:
    /// any provider that advertises deps in this shape is understood uniformly.
    pub fn list_from_descriptor(d: &CapabilityDescriptor) -> Vec<DependencySpec> {
        let Some(arr) = d.extensions.get("dependencies").and_then(|v| v.as_array()) else {
            return Vec::new();
        };
        arr.iter()
            .filter_map(|item| {
                let coord = item.get("coordinate")?.as_str()?;
                let version_req = item
                    .get("version_req")
                    .and_then(|v| v.as_str())
                    .unwrap_or("*")
                    .to_string();
                let optional = item
                    .get("optional")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                Some(DependencySpec {
                    coordinate: CapabilityCoordinate::parse(coord).ok()?,
                    version_req,
                    optional,
                })
            })
            .collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 6.2b — Brain-owned trust policy (the install/trust *decision* lives in KRIA)
// ─────────────────────────────────────────────────────────────────────────────

/// Neutral trust ranking for an open-vocabulary tier string. The Brain owns this
/// decision (vision: KRIA decides, OpenClaw executes). Higher = more trusted.
pub fn trust_tier_rank(tier: Option<&str>) -> u8 {
    match tier.map(|t| t.to_ascii_lowercase()) {
        // NOTE: check "untrusted" first — it contains the substring "trusted".
        Some(t) if t.contains("untrusted") => 0,
        // Synthesized (KRIA-generated) is the LOWEST *installable* tier: above
        // "untrusted" (so it can be installed for review) but explicitly the
        // least-trusted known tier. Its execution is gated by declared effects
        // (conservative/elevated), so it never bypasses permission (spec R7.2).
        Some(t) if t.contains("synthesized") => 1,
        Some(t) if t.contains("verified") || t.contains("official") => 4,
        Some(t) if t.contains("trusted") || t.contains("local") => 3,
        Some(t) if t.contains("community") => 2,
        Some(_) => 1, // named but unrecognized → low-but-not-blocked
        None => 1,    // undeclared → conservative-low
    }
}

/// The Brain's policy for whether an acquired capability is trusted enough to
/// activate (spec R8.3). Data, not code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrustPolicy {
    /// Require a verified signature on the artifact to activate.
    pub require_signature: bool,
    /// Minimum acceptable trust-tier rank (see [`trust_tier_rank`]). Anything
    /// below is quarantined rather than activated.
    pub min_tier_rank: u8,
}

impl Default for TrustPolicy {
    fn default() -> Self {
        // Conservative-but-non-nagging default: block only explicitly untrusted
        // tiers (rank 0); leave signature enforcement to the byte-level installer
        // unless a caller opts in. This preserves legacy install behavior for
        // community skills while giving the Brain a real veto over untrusted ones.
        Self {
            require_signature: false,
            min_tier_rank: 1,
        }
    }
}

/// The Brain's trust decision for an acquired capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "verdict")]
pub enum TrustVerdict {
    /// Trusted enough to activate.
    Trusted,
    /// Untrusted — must be quarantined, not activated (with a reason).
    Untrusted { reason: String },
}

impl TrustVerdict {
    pub fn is_trusted(&self) -> bool {
        matches!(self, TrustVerdict::Trusted)
    }
}

impl TrustPolicy {
    /// Evaluate an acquired capability's declared trust against this policy.
    pub fn evaluate(&self, trust: &super::super::descriptor::TrustInfo) -> TrustVerdict {
        let rank = trust_tier_rank(trust.tier.as_deref());
        if rank < self.min_tier_rank {
            return TrustVerdict::Untrusted {
                reason: format!(
                    "trust tier {:?} (rank {rank}) below policy minimum {}",
                    trust.tier, self.min_tier_rank
                ),
            };
        }
        if self.require_signature && !trust.signed {
            return TrustVerdict::Untrusted {
                reason: "policy requires a verified signature but artifact is unsigned".into(),
            };
        }
        TrustVerdict::Trusted
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 6.5 — ClawHub model (neutral schema)
// ─────────────────────────────────────────────────────────────────────────────

/// An update channel a published version belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
    Nightly,
}

/// A user review of a listing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub author: String,
    /// 1..=5 stars.
    pub stars: u8,
    #[serde(default)]
    pub comment: String,
    pub created_at: String,
}

/// Aggregate rating for a listing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rating {
    /// Mean stars 0.0..=5.0.
    pub average: f32,
    pub count: u64,
}

/// One published version of a capability on the marketplace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublishedVersion {
    /// Semver version string.
    pub version: String,
    /// The artifact content digest (integrity, 6.2). `None` until the artifact is
    /// downloaded/pinned — catalog listings often advertise no hash, and a hash
    /// is never fabricated (honest provenance).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_digest: Option<Digest>,
    /// Optional publisher signature material (base64/hex encoded by transport).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_hex: Option<String>,
    /// Optional publisher public key (hex), paired with `signature_hex`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key_hex: Option<String>,
    /// Declared dependencies (6.4).
    #[serde(default)]
    pub dependencies: Vec<DependencySpec>,
    /// Update channel.
    #[serde(default)]
    pub channel: UpdateChannel,
    /// Whether this version is a breaking change vs the previous stable.
    #[serde(default)]
    pub breaking: bool,
    /// Open compatibility tags (host/OS/runtime), matched structurally.
    #[serde(default)]
    pub compatibility: Vec<String>,
    /// ISO-8601 publish timestamp.
    pub published_at: String,
    /// Whether this version is yanked (must not be freshly installed).
    #[serde(default)]
    pub yanked: bool,
}

/// A ClawHub marketplace listing: the full published history + social/trust
/// metadata for one namespaced capability. This is the neutral *schema* — a
/// ClawHub backend adapter serializes to/from it; KRIA-core owns no ClawHub HTTP.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClawHubListing {
    pub coordinate: CapabilityCoordinate,
    pub publisher: String,
    #[serde(default)]
    pub description: String,
    /// All published versions (any order; helpers sort).
    #[serde(default)]
    pub versions: Vec<PublishedVersion>,
    #[serde(default)]
    pub rating: Rating,
    #[serde(default)]
    pub reviews: Vec<Review>,
    /// Open trust-tier string mirrored into installed descriptors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_tier: Option<String>,
}

impl ClawHubListing {
    /// The latest non-yanked version on the given channel (by semver), if any.
    pub fn latest_on_channel(&self, channel: UpdateChannel) -> Option<&PublishedVersion> {
        self.versions
            .iter()
            .filter(|v| !v.yanked && v.channel == channel)
            .filter_map(|v| {
                semver::Version::parse(v.version.trim())
                    .ok()
                    .map(|sv| (sv, v))
            })
            .max_by(|(a, _), (b, _)| a.cmp(b))
            .map(|(_, v)| v)
    }

    /// Project this listing into installable (not-yet-installed) neutral
    /// [`CapabilityDescriptor`]s — the canonical catalog representation a
    /// provider adapter emits from its marketplace index. Uses the latest stable
    /// version; carries trust tier + `installed=false` + `version`.
    pub fn to_catalog_descriptors(&self, provider_id: &str) -> Vec<CapabilityDescriptor> {
        let latest = self
            .latest_on_channel(UpdateChannel::Stable)
            .or_else(|| self.versions.iter().find(|v| !v.yanked));
        let Some(pv) = latest else {
            return Vec::new();
        };
        let mut d = CapabilityDescriptor::minimal(
            provider_id,
            &self.coordinate.name,
            &self.coordinate.name,
            &self.description,
            serde_json::json!({}),
        );
        d.version = pv.version.clone();
        d.trust.tier = self.trust_tier.clone();
        d.trust.publisher = Some(self.publisher.clone());
        d.trust.signed = pv.signature_hex.is_some();
        d.quality.stars = if self.rating.count > 0 {
            Some(self.rating.average)
        } else {
            None
        };
        d.extensions
            .insert("installed".to_string(), serde_json::Value::Bool(false));
        if !pv.dependencies.is_empty() {
            let deps: Vec<serde_json::Value> = pv
                .dependencies
                .iter()
                .map(|dep| {
                    serde_json::json!({
                        "coordinate": dep.coordinate.to_string(),
                        "version_req": dep.version_req,
                        "optional": dep.optional,
                    })
                })
                .collect();
            d.extensions
                .insert("dependencies".to_string(), serde_json::Value::Array(deps));
        }
        vec![d]
    }

    /// The highest non-yanked version satisfying a semver requirement, if any.
    pub fn resolve(&self, req: &str) -> Result<Option<&PublishedVersion>, CapError> {
        let r = semver::VersionReq::parse(req.trim())
            .map_err(|e| CapError::Descriptor(format!("bad version req '{req}': {e}")))?;
        let best = self
            .versions
            .iter()
            .filter(|v| !v.yanked)
            .filter_map(|v| {
                semver::Version::parse(v.version.trim())
                    .ok()
                    .map(|sv| (sv, v))
            })
            .filter(|(sv, _)| r.matches(sv))
            .max_by(|(a, _), (b, _)| a.cmp(b))
            .map(|(_, v)| v);
        Ok(best)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::{
        CostHint, Expectations, QualitySignals, TrustInfo, UsageStats,
    };

    fn desc(provider: &str, id: &str) -> CapabilityDescriptor {
        CapabilityDescriptor::minimal(provider, id, id, "test capability", serde_json::json!({}))
    }

    // ── 6.1 ranking ──────────────────────────────────────────────────────────

    #[test]
    fn verified_signed_outranks_untrusted() {
        let mut good = desc("p", "good");
        good.trust = TrustInfo {
            publisher: Some("acme".into()),
            signed: true,
            tier: Some("verified".into()),
        };
        good.quality = QualitySignals {
            stars: Some(5.0),
            validator_score: Some(0.9),
        };
        let mut bad = desc("p", "bad");
        bad.trust = TrustInfo {
            publisher: None,
            signed: false,
            tier: Some("untrusted".into()),
        };
        let ranker = CatalogRanker::default();
        let ranked = ranker.rank(&[bad.clone(), good.clone()], &HashMap::new());
        assert_eq!(ranked[0].descriptor.capability_id, "good");
        assert!(ranked[0].score > ranked[1].score);
    }

    #[test]
    fn adoption_and_cost_signals_move_score() {
        let mut popular_free = desc("p", "pop");
        popular_free.stats = Some(UsageStats {
            success_rate: 0.9,
            usage_count: 9_000,
            avg_latency_ms: 10,
        });
        popular_free.expectations = Some(Expectations {
            cost: Some(CostHint::Free),
            ..Default::default()
        });
        let mut niche_metered = desc("p", "niche");
        niche_metered.expectations = Some(Expectations {
            cost: Some(CostHint::Metered {
                unit: "call".into(),
                amount: 5.0,
            }),
            ..Default::default()
        });
        let ranker = CatalogRanker::default();
        let ranked = ranker.rank(&[niche_metered, popular_free], &HashMap::new());
        assert_eq!(ranked[0].descriptor.capability_id, "pop");
    }

    #[test]
    fn relevance_weight_breaks_ties() {
        let a = desc("p", "a");
        let b = desc("p", "b");
        let mut rel = HashMap::new();
        rel.insert(("p".to_string(), "b".to_string()), 1.0);
        rel.insert(("p".to_string(), "a".to_string()), 0.0);
        let ranked = CatalogRanker::default().rank(&[a, b], &rel);
        assert_eq!(ranked[0].descriptor.capability_id, "b");
    }

    // ── 6.2 integrity ─────────────────────────────────────────────────────────

    #[test]
    fn hash_match_verifies_and_mismatch_quarantines() {
        let bytes = b"artifact-bytes";
        let expected = Digest::compute(DigestAlgorithm::Sha256, bytes);
        let v = ArtifactVerifier;
        assert!(v.verify(bytes, Some(&expected), None).is_verified());

        let wrong = Digest {
            algorithm: DigestAlgorithm::Sha256,
            hex: "00".repeat(32),
        };
        let mut q = Quarantine::new();
        let verdict = q.verify_and_gate(&v, "p", "c", bytes, Some(&wrong), None);
        assert!(matches!(verdict, IntegrityVerdict::HashMismatch { .. }));
        assert!(q.is_quarantined("p", "c"));
        assert!(q.release("p", "c"));
        assert!(!q.is_quarantined("p", "c"));
    }

    #[test]
    fn blake3_digest_matches() {
        let bytes = b"hello";
        let d = Digest::compute(DigestAlgorithm::Blake3, bytes);
        let v = ArtifactVerifier;
        assert!(v.verify(bytes, Some(&d), None).is_verified());
    }

    #[test]
    fn missing_expected_hash_is_honest_not_a_pass() {
        let v = ArtifactVerifier;
        assert_eq!(v.verify(b"x", None, None), IntegrityVerdict::NoExpectedHash);
        assert!(!v.verify(b"x", None, None).is_verified());
    }

    #[test]
    fn ed25519_signature_verifies_and_bad_signature_fails() {
        use ed25519_dalek::{Signer, SigningKey};
        let bytes = b"signed-artifact";
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let sig = sk.sign(bytes);
        let expected = Digest::compute(DigestAlgorithm::Sha256, bytes);
        let v = ArtifactVerifier;

        let good = Signature {
            public_key: sk.verifying_key().to_bytes().to_vec(),
            signature: sig.to_bytes().to_vec(),
        };
        assert_eq!(
            v.verify(bytes, Some(&expected), Some(&good)),
            IntegrityVerdict::Verified { signed: true }
        );

        let mut tampered = good.clone();
        tampered.signature[0] ^= 0xFF;
        assert!(matches!(
            v.verify(bytes, Some(&expected), Some(&tampered)),
            IntegrityVerdict::SignatureInvalid { .. }
        ));
    }

    // ── 6.3 cache ─────────────────────────────────────────────────────────────

    #[test]
    fn cache_stores_and_invalidates() {
        let mut cache = CatalogCache::new(Duration::from_secs(60));
        cache.put("p", vec![desc("p", "c")]);
        assert!(cache.is_fresh("p"));
        assert_eq!(cache.get("p").unwrap().len(), 1);
        assert!(cache.invalidate("p"));
        assert!(cache.get("p").is_none());
        assert!(!cache.invalidate("p"));
    }

    #[test]
    fn cache_expires_after_ttl() {
        let mut cache = CatalogCache::new(Duration::from_millis(0));
        cache.put("p", vec![desc("p", "c")]);
        std::thread::sleep(Duration::from_millis(2));
        assert!(cache.get("p").is_none());
        assert!(!cache.is_fresh("p"));
    }

    // ── 6.4 coordinates + versioning ──────────────────────────────────────────

    #[test]
    fn coordinate_parses_and_rejects_malformed() {
        let c = CapabilityCoordinate::parse("acme/pdf-ocr").unwrap();
        assert_eq!(c.namespace, "acme");
        assert_eq!(c.name, "pdf-ocr");
        assert_eq!(c.to_string(), "acme/pdf-ocr");
        assert!(CapabilityCoordinate::parse("no-slash").is_err());
        assert!(CapabilityCoordinate::parse("/name").is_err());
        assert!(CapabilityCoordinate::parse("ns/").is_err());
        assert!(CapabilityCoordinate::parse("a/b/c").is_err());
    }

    #[test]
    fn semver_satisfies_and_breaking() {
        assert!(version_satisfies("1.4.2", ">=1.2, <2.0").unwrap());
        assert!(!version_satisfies("2.0.0", ">=1.2, <2.0").unwrap());
        assert!(version_satisfies("bad", ">=1.0").is_err());
    }

    #[test]
    fn trust_policy_blocks_untrusted_and_passes_community() {
        let policy = TrustPolicy::default();
        let community = super::super::super::descriptor::TrustInfo {
            publisher: Some("acme".into()),
            signed: false,
            tier: Some("community".into()),
        };
        assert!(policy.evaluate(&community).is_trusted());

        let untrusted = super::super::super::descriptor::TrustInfo {
            publisher: None,
            signed: false,
            tier: Some("untrusted".into()),
        };
        assert!(!policy.evaluate(&untrusted).is_trusted());

        let strict = TrustPolicy {
            require_signature: true,
            min_tier_rank: 1,
        };
        assert!(!strict.evaluate(&community).is_trusted()); // unsigned blocked
    }

    #[test]
    fn dependency_specs_parse_from_descriptor_extensions() {
        let mut d = desc("p", "c");
        d.extensions.insert(
            "dependencies".into(),
            serde_json::json!([
                {"coordinate": "acme/pdf", "version_req": ">=1.0, <2.0"},
                {"coordinate": "acme/ocr", "optional": true}
            ]),
        );
        let deps = DependencySpec::list_from_descriptor(&d);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].coordinate.to_string(), "acme/pdf");
        assert_eq!(deps[0].version_req, ">=1.0, <2.0");
        assert!(deps[1].optional);
        assert_eq!(
            DependencySpec::list_from_descriptor(&desc("p", "n")).len(),
            0
        );
    }

    // ── 6.5 ClawHub model ─────────────────────────────────────────────────────

    fn ver(v: &str, channel: UpdateChannel, yanked: bool) -> PublishedVersion {
        PublishedVersion {
            version: v.into(),
            artifact_digest: Some(Digest {
                algorithm: DigestAlgorithm::Sha256,
                hex: "ab".repeat(32),
            }),
            signature_hex: None,
            public_key_hex: None,
            dependencies: vec![],
            channel,
            breaking: false,
            compatibility: vec![],
            published_at: "2026-01-01T00:00:00Z".into(),
            yanked,
        }
    }

    #[test]
    fn clawhub_resolves_latest_and_respects_yank_and_channel() {
        let listing = ClawHubListing {
            coordinate: CapabilityCoordinate::parse("acme/tool").unwrap(),
            publisher: "acme".into(),
            description: "d".into(),
            versions: vec![
                ver("1.0.0", UpdateChannel::Stable, false),
                ver("1.2.0", UpdateChannel::Stable, false),
                ver("1.3.0", UpdateChannel::Stable, true), // yanked
                ver("2.0.0-beta.1", UpdateChannel::Beta, false),
            ],
            rating: Rating::default(),
            reviews: vec![],
            trust_tier: Some("verified".into()),
        };
        assert_eq!(
            listing
                .latest_on_channel(UpdateChannel::Stable)
                .unwrap()
                .version,
            "1.2.0" // 1.3.0 yanked
        );
        assert_eq!(
            listing.resolve(">=1.0, <2.0").unwrap().unwrap().version,
            "1.2.0"
        );
        assert!(listing.resolve(">=3.0").unwrap().is_none());
    }

    #[test]
    fn clawhub_listing_projects_to_catalog_descriptor() {
        let listing = ClawHubListing {
            coordinate: CapabilityCoordinate::parse("acme/pdf-ocr").unwrap(),
            publisher: "acme".into(),
            description: "extract text".into(),
            versions: vec![ver("1.2.0", UpdateChannel::Stable, false)],
            rating: Rating {
                average: 4.5,
                count: 8,
            },
            reviews: vec![],
            trust_tier: Some("verified".into()),
        };
        let descs = listing.to_catalog_descriptors("openclaw");
        assert_eq!(descs.len(), 1);
        assert_eq!(descs[0].version, "1.2.0");
        assert_eq!(descs[0].trust.tier.as_deref(), Some("verified"));
        assert_eq!(descs[0].quality.stars, Some(4.5));
        assert_eq!(
            descs[0].extensions.get("installed"),
            Some(&serde_json::Value::Bool(false))
        );
    }

    #[test]
    fn clawhub_listing_roundtrips_json() {
        let listing = ClawHubListing {
            coordinate: CapabilityCoordinate::parse("acme/tool").unwrap(),
            publisher: "acme".into(),
            description: "d".into(),
            versions: vec![ver("1.0.0", UpdateChannel::Stable, false)],
            rating: Rating {
                average: 4.5,
                count: 10,
            },
            reviews: vec![Review {
                author: "u".into(),
                stars: 5,
                comment: "good".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
            }],
            trust_tier: Some("verified".into()),
        };
        let j = serde_json::to_string(&listing).unwrap();
        let back: ClawHubListing = serde_json::from_str(&j).unwrap();
        assert_eq!(listing, back);
    }
}
