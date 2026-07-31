//! Evidence-driven cross-domain tool-injection gate (settings-nl-intelligence
//! Wave 5). Replaces the flat "top-K with confidence ≥ 0.35" injection — which
//! injected broad tools (`search_marketplace`, `recall_fact`, browser, …) into
//! unrelated turns — with a decision that fuses MULTIPLE independent evidence
//! sources, none dominating:
//!
//! - **semantic confidence** (the embedding cosine),
//! - **candidate competition** (distance from the best candidate — trailing,
//!   near-tie noise is penalised),
//! - **domain agreement** (does the candidate's category match the categories the
//!   semantic router already chose for this turn? cross-domain needs more signal),
//! - **negative evidence** (a conversation-leaning turn suppresses injections),
//! - an absolute floor + a "strong match always passes" escape hatch.
//!
//! It is fully generalized: NO tool names, NO prompt patterns. Behaviour comes
//! from category metadata + scores, so new tools/providers need no code change.
//! Every decision emits an explainable per-candidate trace.

use std::collections::HashSet;

/// One semantic candidate to consider injecting.
#[derive(Clone, Debug)]
pub struct InjectionCandidate {
    pub name: String,
    pub category: String,
    pub confidence: f32,
}

/// Turn evidence used to gate injections.
#[derive(Clone, Debug, Default)]
pub struct InjectionEvidence {
    /// Categories of the tools the semantic router selected for this turn.
    pub domain_categories: HashSet<String>,
    /// The turn leans conversational (negative evidence for tool injection).
    pub conversation_only: bool,
}

/// Tunable weights/thresholds (documented; calibrated, not per-prompt tuned).
#[derive(Clone, Copy, Debug)]
pub struct InjectionParams {
    /// Hard minimum cosine to be considered at all (baseline preserved).
    pub abs_floor: f32,
    /// A match at/above this cosine is accepted regardless of domain/competition.
    pub strong: f32,
    /// Bonus when the candidate's category agrees with the routed domain.
    pub domain_bonus: f32,
    /// Penalty when it does NOT (cross-domain injection needs more signal).
    pub cross_domain_penalty: f32,
    /// Penalty when the turn is conversation-leaning (negative evidence).
    pub conv_penalty: f32,
    /// How strongly distance-from-best suppresses trailing candidates.
    pub competition_weight: f32,
    /// Fused-score acceptance threshold.
    pub accept: f32,
    /// Max tools to inject.
    pub max: usize,
}

impl Default for InjectionParams {
    fn default() -> Self {
        Self {
            abs_floor: 0.35,
            strong: 0.60,
            domain_bonus: 0.12,
            cross_domain_penalty: 0.18,
            conv_penalty: 0.25,
            competition_weight: 0.6,
            accept: 0.42,
            max: 2,
        }
    }
}

/// Per-candidate explainable scoring record (persisted for observability).
#[derive(Clone, Debug)]
#[allow(dead_code)] // Fields written for trace/observability; consumed externally via InjectionDecision.
pub struct InjectionScore {
    pub name: String,
    pub confidence: f32,
    pub domain_agree: bool,
    pub score: f32,
    pub accepted: bool,
    pub reason: &'static str,
}

/// The gate's decision + full trace.
#[derive(Clone, Debug, Default)]
pub struct InjectionDecision {
    pub accepted: Vec<InjectionCandidate>,
    pub trace: Vec<InjectionScore>,
}

/// Decide which candidates to inject using fused evidence. `candidates` need not
/// be pre-sorted.
pub fn gate(
    mut candidates: Vec<InjectionCandidate>,
    ev: &InjectionEvidence,
    p: &InjectionParams,
) -> InjectionDecision {
    candidates.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let best = candidates.first().map(|c| c.confidence).unwrap_or(0.0);

    let mut decision = InjectionDecision::default();
    for c in candidates {
        if c.confidence < p.abs_floor {
            decision.trace.push(InjectionScore {
                name: c.name,
                confidence: c.confidence,
                domain_agree: false,
                score: c.confidence,
                accepted: false,
                reason: "below_floor",
            });
            continue;
        }
        let domain_agree =
            ev.domain_categories.is_empty() || ev.domain_categories.contains(&c.category);
        let mut score = c.confidence;
        // Domain agreement (positive) / cross-domain (negative) evidence. When the
        // router selected nothing, treat as neutral agreement (no penalty).
        if ev.domain_categories.is_empty() {
            // neutral
        } else if domain_agree {
            score += p.domain_bonus;
        } else {
            score -= p.cross_domain_penalty;
        }
        // Negative evidence: conversation-leaning turn.
        if ev.conversation_only {
            score -= p.conv_penalty;
        }
        // Candidate competition: penalise distance from the best candidate.
        score -= (best - c.confidence) * p.competition_weight;

        // A strong absolute match is accepted regardless (real direct hit).
        let strong = c.confidence >= p.strong;
        let accepted = decision.accepted.len() < p.max && (strong || score >= p.accept);
        let reason = if !accepted {
            if !domain_agree {
                "cross_domain_low_score"
            } else if ev.conversation_only {
                "conversation_suppressed"
            } else {
                "low_fused_score"
            }
        } else if strong {
            "strong_match"
        } else {
            "evidence_accepted"
        };
        if accepted {
            decision.accepted.push(c.clone());
        }
        decision.trace.push(InjectionScore {
            name: c.name,
            confidence: c.confidence,
            domain_agree,
            score,
            accepted,
            reason,
        });
    }
    decision
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(name: &str, cat: &str, conf: f32) -> InjectionCandidate {
        InjectionCandidate {
            name: name.into(),
            category: cat.into(),
            confidence: conf,
        }
    }

    fn ev(domains: &[&str], conv: bool) -> InjectionEvidence {
        InjectionEvidence {
            domain_categories: domains.iter().map(|s| s.to_string()).collect(),
            conversation_only: conv,
        }
    }

    #[test]
    fn keeps_in_domain_drops_cross_domain_noise() {
        // "capital of India": web search is in-domain; marketplace/recall are noise.
        let d = gate(
            vec![
                cand("searxng_search", "internet", 0.52),
                cand("search_marketplace", "marketplace", 0.40),
                cand("recall_fact", "knowledge", 0.38),
            ],
            &ev(&["internet"], false),
            &InjectionParams::default(),
        );
        let names: Vec<_> = d.accepted.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["searxng_search"], "trace: {:?}", d.trace);
    }

    #[test]
    fn strong_match_always_passes_even_cross_domain() {
        // A genuinely strong marketplace hit for "install a calculator skill".
        let d = gate(
            vec![cand("search_marketplace", "marketplace", 0.72)],
            &ev(&["internet"], false),
            &InjectionParams::default(),
        );
        assert_eq!(d.accepted.len(), 1);
    }

    #[test]
    fn conversation_leaning_turn_suppresses_weak_injection() {
        let d = gate(
            vec![cand("recall_fact", "knowledge", 0.44)],
            &ev(&["knowledge"], true),
            &InjectionParams::default(),
        );
        assert!(d.accepted.is_empty(), "trace: {:?}", d.trace);
    }

    #[test]
    fn respects_max_cap() {
        let d = gate(
            vec![
                cand("a", "internet", 0.9),
                cand("b", "internet", 0.85),
                cand("c", "internet", 0.8),
            ],
            &ev(&["internet"], false),
            &InjectionParams::default(),
        );
        assert_eq!(d.accepted.len(), 2);
    }

    #[test]
    fn empty_domain_is_neutral_not_penalised() {
        // No router selection → rely on floor + competition, no cross-domain penalty.
        let d = gate(
            vec![cand("x", "whatever", 0.55)],
            &ev(&[], false),
            &InjectionParams::default(),
        );
        assert_eq!(d.accepted.len(), 1);
    }

    #[test]
    fn below_floor_is_never_injected() {
        let d = gate(
            vec![cand("x", "internet", 0.20)],
            &ev(&["internet"], false),
            &InjectionParams::default(),
        );
        assert!(d.accepted.is_empty());
    }
}
