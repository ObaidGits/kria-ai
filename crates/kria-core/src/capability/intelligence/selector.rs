//! [`DefaultCapabilitySelector`] — confidence-based candidate comparison + path
//! selection (spec R3), the deterministic core of the reasoning pipeline.
//!
//! Given candidates already scored by the federated index (semantic ⊕ lexical —
//! *not* keyword rules) plus the CKB's learned success signal, it fuses a
//! calibrated confidence using the versioned [`ReasoningPolicy`] weights and
//! chooses an [`ExecutionPath`]:
//!
//! - a high-confidence **native/installed** candidate ⇒ `Reuse`/`Native` on the
//!   fast path, and — via the **native-sufficiency gate** (spec R3.6) — remote
//!   marketplace/generation is skipped;
//! - a confident but **not-installed** best candidate ⇒ `Acquire` (marketplace);
//! - below the action threshold ⇒ `Ask` (clarify), never act on a guess (R2.3/R3.3).
//!
//! Path selection emerges from confidence + kind + cost/risk priors — never from
//! provider names or prompt keywords (spec R3.2).

use std::collections::HashMap;
use std::sync::Arc;

use super::kind::{infer_family, infer_kind, CapabilityKind};
use super::{
    CapabilityKnowledge, CostVector, ExecutionPath, ReasoningPolicy, ScoredCandidate, Selection,
};
use crate::capability::index::ScoredDescriptor;

/// Default, provider-neutral capability selector.
pub struct DefaultCapabilitySelector {
    policy: ReasoningPolicy,
}

impl DefaultCapabilitySelector {
    pub fn new(policy: ReasoningPolicy) -> Self {
        Self { policy }
    }

    pub fn with_default_policy() -> Self {
        Self {
            policy: ReasoningPolicy::default(),
        }
    }

    pub fn policy(&self) -> &ReasoningPolicy {
        &self.policy
    }

    /// True when a candidate's kind is locally available (native or installed),
    /// i.e. a *reuse* path rather than acquisition.
    fn is_local(kind: &CapabilityKind) -> bool {
        matches!(
            kind,
            CapabilityKind::Native | CapabilityKind::Installed | CapabilityKind::Gui
        )
    }

    /// Fuse component signals into a calibrated confidence in `0.0..=1.0` using
    /// the policy weights. `success_rate` is the CKB learned rate (0.5 neutral);
    /// `trust`/`recency` default to neutral when unavailable (honest, not faked).
    fn confidence(
        &self,
        semantic: f32,
        lexical: f32,
        success_rate: f32,
        trust: f32,
        recency: f32,
    ) -> f32 {
        let p = &self.policy;
        // Learned success centered at 0 so a neutral 0.5 neither boosts nor hurts.
        let success_signal = (success_rate - 0.5) * 2.0; // -1.0..=1.0
        let raw = p.weight_semantic * semantic
            + p.weight_lexical * lexical
            + p.weight_success * (0.5 + 0.5 * success_signal)
            + p.weight_trust * trust
            + p.weight_recency * recency;
        let wsum = p.weight_semantic
            + p.weight_lexical
            + p.weight_success
            + p.weight_trust
            + p.weight_recency;
        if wsum <= 0.0 {
            return semantic.clamp(0.0, 1.0);
        }
        (raw / wsum).clamp(0.0, 1.0)
    }

    /// Async selection over index-scored candidates, consulting the CKB (if wired)
    /// for the learned success rate of each candidate before fusing confidence.
    pub async fn select(
        &self,
        scored: &[ScoredDescriptor],
        ckb: Option<&Arc<dyn CapabilityKnowledge>>,
    ) -> Selection {
        let mut rates: HashMap<(String, String), f32> = HashMap::new();
        if let Some(ckb) = ckb {
            for s in scored {
                let (p, c) = s.descriptor.key();
                let rate = ckb.success_rate(&p, &c).await;
                rates.insert((p, c), rate);
            }
        }
        self.select_with_rates(scored, &rates)
    }

    /// Select over index-scored candidates, given the CKB learned success rate
    /// per `(provider_id, capability_id)` (pre-fetched to keep this pure/sync).
    pub fn select_with_rates(
        &self,
        scored: &[ScoredDescriptor],
        success_rates: &HashMap<(String, String), f32>,
    ) -> Selection {
        let mut candidates: Vec<ScoredCandidate> = scored
            .iter()
            .map(|s| {
                let d = &s.descriptor;
                let kind = infer_kind(d);
                let family = infer_family(d);
                let key = d.key();
                let success = success_rates.get(&key).copied().unwrap_or(0.5);
                let trust = 0.5; // neutral until TrustInfo is numericized (P6)
                let recency = 0.5; // neutral until recency is surfaced (P8)
                let confidence = self.confidence(s.semantic, s.lexical, success, trust, recency);
                ScoredCandidate {
                    descriptor: d.clone(),
                    kind,
                    family,
                    semantic: s.semantic,
                    lexical: s.lexical,
                    learned_success: success,
                    trust,
                    recency,
                    cost: CostVector {
                        uncalibrated: true,
                        ..CostVector::default()
                    },
                    confidence,
                }
            })
            .collect();

        candidates.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let policy_version = self.policy.version;
        let Some(best) = candidates.first().cloned() else {
            return Selection {
                candidates,
                chosen: None,
                confidence: 0.0,
                path: ExecutionPath::Ask,
                rationale: "No candidate capabilities found; asking the user.".into(),
                policy_version,
            };
        };

        let best_key = best.descriptor.key();
        let best_local = Self::is_local(&best.kind);

        // Below the action threshold ⇒ never act; ask/clarify (R2.3 / R3.3).
        if best.confidence < self.policy.min_action_confidence {
            return Selection {
                candidates,
                chosen: None,
                confidence: best.confidence,
                path: ExecutionPath::Ask,
                rationale: format!(
                    "Top candidate confidence {:.2} < action threshold {:.2}; clarifying.",
                    best.confidence, self.policy.min_action_confidence
                ),
                policy_version,
            };
        }

        // Native/installed sufficiency gate (R3.6): a locally-available candidate
        // at/above the sufficiency threshold wins — skip marketplace/generation.
        if best_local && best.confidence >= self.policy.native_sufficiency_confidence {
            let path = if matches!(best.kind, CapabilityKind::Native) {
                ExecutionPath::Native
            } else {
                ExecutionPath::Reuse
            };
            return Selection {
                candidates,
                chosen: Some(best_key),
                confidence: best.confidence,
                path,
                rationale: format!(
                    "Local {:?} candidate sufficient (confidence {:.2} ≥ {:.2}); skipping marketplace.",
                    best.kind, best.confidence, self.policy.native_sufficiency_confidence
                ),
                policy_version,
            };
        }

        // Best is confident but NOT locally available ⇒ acquire from marketplace.
        if !best_local {
            return Selection {
                candidates,
                chosen: Some(best_key),
                confidence: best.confidence,
                path: ExecutionPath::Acquire,
                rationale: format!(
                    "Best candidate is {:?} (not installed), confidence {:.2}; acquire from marketplace.",
                    best.kind, best.confidence
                ),
                policy_version,
            };
        }

        // Local candidate above action but below sufficiency ⇒ still reuse it
        // (cheaper/lower-risk than acquiring), with a modest-confidence note.
        Selection {
            candidates,
            chosen: Some(best_key),
            confidence: best.confidence,
            path: if matches!(best.kind, CapabilityKind::Native) {
                ExecutionPath::Native
            } else {
                ExecutionPath::Reuse
            },
            rationale: format!(
                "Local candidate reused at modest confidence {:.2} (≥ action {:.2}).",
                best.confidence, self.policy.min_action_confidence
            ),
            policy_version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::CapabilityDescriptor;
    use crate::capability::index::ScoredDescriptor;

    fn scored(provider: &str, cap: &str, semantic: f32, lexical: f32) -> ScoredDescriptor {
        let mut d = CapabilityDescriptor::minimal(
            provider,
            cap,
            cap,
            "",
            serde_json::json!({"type":"object"}),
        );
        // Mirror real adapters: the Hands DECLARE their substrate kind (the Brain
        // never infers kind from a provider name). Map the test provider to a
        // declared kind for realistic selection behavior.
        let kind = if provider == "native" {
            "native"
        } else {
            "installed"
        };
        d.extensions
            .insert("kind".into(), serde_json::Value::String(kind.into()));
        ScoredDescriptor {
            descriptor: d,
            score: semantic,
            semantic,
            lexical,
        }
    }

    #[test]
    fn native_sufficiency_gate_skips_marketplace() {
        let sel = DefaultCapabilitySelector::with_default_policy();
        // A strong native candidate + a weaker remote (openclaw) one.
        let cands = vec![
            scored("native", "get_public_ip", 0.95, 0.9),
            scored("openclaw", "oc_ip_info", 0.6, 0.3),
        ];
        let out = sel.select_with_rates(&cands, &HashMap::new());
        assert_eq!(out.path, ExecutionPath::Native);
        assert_eq!(out.chosen, Some(("native".into(), "get_public_ip".into())));
        assert!(out.confidence >= sel.policy().native_sufficiency_confidence);
    }

    #[test]
    fn not_installed_best_goes_to_acquire() {
        let sel = DefaultCapabilitySelector::with_default_policy();
        // Only a strong marketplace candidate (openclaw = installed kind here,
        // so use an explicitly not-installed kind via extensions).
        let mut d = CapabilityDescriptor::minimal(
            "someprov",
            "pdf_extract",
            "pdf_extract",
            "",
            serde_json::json!({"type":"object"}),
        );
        d.extensions
            .insert("kind".into(), serde_json::json!("cloud_api"));
        let cands = vec![ScoredDescriptor {
            descriptor: d,
            score: 0.9,
            semantic: 0.9,
            lexical: 0.8,
        }];
        let out = sel.select_with_rates(&cands, &HashMap::new());
        assert_eq!(out.path, ExecutionPath::Acquire);
    }

    #[test]
    fn low_confidence_asks() {
        let sel = DefaultCapabilitySelector::with_default_policy();
        let cands = vec![scored("openclaw", "oc_ip_info", 0.1, 0.0)];
        let out = sel.select_with_rates(&cands, &HashMap::new());
        assert_eq!(out.path, ExecutionPath::Ask);
        assert!(out.chosen.is_none());
    }

    #[test]
    fn empty_candidates_asks() {
        let sel = DefaultCapabilitySelector::with_default_policy();
        let out = sel.select_with_rates(&[], &HashMap::new());
        assert_eq!(out.path, ExecutionPath::Ask);
    }

    #[test]
    fn learned_success_breaks_ties() {
        let sel = DefaultCapabilitySelector::with_default_policy();
        // Two equal installed candidates; the one with better history wins.
        let cands = vec![
            scored("openclaw", "cap_a", 0.7, 0.5),
            scored("openclaw", "cap_b", 0.7, 0.5),
        ];
        let mut rates = HashMap::new();
        rates.insert(("openclaw".to_string(), "cap_b".to_string()), 1.0);
        rates.insert(("openclaw".to_string(), "cap_a".to_string()), 0.0);
        let out = sel.select_with_rates(&cands, &rates);
        assert_eq!(out.chosen, Some(("openclaw".into(), "cap_b".into())));
    }
}
