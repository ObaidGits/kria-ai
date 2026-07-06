//! A8.7 Trust Framework — ONE trust engine unifying skill/publisher/repository/
//! signature/runtime trust into a single admission decision.
//!
//! Builds on the frozen `bundle::verify::TrustPolicy` (ed25519 signatures) and the
//! publisher registry. No unsigned skill executes unless explicitly allowed by policy.

use super::publisher::{PublisherRegistry, PublisherTrust, VerificationStatus};
use crate::openclaw::bundle::verify::TrustPolicy;
use serde::{Deserialize, Serialize};

/// Trust level of a repository (A8.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RepositoryTrust {
    Untrusted,
    Community,
    Verified,
    FirstParty,
}

/// Enterprise/site policy knobs (A8.7 + A8.13 extension point).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnterprisePolicy {
    /// If true, only signed skills may install/execute.
    pub require_signature: bool,
    /// If true, only verified publishers are allowed.
    pub require_verified_publisher: bool,
    /// If true, unsigned skills may run when explicitly allowlisted below.
    pub allow_unsigned_allowlist: bool,
    /// Explicit skill-id allowlist for unsigned execution.
    pub unsigned_allowlist: Vec<String>,
    /// Minimum acceptable repository trust.
    pub min_repository_trust: RepositoryTrust,
}

impl Default for EnterprisePolicy {
    fn default() -> Self {
        // Secure default: signatures required, verified publishers not forced
        // (community skills allowed if signed), no unsigned execution.
        Self {
            require_signature: true,
            require_verified_publisher: false,
            allow_unsigned_allowlist: false,
            unsigned_allowlist: Vec::new(),
            min_repository_trust: RepositoryTrust::Community,
        }
    }
}

impl EnterprisePolicy {
    /// Relaxed policy for local development (unsigned allowed).
    pub fn permissive() -> Self {
        Self {
            require_signature: false,
            require_verified_publisher: false,
            allow_unsigned_allowlist: true,
            unsigned_allowlist: Vec::new(),
            min_repository_trust: RepositoryTrust::Untrusted,
        }
    }
}

/// A trust decision for a candidate skill (A8.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustDecision {
    /// Allowed to install/execute.
    Allow,
    /// Blocked with a reason.
    Deny(String),
}

impl TrustDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, TrustDecision::Allow)
    }
}

/// Inputs describing a candidate skill for a trust evaluation.
#[derive(Debug, Clone)]
pub struct TrustQuery<'a> {
    pub skill_id: &'a str,
    pub publisher_id: Option<&'a str>,
    pub signed: bool,
    pub signature_valid: bool,
    pub repository_trust: RepositoryTrust,
}

/// The single trust engine (A8.7).
#[derive(Clone)]
pub struct TrustFramework {
    publishers: PublisherRegistry,
    policy: EnterprisePolicy,
}

impl TrustFramework {
    pub fn new(publishers: PublisherRegistry, policy: EnterprisePolicy) -> Self {
        Self { publishers, policy }
    }

    pub fn policy(&self) -> &EnterprisePolicy {
        &self.policy
    }

    pub fn set_policy(&mut self, policy: EnterprisePolicy) {
        self.policy = policy;
    }

    /// Derive the frozen `bundle::verify::TrustPolicy` from the current publisher set +
    /// enterprise policy. This is the single bridge into the signing verification layer.
    pub fn verify_policy(&self) -> TrustPolicy {
        TrustPolicy {
            trusted_keys: self.publishers.trusted_keys(),
            require_signature: self.policy.require_signature,
        }
    }

    /// Evaluate whether a candidate skill is trusted enough to install/execute (A8.7).
    pub fn evaluate(&self, q: &TrustQuery) -> TrustDecision {
        // 1. Repository trust floor.
        if q.repository_trust < self.policy.min_repository_trust {
            return TrustDecision::Deny(format!(
                "repository trust {:?} below required {:?}",
                q.repository_trust, self.policy.min_repository_trust
            ));
        }

        // 2. Signature requirement.
        if !q.signed {
            let allowed = self.policy.allow_unsigned_allowlist
                && self
                    .policy
                    .unsigned_allowlist
                    .iter()
                    .any(|s| s == q.skill_id);
            if self.policy.require_signature && !allowed {
                return TrustDecision::Deny("unsigned skill and signatures are required".into());
            }
        } else if !q.signature_valid {
            return TrustDecision::Deny("signature present but invalid".into());
        }

        // 3. Publisher checks.
        match q.publisher_id {
            Some(pid) => match self.publishers.get(pid) {
                Some(p) => {
                    if p.verification == VerificationStatus::Revoked {
                        return TrustDecision::Deny(format!("publisher '{pid}' is revoked"));
                    }
                    if p.trust == PublisherTrust::Untrusted {
                        return TrustDecision::Deny(format!("publisher '{pid}' is untrusted"));
                    }
                    if self.policy.require_verified_publisher
                        && p.verification != VerificationStatus::Verified
                    {
                        return TrustDecision::Deny(format!(
                            "publisher '{pid}' is not verified (policy requires verification)"
                        ));
                    }
                }
                None => {
                    if self.policy.require_verified_publisher {
                        return TrustDecision::Deny(format!("unknown publisher '{pid}'"));
                    }
                }
            },
            None => {
                if self.policy.require_verified_publisher {
                    return TrustDecision::Deny("no publisher identity provided".into());
                }
            }
        }

        TrustDecision::Allow
    }
}
