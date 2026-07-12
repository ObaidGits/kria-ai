//! Wave 3.1 — the single **Planning Authority** + arbitration (spec R10, §9).
//!
//! This is the anti-proliferation keystone: for any turn, **exactly one** runtime
//! owns it (conversation, Settings-NLP, GUI/HTN automation, n8n workflow, a
//! native tool, or the Capability Reasoning Pipeline). The authority decides the
//! owner from neutral evidence — semantic relevance, declared-effects risk,
//! confidence, and (sticky) explicit user intent — and emits **one winner + a
//! rationale trace**. It never executes anything; it only selects ownership, so
//! capability planning stays a *producer* that feeds the existing HTN runtime and
//! never becomes a second graph engine (Property 3).
//!
//! Neutrality: [`PlanningDomain`] names KRIA-internal *execution-runtime roles*,
//! not capability providers, and the arbiter contains no provider names and no
//! hardcoded prompt/keyword branch — every input is evidence supplied by the
//! caller. Inert until the `capability.intelligence.routing_gate` flag wires it
//! into the turn router (flag-off ⇒ byte-identical legacy routing).

use serde::{Deserialize, Serialize};

use super::REASONING_POLICY_VERSION;

/// The execution-runtime that can own a turn. Open vocabulary via `Other` so a
/// new runtime role needs no code change; the named set matches §9.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningDomain {
    /// Plain conversational answer (LLM, no tools).
    Conversation,
    /// Settings / configuration natural-language changes.
    SettingsNlp,
    /// GUI / desktop automation via the HTN runtime.
    GuiHtn,
    /// n8n workflow execution.
    N8nWorkflow,
    /// A single native tool call.
    NativeTool,
    /// The Capability Reasoning Pipeline (CPP) — reason/compose/acquire/execute.
    CapabilityPipeline,
    /// Any runtime role not in the named set (open vocabulary).
    Other(String),
}

/// Neutral evidence for one candidate domain owning the turn. All fields are
/// supplied by the caller (semantic router / reasoner); the arbiter adds no
/// hidden knowledge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainEvidence {
    pub domain: PlanningDomain,
    /// Semantic relevance of the turn to this domain's corpus, 0.0..=1.0.
    pub semantic: f32,
    /// The domain's own confidence it can handle the turn, 0.0..=1.0.
    pub confidence: f32,
    /// Declared-effects risk of taking this path, 0.0..=1.0 (higher = riskier;
    /// used as a conservative tie-break, never to silently override intent).
    #[serde(default)]
    pub effects_risk: f32,
    /// True when the user explicitly asked for this domain (e.g. "run a
    /// workflow", "open settings"). Sticky: dominates scoring (spec §9).
    #[serde(default)]
    pub explicit_intent: bool,
}

impl DomainEvidence {
    /// Convenience constructor with default risk + no explicit intent.
    pub fn new(domain: PlanningDomain, semantic: f32, confidence: f32) -> Self {
        Self {
            domain,
            semantic: semantic.clamp(0.0, 1.0),
            confidence: confidence.clamp(0.0, 1.0),
            effects_risk: 0.0,
            explicit_intent: false,
        }
    }
}

/// Tunable arbitration policy (data, not code).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArbitrationPolicy {
    pub version: u32,
    /// Weight on semantic relevance.
    pub weight_semantic: f32,
    /// Weight on the domain's self-confidence.
    pub weight_confidence: f32,
    /// Penalty weight applied to effects-risk (subtractive).
    pub weight_risk_penalty: f32,
    /// Score bonus added to a candidate carrying explicit user intent — large so
    /// intent is effectively sticky, but still auditable as data.
    pub explicit_intent_bonus: f32,
    /// Minimum winning score below which the authority abstains (falls back to
    /// the caller's default — usually plain conversation).
    pub min_win_score: f32,
}

impl Default for ArbitrationPolicy {
    fn default() -> Self {
        Self {
            version: REASONING_POLICY_VERSION,
            weight_semantic: 0.55,
            weight_confidence: 0.45,
            weight_risk_penalty: 0.15,
            explicit_intent_bonus: 1.0,
            min_win_score: 0.15,
        }
    }
}

/// One candidate's computed arbitration score + its components (for the trace).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainScore {
    pub domain: PlanningDomain,
    pub score: f32,
    pub semantic: f32,
    pub confidence: f32,
    pub effects_risk: f32,
    pub explicit_intent: bool,
}

/// The arbitration outcome: exactly one winning domain (or `None` = abstain),
/// the ranked scores, and a human-readable rationale for the trace/audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArbitrationDecision {
    /// The single turn owner, or `None` when the authority abstains (caller uses
    /// its default runtime — conversation).
    pub winner: Option<PlanningDomain>,
    /// All candidates, best first.
    pub scores: Vec<DomainScore>,
    /// Whether the winner was chosen because of sticky explicit intent.
    pub sticky_applied: bool,
    /// Why this winner (for the reasoning trace + `CapabilityEvent`).
    pub rationale: String,
    pub policy_version: u32,
}

/// The single Planning Authority. Stateless + neutral: given the candidate
/// evidence for a turn, it returns exactly one winner + a trace.
#[derive(Debug, Clone, Default)]
pub struct PlanningAuthority {
    policy: ArbitrationPolicy,
}

impl PlanningAuthority {
    pub fn new(policy: ArbitrationPolicy) -> Self {
        Self { policy }
    }

    fn score(&self, e: &DomainEvidence) -> f32 {
        let p = &self.policy;
        let mut s = p.weight_semantic * e.semantic + p.weight_confidence * e.confidence
            - p.weight_risk_penalty * e.effects_risk;
        if e.explicit_intent {
            s += p.explicit_intent_bonus;
        }
        s
    }

    /// Arbitrate ownership of a turn among the candidate domains.
    ///
    /// `prior_sticky` is the domain that owned the *previous* turn in a
    /// multi-turn interaction; when a candidate matches it and no other candidate
    /// carries fresh explicit intent, the prior owner is nudged to win (sticky
    /// continuity, spec §9) — implemented as a small, auditable tie-break, not a
    /// hard override.
    pub fn arbitrate(
        &self,
        candidates: &[DomainEvidence],
        prior_sticky: Option<&PlanningDomain>,
    ) -> ArbitrationDecision {
        if candidates.is_empty() {
            return ArbitrationDecision {
                winner: None,
                scores: Vec::new(),
                sticky_applied: false,
                rationale: "no candidate domains — abstain to default runtime".into(),
                policy_version: self.policy.version,
            };
        }

        let any_explicit = candidates.iter().any(|c| c.explicit_intent);

        let mut scored: Vec<(DomainScore, bool)> = candidates
            .iter()
            .map(|e| {
                let mut score = self.score(e);
                // Sticky continuity: only when NO fresh explicit intent exists,
                // and only as a small nudge (half the min-win threshold).
                let sticky_nudge =
                    !any_explicit && prior_sticky == Some(&e.domain) && !e.explicit_intent;
                if sticky_nudge {
                    score += self.policy.min_win_score * 0.5;
                }
                (
                    DomainScore {
                        domain: e.domain.clone(),
                        score,
                        semantic: e.semantic,
                        confidence: e.confidence,
                        effects_risk: e.effects_risk,
                        explicit_intent: e.explicit_intent,
                    },
                    sticky_nudge,
                )
            })
            .collect();

        // Deterministic order: score desc, then explicit-intent, then lower risk.
        scored.sort_by(|a, b| {
            b.0.score
                .partial_cmp(&a.0.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.0.explicit_intent.cmp(&a.0.explicit_intent))
                .then_with(|| {
                    a.0.effects_risk
                        .partial_cmp(&b.0.effects_risk)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        let top = &scored[0];
        let sticky_applied = top.0.explicit_intent || top.1;
        let winner_domain = top.0.domain.clone();
        let win_score = top.0.score;

        let (winner, rationale) = if win_score < self.policy.min_win_score {
            (
                None,
                format!(
                    "top candidate {:?} scored {win_score:.3} < min_win_score {:.3} — abstain to default runtime",
                    winner_domain, self.policy.min_win_score
                ),
            )
        } else if top.0.explicit_intent {
            (
                Some(winner_domain.clone()),
                format!("explicit user intent selects {winner_domain:?} (score {win_score:.3})"),
            )
        } else if top.1 {
            (
                Some(winner_domain.clone()),
                format!(
                    "sticky continuity keeps {winner_domain:?} (score {win_score:.3}, no fresh explicit intent)"
                ),
            )
        } else {
            (
                Some(winner_domain.clone()),
                format!(
                    "highest evidence selects {winner_domain:?} (score {win_score:.3}: semantic {:.2}, confidence {:.2})",
                    top.0.semantic, top.0.confidence
                ),
            )
        };

        ArbitrationDecision {
            winner,
            scores: scored.into_iter().map(|(s, _)| s).collect(),
            sticky_applied,
            rationale,
            policy_version: self.policy.version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highest_evidence_wins_exactly_one() {
        let auth = PlanningAuthority::default();
        let cands = vec![
            DomainEvidence::new(PlanningDomain::Conversation, 0.3, 0.4),
            DomainEvidence::new(PlanningDomain::CapabilityPipeline, 0.9, 0.85),
            DomainEvidence::new(PlanningDomain::N8nWorkflow, 0.5, 0.5),
        ];
        let d = auth.arbitrate(&cands, None);
        assert_eq!(d.winner, Some(PlanningDomain::CapabilityPipeline));
        // exactly one winner; the rest are ranked but not selected
        assert_eq!(d.scores.len(), 3);
        assert!(!d.sticky_applied);
    }

    #[test]
    fn explicit_intent_is_sticky_and_dominates() {
        let auth = PlanningAuthority::default();
        // n8n has weak evidence but explicit intent; CPP has strong evidence.
        let mut n8n = DomainEvidence::new(PlanningDomain::N8nWorkflow, 0.2, 0.2);
        n8n.explicit_intent = true;
        let cpp = DomainEvidence::new(PlanningDomain::CapabilityPipeline, 0.95, 0.95);
        let d = auth.arbitrate(&[cpp, n8n], None);
        assert_eq!(d.winner, Some(PlanningDomain::N8nWorkflow));
        assert!(d.sticky_applied);
        assert!(d.rationale.contains("explicit user intent"));
    }

    #[test]
    fn prior_sticky_breaks_a_close_call_without_explicit_intent() {
        let auth = PlanningAuthority::default();
        // two near-equal candidates; prior owner should keep the turn.
        let a = DomainEvidence::new(PlanningDomain::CapabilityPipeline, 0.6, 0.6);
        let b = DomainEvidence::new(PlanningDomain::GuiHtn, 0.6, 0.6);
        let d = auth.arbitrate(&[a, b], Some(&PlanningDomain::GuiHtn));
        assert_eq!(d.winner, Some(PlanningDomain::GuiHtn));
        assert!(d.sticky_applied);
    }

    #[test]
    fn fresh_explicit_intent_overrides_prior_sticky() {
        let auth = PlanningAuthority::default();
        let mut cpp = DomainEvidence::new(PlanningDomain::CapabilityPipeline, 0.5, 0.5);
        cpp.explicit_intent = true;
        let gui = DomainEvidence::new(PlanningDomain::GuiHtn, 0.6, 0.6);
        let d = auth.arbitrate(&[gui, cpp], Some(&PlanningDomain::GuiHtn));
        assert_eq!(d.winner, Some(PlanningDomain::CapabilityPipeline));
    }

    #[test]
    fn abstains_when_all_evidence_is_weak() {
        let auth = PlanningAuthority::default();
        let cands = vec![
            DomainEvidence::new(PlanningDomain::CapabilityPipeline, 0.05, 0.05),
            DomainEvidence::new(PlanningDomain::N8nWorkflow, 0.02, 0.03),
        ];
        let d = auth.arbitrate(&cands, None);
        assert_eq!(d.winner, None);
        assert!(d.rationale.contains("abstain"));
    }

    #[test]
    fn risk_penalty_is_a_tiebreak_not_an_override() {
        let auth = PlanningAuthority::default();
        // equal semantic/confidence; the lower-risk path should edge ahead.
        let mut risky = DomainEvidence::new(PlanningDomain::CapabilityPipeline, 0.7, 0.7);
        risky.effects_risk = 0.9;
        let safe = DomainEvidence::new(PlanningDomain::NativeTool, 0.7, 0.7);
        let d = auth.arbitrate(&[risky, safe], None);
        assert_eq!(d.winner, Some(PlanningDomain::NativeTool));
    }

    #[test]
    fn empty_candidates_abstain() {
        let auth = PlanningAuthority::default();
        let d = auth.arbitrate(&[], None);
        assert_eq!(d.winner, None);
        assert!(d.scores.is_empty());
    }

    #[test]
    fn decision_roundtrips_json_for_trace() {
        let auth = PlanningAuthority::default();
        let d = auth.arbitrate(
            &[DomainEvidence::new(
                PlanningDomain::CapabilityPipeline,
                0.9,
                0.9,
            )],
            None,
        );
        let j = serde_json::to_string(&d).unwrap();
        let back: ArbitrationDecision = serde_json::from_str(&j).unwrap();
        assert_eq!(d, back);
    }
}
