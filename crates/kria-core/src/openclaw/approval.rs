//! Capability approval flow (A3.7 / security-contract §3).
//!
//! Capability set → risk → approval token (bound to slug+version+granted caps+budget+schema epoch)
//! → cached. Re-approval is required ONLY when the capability set widens; narrowing or an
//! unchanged/previously-approved set is auto-accepted. GREEN risk auto-approves.

use super::capability::{requires_reapproval, Capability};
use crate::safety::RiskLevel;
use dashmap::DashMap;
use sha2::{Digest, Sha256};

/// An approval token bound to the exact (slug, version, granted caps, budget, schema epoch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalToken {
    pub hash: String,
    pub risk: RiskLevel,
    pub issued_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Auto-approved (GREEN risk, or a narrowing/unchanged set).
    AutoApproved(ApprovalToken),
    /// A previously-approved identical grant set — reused from cache.
    Reused(ApprovalToken),
    /// Requires human-in-the-loop approval before execution (elevated + widened).
    NeedsHitl(ApprovalToken),
}

impl ApprovalDecision {
    pub fn token(&self) -> &ApprovalToken {
        match self {
            Self::AutoApproved(t) | Self::Reused(t) | Self::NeedsHitl(t) => t,
        }
    }
    pub fn is_approved(&self) -> bool {
        !matches!(self, Self::NeedsHitl(_))
    }
}

/// Process approval cache. Keyed by the approval hash.
#[derive(Default)]
pub struct ApprovalCache {
    approved: DashMap<String, ApprovalToken>,
}

impl ApprovalCache {
    pub fn new() -> Self {
        Self {
            approved: DashMap::new(),
        }
    }

    /// Deterministic approval hash over the identity-affecting inputs (excludes cosmetic fields).
    pub fn compute_hash(
        slug: &str,
        version: &str,
        granted: &[Capability],
        budget: &str,
        schema_epoch: &str,
    ) -> String {
        // Canonicalize the capability set: sorted JSON.
        let mut caps_json: Vec<String> = granted
            .iter()
            .map(|c| serde_json::to_string(c).unwrap_or_default())
            .collect();
        caps_json.sort();
        let payload = format!(
            "{slug}|{version}|{budget}|{schema_epoch}|{}",
            caps_json.join(",")
        );
        let mut h = Sha256::new();
        h.update(payload.as_bytes());
        hex::encode(h.finalize())
    }

    /// Evaluate approval for a grant set given the previously-installed capability set (if any).
    pub fn evaluate(
        &self,
        slug: &str,
        version: &str,
        granted: &[Capability],
        previous: Option<&[Capability]>,
        budget: &str,
        schema_epoch: &str,
        risk: RiskLevel,
    ) -> ApprovalDecision {
        let hash = Self::compute_hash(slug, version, granted, budget, schema_epoch);
        let token = ApprovalToken {
            hash: hash.clone(),
            risk,
            issued_at: chrono::Utc::now(),
        };

        // Exact prior approval → reuse.
        if self.approved.contains_key(&hash) {
            return ApprovalDecision::Reused(token);
        }

        // GREEN risk auto-approves.
        if matches!(risk, RiskLevel::Green) {
            self.approved.insert(hash, token.clone());
            return ApprovalDecision::AutoApproved(token);
        }

        // Elevated risk: auto-approve only if this is NOT a widening vs the previous grant.
        if let Some(prev) = previous {
            if !requires_reapproval(prev, granted) {
                // Narrowing or unchanged → no new approval required.
                self.approved.insert(hash, token.clone());
                return ApprovalDecision::AutoApproved(token);
            }
        }

        // Elevated + new/widened → HITL required.
        ApprovalDecision::NeedsHitl(token)
    }

    /// Record that a token was approved (e.g. after a HITL prompt).
    pub fn record_approved(&self, token: &ApprovalToken) {
        self.approved.insert(token.hash.clone(), token.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::capability::{
        Capability, CapabilityKind, CapabilityMode, CapabilityScope,
    };

    fn cap(scope: CapabilityScope) -> Capability {
        Capability {
            kind: CapabilityKind::Network,
            mode: CapabilityMode::Egress,
            scope,
        }
    }

    #[test]
    fn green_auto_approves() {
        let c = ApprovalCache::new();
        let d = c.evaluate("oc_x", "1.0.0", &[], None, "light", "1", RiskLevel::Green);
        assert!(matches!(d, ApprovalDecision::AutoApproved(_)));
    }

    #[test]
    fn elevated_new_needs_hitl_then_reuses() {
        let c = ApprovalCache::new();
        let caps = vec![cap(CapabilityScope::Domains(vec!["a.com".into()]))];
        let d = c.evaluate(
            "oc_x",
            "1.0.0",
            &caps,
            None,
            "light",
            "1",
            RiskLevel::Yellow,
        );
        assert!(matches!(d, ApprovalDecision::NeedsHitl(_)));
        // Simulate HITL approval.
        c.record_approved(d.token());
        let d2 = c.evaluate(
            "oc_x",
            "1.0.0",
            &caps,
            None,
            "light",
            "1",
            RiskLevel::Yellow,
        );
        assert!(matches!(d2, ApprovalDecision::Reused(_)));
    }

    #[test]
    fn narrowing_does_not_need_reapproval() {
        let c = ApprovalCache::new();
        let old = vec![cap(CapabilityScope::Domains(vec![
            "a.com".into(),
            "b.com".into(),
        ]))];
        let narrowed = vec![cap(CapabilityScope::Domains(vec!["a.com".into()]))];
        let d = c.evaluate(
            "oc_x",
            "1.1.0",
            &narrowed,
            Some(&old),
            "light",
            "1",
            RiskLevel::Yellow,
        );
        assert!(matches!(d, ApprovalDecision::AutoApproved(_)));
    }

    #[test]
    fn widening_needs_reapproval() {
        let c = ApprovalCache::new();
        let old = vec![cap(CapabilityScope::Domains(vec!["a.com".into()]))];
        let widened = vec![cap(CapabilityScope::Domains(vec![
            "a.com".into(),
            "evil.com".into(),
        ]))];
        let d = c.evaluate(
            "oc_x",
            "1.1.0",
            &widened,
            Some(&old),
            "light",
            "1",
            RiskLevel::Yellow,
        );
        assert!(matches!(d, ApprovalDecision::NeedsHitl(_)));
    }
}
