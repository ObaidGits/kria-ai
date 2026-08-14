//! Deterministic contradiction evaluation and unresolved conflict preservation.
//!
//! # Design invariants (Design §7.1, MGR-037, MGR-001 AC 4)
//!
//! Precedence order (design §7.1):
//! 1. `user_confirmed` — overrides all other factors
//! 2. `verification_recency` — more recent last_verified_at wins
//! 3. `independent_evidence_quality` — count × diversity_score
//! 4. `memory_worth` — only active when observations ≥ 20 (design §6.4)
//!
//! **Unresolved ties preserve BOTH claims** — no side is forced to win.
//! The `Contradicted` truth state is set when conflicting evidence exists
//! and no side wins (MGR-037).

use crate::model::entity::EvidencePolarity;
use crate::model::truth::TruthState;
use crate::model::{GraphRevision, UtcTimestamp};

// ── EvidenceWeight ────────────────────────────────────────────────────────

/// The quality weight of evidence supporting one side of a potential
/// contradiction.
///
/// Precedence order (design §7.1):
/// 1. `user_confirmed` (overrides all other factors)
/// 2. `verification_recency` (more recent verification wins)
/// 3. `independent_evidence_quality` (count × diversity score)
/// 4. `memory_worth` (only used when n ≥ 20; below threshold inert)
#[derive(Debug, Clone, PartialEq)]
pub struct EvidenceWeight {
    /// Whether this side has a user-confirmed truth state.
    pub user_confirmed: bool,
    /// When this side was last verified (`None` = never/unknown).
    pub last_verified_at: Option<UtcTimestamp>,
    /// Count of independent evidence items supporting this side.
    pub independent_evidence_count: u32,
    /// Normalized diversity score [0.0, 1.0] of evidence sources.
    /// Used when `independent_evidence_count > 0`.
    pub source_diversity_score: f64,
    /// Memory Worth (`0.0` if n < 20; only meaningful when n ≥ 20).
    /// Inert below 20 samples per design §6.4.
    pub memory_worth: Option<f64>,
    /// Number of observations used for Memory Worth calculation.
    /// `None` or `Some(n)` where n < 20 means Memory Worth is inert.
    pub memory_worth_observations: Option<u32>,
}

impl EvidenceWeight {
    /// Compute the independent evidence quality score: count × diversity_score.
    pub fn evidence_quality_score(&self) -> f64 {
        self.independent_evidence_count as f64 * self.source_diversity_score
    }

    /// Whether Memory Worth is active (observations ≥ 20 AND value is Some).
    pub fn memory_worth_is_active(&self) -> bool {
        match (self.memory_worth, self.memory_worth_observations) {
            (Some(_), Some(n)) => n >= 20,
            _ => false,
        }
    }
}

// ── ConflictSide ──────────────────────────────────────────────────────────

/// One side of a contradiction — a record or relationship with its supporting
/// evidence weight.
#[derive(Debug, Clone, PartialEq)]
pub struct ConflictSide {
    /// The record/relationship ID for this side.
    pub record_id: String,
    /// The claim value (policy-safe text for display; `None` = omitted/unavailable).
    pub claim_summary: Option<String>,
    /// Polarity of the evidence for this side (supports/contradicts this claim).
    pub evidence_polarity: EvidencePolarity,
    /// Evidence weight for precedence evaluation.
    pub weight: EvidenceWeight,
    /// Truth state of this side.
    pub truth_state: TruthState,
    /// Revision at which this side was recorded.
    pub revision: GraphRevision,
}

// ── PrecedenceFactor ──────────────────────────────────────────────────────

/// Which factor in the precedence chain decided the outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrecedenceFactor {
    /// One side is user-confirmed and the other is not.
    UserConfirmed,
    /// More recent `last_verified_at` decided the winner.
    VerificationRecency,
    /// Higher `count × diversity_score` decided the winner.
    IndependentEvidenceQuality,
    /// Higher Memory Worth (when both have n ≥ 20) decided the winner.
    MemoryWorth,
}

impl PrecedenceFactor {
    /// Human-readable name for this factor.
    pub fn display_name(&self) -> &'static str {
        match self {
            PrecedenceFactor::UserConfirmed => "User-Confirmed",
            PrecedenceFactor::VerificationRecency => "Verification Recency",
            PrecedenceFactor::IndependentEvidenceQuality => "Independent Evidence Quality",
            PrecedenceFactor::MemoryWorth => "Memory Worth",
        }
    }
}

// ── UnresolvedReason ──────────────────────────────────────────────────────

/// Why no side won when `ContradictionResolution::Unresolved` is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnresolvedReason {
    /// Neither side has any evidence advantage; all factors tie.
    EqualWeight,
    /// Memory Worth is below threshold (n < 20) for all sides and all
    /// earlier factors also tied.
    InsufficientMemoryWorthData,
    /// Three or more sides exist (n-way conflict); multi-side comparison is
    /// not supported.
    MultipleConflicts,
    /// Evidence cannot be compared (different scopes, policies, or
    /// unavailable data).
    IncomparableEvidence,
}

impl UnresolvedReason {
    /// Human-readable explanation for this reason.
    pub fn description(&self) -> &'static str {
        match self {
            UnresolvedReason::EqualWeight =>
                "All precedence factors are equal; neither side has an advantage.",
            UnresolvedReason::InsufficientMemoryWorthData =>
                "Memory Worth is below the 20-observation threshold for all sides; earlier factors also tied.",
            UnresolvedReason::MultipleConflicts =>
                "Three or more conflicting sides exist; multi-way conflicts are not auto-resolved.",
            UnresolvedReason::IncomparableEvidence =>
                "Evidence cannot be compared due to different scopes, policies, or unavailable data.",
        }
    }
}

// ── ContradictionResolution ───────────────────────────────────────────────

/// The outcome of evaluating two or more conflicting sides.
#[derive(Debug, Clone, PartialEq)]
pub enum ContradictionResolution {
    /// One side wins by precedence; the other becomes Superseded.
    Resolved {
        /// The winning side's `record_id`.
        winner_id: String,
        /// The losing side's `record_id` (should be superseded).
        loser_id: String,
        /// Which precedence factor decided the outcome.
        deciding_factor: PrecedenceFactor,
    },
    /// No side wins; both are preserved as Contradicted.
    Unresolved {
        /// The IDs of all conflicting sides.
        conflict_sides: Vec<String>,
        /// Why no side won.
        unresolved_reason: UnresolvedReason,
    },
    /// One side is clearly false/retracted and should be marked Superseded.
    ClearlySuperseded {
        /// The active (winning) side's `record_id`.
        active_id: String,
        /// The superseded (losing) side's `record_id`.
        superseded_id: String,
        /// Human-readable reason for supersession.
        reason: String,
    },
}

// ── ContradictionExplanation ──────────────────────────────────────────────

/// A result for one factor checked during precedence evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct PrecedenceFactorResult {
    /// Which factor was checked.
    pub factor: PrecedenceFactor,
    /// Whether this factor decided the outcome.
    pub decisive: bool,
    /// Text description of this factor's result.
    pub description: String,
}

/// Human-readable (policy-safe) explanation of why a contradiction is
/// unresolved or how it was resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct ContradictionExplanation {
    /// Summary of the conflict.
    pub summary: String,
    /// The precedence factors checked and their outcome.
    pub factors_checked: Vec<PrecedenceFactorResult>,
    /// Recommended user action (if any).
    pub recommended_action: Option<String>,
}

// ── ContradictionEvaluator ────────────────────────────────────────────────

/// Stateless, deterministic evaluator for contradiction precedence (design §7.1).
///
/// Given identical inputs, always returns the same output — no randomness.
pub struct ContradictionEvaluator;

impl ContradictionEvaluator {
    /// Deterministically evaluate two conflicting sides and return a resolution.
    ///
    /// Rules (design §7.1 precedence):
    /// 1. If exactly one side is `user_confirmed` → that side wins (`UserConfirmed`).
    /// 2. If both or neither `user_confirmed` → compare `last_verified_at`:
    ///    if one is strictly more recent → it wins (`VerificationRecency`).
    /// 3. If still tied → compare `count × diversity_score`:
    ///    if one is strictly greater → it wins (`IndependentEvidenceQuality`).
    /// 4. If still tied → check Memory Worth:
    ///    both must have `observations ≥ 20`; if one is strictly greater → it wins (`MemoryWorth`).
    ///    If one or both lack sufficient observations → `InsufficientMemoryWorthData`.
    /// 5. If still tied → `Unresolved(EqualWeight)`.
    pub fn evaluate(left: &ConflictSide, right: &ConflictSide) -> ContradictionResolution {
        // ── 1. User-confirmed ────────────────────────────────────────────
        match (left.weight.user_confirmed, right.weight.user_confirmed) {
            (true, false) => {
                return ContradictionResolution::Resolved {
                    winner_id: left.record_id.clone(),
                    loser_id: right.record_id.clone(),
                    deciding_factor: PrecedenceFactor::UserConfirmed,
                };
            }
            (false, true) => {
                return ContradictionResolution::Resolved {
                    winner_id: right.record_id.clone(),
                    loser_id: left.record_id.clone(),
                    deciding_factor: PrecedenceFactor::UserConfirmed,
                };
            }
            // Both confirmed or neither confirmed: fall through to next factor.
            _ => {}
        }

        // ── 2. Verification recency ──────────────────────────────────────
        match (left.weight.last_verified_at, right.weight.last_verified_at) {
            (Some(lv), Some(rv)) => {
                if lv > rv {
                    return ContradictionResolution::Resolved {
                        winner_id: left.record_id.clone(),
                        loser_id: right.record_id.clone(),
                        deciding_factor: PrecedenceFactor::VerificationRecency,
                    };
                } else if rv > lv {
                    return ContradictionResolution::Resolved {
                        winner_id: right.record_id.clone(),
                        loser_id: left.record_id.clone(),
                        deciding_factor: PrecedenceFactor::VerificationRecency,
                    };
                }
                // Equal timestamps: fall through.
            }
            (Some(_), None) => {
                // Left is verified, right is not → left wins.
                return ContradictionResolution::Resolved {
                    winner_id: left.record_id.clone(),
                    loser_id: right.record_id.clone(),
                    deciding_factor: PrecedenceFactor::VerificationRecency,
                };
            }
            (None, Some(_)) => {
                // Right is verified, left is not → right wins.
                return ContradictionResolution::Resolved {
                    winner_id: right.record_id.clone(),
                    loser_id: left.record_id.clone(),
                    deciding_factor: PrecedenceFactor::VerificationRecency,
                };
            }
            (None, None) => {
                // Neither verified: fall through.
            }
        }

        // ── 3. Independent evidence quality ─────────────────────────────
        let lq = left.weight.evidence_quality_score();
        let rq = right.weight.evidence_quality_score();
        if lq > rq {
            return ContradictionResolution::Resolved {
                winner_id: left.record_id.clone(),
                loser_id: right.record_id.clone(),
                deciding_factor: PrecedenceFactor::IndependentEvidenceQuality,
            };
        } else if rq > lq {
            return ContradictionResolution::Resolved {
                winner_id: right.record_id.clone(),
                loser_id: left.record_id.clone(),
                deciding_factor: PrecedenceFactor::IndependentEvidenceQuality,
            };
        }

        // ── 4. Memory Worth ──────────────────────────────────────────────
        let l_active = left.weight.memory_worth_is_active();
        let r_active = right.weight.memory_worth_is_active();

        if l_active && r_active {
            // Both have sufficient data — compare values.
            let lw = left.weight.memory_worth.unwrap_or(0.0);
            let rw = right.weight.memory_worth.unwrap_or(0.0);
            if lw > rw {
                return ContradictionResolution::Resolved {
                    winner_id: left.record_id.clone(),
                    loser_id: right.record_id.clone(),
                    deciding_factor: PrecedenceFactor::MemoryWorth,
                };
            } else if rw > lw {
                return ContradictionResolution::Resolved {
                    winner_id: right.record_id.clone(),
                    loser_id: left.record_id.clone(),
                    deciding_factor: PrecedenceFactor::MemoryWorth,
                };
            }
            // Equal memory worth: EqualWeight.
            return ContradictionResolution::Unresolved {
                conflict_sides: vec![left.record_id.clone(), right.record_id.clone()],
                unresolved_reason: UnresolvedReason::EqualWeight,
            };
        } else {
            // At least one side lacks sufficient observations → InsufficientMemoryWorthData.
            return ContradictionResolution::Unresolved {
                conflict_sides: vec![left.record_id.clone(), right.record_id.clone()],
                unresolved_reason: UnresolvedReason::InsufficientMemoryWorthData,
            };
        }
    }

    /// Evaluate n-way conflicts (more than 2 sides).
    ///
    /// When `sides.len() > 2`: always `Unresolved(MultipleConflicts)` with all
    /// side IDs.
    /// When `sides.len() == 2`: delegates to [`Self::evaluate`].
    /// When `sides.len() == 0` or `1`: `Unresolved(IncomparableEvidence)`.
    pub fn evaluate_multi(sides: &[ConflictSide]) -> ContradictionResolution {
        match sides.len() {
            0 | 1 => ContradictionResolution::Unresolved {
                conflict_sides: sides.iter().map(|s| s.record_id.clone()).collect(),
                unresolved_reason: UnresolvedReason::IncomparableEvidence,
            },
            2 => Self::evaluate(&sides[0], &sides[1]),
            _ => ContradictionResolution::Unresolved {
                conflict_sides: sides.iter().map(|s| s.record_id.clone()).collect(),
                unresolved_reason: UnresolvedReason::MultipleConflicts,
            },
        }
    }

    /// Produce a `ContradictionExplanation` describing why a conflict is
    /// unresolved or how it was resolved, listing all checked factors.
    pub fn explain(left: &ConflictSide, right: &ConflictSide) -> ContradictionExplanation {
        let mut factors: Vec<PrecedenceFactorResult> = Vec::new();

        // ── Factor 1: User-confirmed ─────────────────────────────────────
        let uc_decisive = left.weight.user_confirmed != right.weight.user_confirmed;
        factors.push(PrecedenceFactorResult {
            factor: PrecedenceFactor::UserConfirmed,
            decisive: uc_decisive,
            description: match (left.weight.user_confirmed, right.weight.user_confirmed) {
                (true, true) => format!(
                    "Both '{}' and '{}' are user-confirmed; factor does not decide.",
                    left.record_id, right.record_id
                ),
                (false, false) => format!(
                    "Neither '{}' nor '{}' is user-confirmed; factor does not decide.",
                    left.record_id, right.record_id
                ),
                (true, false) => format!(
                    "'{}' is user-confirmed; '{}' is not. '{}' wins this factor.",
                    left.record_id, right.record_id, left.record_id
                ),
                (false, true) => format!(
                    "'{}' is user-confirmed; '{}' is not. '{}' wins this factor.",
                    right.record_id, left.record_id, right.record_id
                ),
            },
        });

        if uc_decisive {
            let winner = if left.weight.user_confirmed {
                &left.record_id
            } else {
                &right.record_id
            };
            return ContradictionExplanation {
                summary: format!(
                    "Contradiction between '{}' and '{}' resolved by user-confirmed status: '{}' wins.",
                    left.record_id, right.record_id, winner
                ),
                factors_checked: factors,
                recommended_action: None,
            };
        }

        // ── Factor 2: Verification recency ───────────────────────────────
        let vr_decisive = match (left.weight.last_verified_at, right.weight.last_verified_at) {
            (Some(lv), Some(rv)) => lv != rv,
            (Some(_), None) | (None, Some(_)) => true,
            (None, None) => false,
        };
        let vr_desc = match (left.weight.last_verified_at, right.weight.last_verified_at) {
            (Some(lv), Some(rv)) if lv > rv => format!(
                "'{}' was more recently verified ({} vs {}). '{}' wins.",
                left.record_id, lv, rv, left.record_id
            ),
            (Some(lv), Some(rv)) if rv > lv => format!(
                "'{}' was more recently verified ({} vs {}). '{}' wins.",
                right.record_id, rv, lv, right.record_id
            ),
            (Some(lv), Some(_rv)) => format!(
                "Both verified at the same time ({}). Factor does not decide.",
                lv
            ),
            (Some(lv), None) => format!(
                "'{}' is verified ({}); '{}' has never been verified. '{}' wins.",
                left.record_id, lv, right.record_id, left.record_id
            ),
            (None, Some(rv)) => format!(
                "'{}' is verified ({}); '{}' has never been verified. '{}' wins.",
                right.record_id, rv, left.record_id, right.record_id
            ),
            (None, None) => format!(
                "Neither '{}' nor '{}' has been verified. Factor does not decide.",
                left.record_id, right.record_id
            ),
        };
        factors.push(PrecedenceFactorResult {
            factor: PrecedenceFactor::VerificationRecency,
            decisive: vr_decisive,
            description: vr_desc,
        });
        if vr_decisive {
            let winner = match (left.weight.last_verified_at, right.weight.last_verified_at) {
                (Some(lv), Some(rv)) if lv > rv => &left.record_id,
                (Some(_), None) => &left.record_id,
                _ => &right.record_id,
            };
            return ContradictionExplanation {
                summary: format!(
                    "Contradiction between '{}' and '{}' resolved by verification recency: '{}' wins.",
                    left.record_id, right.record_id, winner
                ),
                factors_checked: factors,
                recommended_action: None,
            };
        }

        // ── Factor 3: Independent evidence quality ───────────────────────
        let lq = left.weight.evidence_quality_score();
        let rq = right.weight.evidence_quality_score();
        let eq_decisive = (lq - rq).abs() > f64::EPSILON;
        let eq_desc = if lq > rq {
            format!(
                "'{}' has higher evidence quality ({:.3} vs {:.3}). '{}' wins.",
                left.record_id, lq, rq, left.record_id
            )
        } else if rq > lq {
            format!(
                "'{}' has higher evidence quality ({:.3} vs {:.3}). '{}' wins.",
                right.record_id, rq, lq, right.record_id
            )
        } else {
            format!(
                "Both sides have equal evidence quality ({:.3}). Factor does not decide.",
                lq
            )
        };
        factors.push(PrecedenceFactorResult {
            factor: PrecedenceFactor::IndependentEvidenceQuality,
            decisive: eq_decisive,
            description: eq_desc,
        });
        if eq_decisive {
            let winner = if lq > rq {
                &left.record_id
            } else {
                &right.record_id
            };
            return ContradictionExplanation {
                summary: format!(
                    "Contradiction between '{}' and '{}' resolved by evidence quality: '{}' wins.",
                    left.record_id, right.record_id, winner
                ),
                factors_checked: factors,
                recommended_action: None,
            };
        }

        // ── Factor 4: Memory Worth ───────────────────────────────────────
        let l_active = left.weight.memory_worth_is_active();
        let r_active = right.weight.memory_worth_is_active();
        if l_active && r_active {
            let lw = left.weight.memory_worth.unwrap_or(0.0);
            let rw = right.weight.memory_worth.unwrap_or(0.0);
            let mw_decisive = (lw - rw).abs() > f64::EPSILON;
            let mw_desc = if lw > rw {
                format!(
                    "'{}' has higher Memory Worth ({:.4} vs {:.4}). '{}' wins.",
                    left.record_id, lw, rw, left.record_id
                )
            } else if rw > lw {
                format!(
                    "'{}' has higher Memory Worth ({:.4} vs {:.4}). '{}' wins.",
                    right.record_id, rw, lw, right.record_id
                )
            } else {
                format!(
                    "Both sides have equal Memory Worth ({:.4}). Factor does not decide.",
                    lw
                )
            };
            factors.push(PrecedenceFactorResult {
                factor: PrecedenceFactor::MemoryWorth,
                decisive: mw_decisive,
                description: mw_desc,
            });
            if mw_decisive {
                let winner = if lw > rw {
                    &left.record_id
                } else {
                    &right.record_id
                };
                return ContradictionExplanation {
                    summary: format!(
                        "Contradiction between '{}' and '{}' resolved by Memory Worth: '{}' wins.",
                        left.record_id, right.record_id, winner
                    ),
                    factors_checked: factors,
                    recommended_action: None,
                };
            }
            // Memory worth equal → EqualWeight.
            ContradictionExplanation {
                summary: format!(
                    "Contradiction between '{}' and '{}' is unresolved: all precedence factors are equal.",
                    left.record_id, right.record_id
                ),
                factors_checked: factors,
                recommended_action: Some(
                    "Manual review required. Confirm one side as the authoritative truth.".to_string()
                ),
            }
        } else {
            let obs_desc = |w: &EvidenceWeight, id: &str| -> String {
                match w.memory_worth_observations {
                    None => format!("'{}' has no Memory Worth observations.", id),
                    Some(n) if n < 20 => {
                        format!("'{}' has only {} observations (need ≥ 20).", id, n)
                    }
                    Some(n) => format!("'{}' has {} observations.", id, n),
                }
            };
            factors.push(PrecedenceFactorResult {
                factor: PrecedenceFactor::MemoryWorth,
                decisive: false,
                description: format!(
                    "Memory Worth is inert: {} {}",
                    obs_desc(&left.weight, &left.record_id),
                    obs_desc(&right.weight, &right.record_id),
                ),
            });
            ContradictionExplanation {
                summary: format!(
                    "Contradiction between '{}' and '{}' is unresolved: insufficient Memory Worth data.",
                    left.record_id, right.record_id
                ),
                factors_checked: factors,
                recommended_action: Some(
                    "Accumulate more observations (≥ 20) or manually confirm one side.".to_string()
                ),
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> UtcTimestamp {
        UtcTimestamp::from_datetime(
            chrono::Utc
                .timestamp_opt(secs, 0)
                .single()
                .expect("valid timestamp"),
        )
    }

    fn rev(n: u64) -> GraphRevision {
        GraphRevision::new(n)
    }

    /// Build a ConflictSide with reasonable defaults.
    fn side(
        id: &str,
        user_confirmed: bool,
        last_verified_at: Option<UtcTimestamp>,
        evidence_count: u32,
        diversity_score: f64,
        memory_worth: Option<f64>,
        memory_worth_obs: Option<u32>,
    ) -> ConflictSide {
        ConflictSide {
            record_id: id.to_string(),
            claim_summary: Some(format!("claim by {}", id)),
            evidence_polarity: EvidencePolarity::Supports,
            weight: EvidenceWeight {
                user_confirmed,
                last_verified_at,
                independent_evidence_count: evidence_count,
                source_diversity_score: diversity_score,
                memory_worth,
                memory_worth_observations: memory_worth_obs,
            },
            truth_state: TruthState::Current,
            revision: rev(1),
        }
    }

    // ── 1. user_confirmed one side → Resolved(UserConfirmed) ─────────────

    #[test]
    fn user_confirmed_left_wins() {
        let l = side("left", true, None, 0, 0.0, None, None);
        let r = side("right", false, None, 10, 1.0, Some(0.9), Some(50));
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Resolved {
                winner_id: "left".to_string(),
                loser_id: "right".to_string(),
                deciding_factor: PrecedenceFactor::UserConfirmed,
            }
        );
    }

    #[test]
    fn user_confirmed_right_wins() {
        let l = side("left", false, None, 10, 1.0, Some(0.9), Some(50));
        let r = side("right", true, None, 0, 0.0, None, None);
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Resolved {
                winner_id: "right".to_string(),
                loser_id: "left".to_string(),
                deciding_factor: PrecedenceFactor::UserConfirmed,
            }
        );
    }

    // ── 2. both user_confirmed → proceeds to VerificationRecency ─────────

    #[test]
    fn both_confirmed_falls_through_to_verification_recency() {
        let l = side("left", true, Some(ts(2_000)), 0, 0.0, None, None);
        let r = side("right", true, Some(ts(1_000)), 0, 0.0, None, None);
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Resolved {
                winner_id: "left".to_string(),
                loser_id: "right".to_string(),
                deciding_factor: PrecedenceFactor::VerificationRecency,
            }
        );
    }

    // ── 3. VerificationRecency: more recent wins ──────────────────────────

    #[test]
    fn verification_recency_left_more_recent() {
        let l = side("left", false, Some(ts(2_000)), 0, 0.0, None, None);
        let r = side("right", false, Some(ts(1_000)), 0, 0.0, None, None);
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Resolved {
                winner_id: "left".to_string(),
                loser_id: "right".to_string(),
                deciding_factor: PrecedenceFactor::VerificationRecency,
            }
        );
    }

    #[test]
    fn verification_recency_right_more_recent() {
        let l = side("left", false, Some(ts(1_000)), 0, 0.0, None, None);
        let r = side("right", false, Some(ts(2_000)), 0, 0.0, None, None);
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Resolved {
                winner_id: "right".to_string(),
                loser_id: "left".to_string(),
                deciding_factor: PrecedenceFactor::VerificationRecency,
            }
        );
    }

    #[test]
    fn verification_recency_left_verified_right_none() {
        let l = side("left", false, Some(ts(1_000)), 0, 0.0, None, None);
        let r = side("right", false, None, 0, 0.0, None, None);
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Resolved {
                winner_id: "left".to_string(),
                loser_id: "right".to_string(),
                deciding_factor: PrecedenceFactor::VerificationRecency,
            }
        );
    }

    // ── 4. VerificationRecency: both None → tie, proceeds ────────────────

    #[test]
    fn both_unverified_proceeds_to_evidence_quality() {
        // Both unverified, left has better evidence quality
        let l = side("left", false, None, 5, 0.8, None, None);
        let r = side("right", false, None, 2, 0.5, None, None);
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Resolved {
                winner_id: "left".to_string(),
                loser_id: "right".to_string(),
                deciding_factor: PrecedenceFactor::IndependentEvidenceQuality,
            }
        );
    }

    // ── 5. IndependentEvidenceQuality: higher score wins ─────────────────

    #[test]
    fn evidence_quality_left_wins() {
        let l = side("left", false, None, 10, 0.9, None, None); // score = 9.0
        let r = side("right", false, None, 5, 0.8, None, None); // score = 4.0
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Resolved {
                winner_id: "left".to_string(),
                loser_id: "right".to_string(),
                deciding_factor: PrecedenceFactor::IndependentEvidenceQuality,
            }
        );
    }

    #[test]
    fn evidence_quality_right_wins() {
        let l = side("left", false, None, 3, 0.5, None, None); // score = 1.5
        let r = side("right", false, None, 8, 0.8, None, None); // score = 6.4
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Resolved {
                winner_id: "right".to_string(),
                loser_id: "left".to_string(),
                deciding_factor: PrecedenceFactor::IndependentEvidenceQuality,
            }
        );
    }

    // ── 6. IndependentEvidenceQuality: equal → tie ────────────────────────

    #[test]
    fn evidence_quality_equal_proceeds_to_memory_worth() {
        // Equal quality, both have enough memory worth observations
        let l = side("left", false, None, 4, 0.5, Some(0.8), Some(25)); // score = 2.0
        let r = side("right", false, None, 4, 0.5, Some(0.6), Some(30)); // score = 2.0
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Resolved {
                winner_id: "left".to_string(),
                loser_id: "right".to_string(),
                deciding_factor: PrecedenceFactor::MemoryWorth,
            }
        );
    }

    // ── 7. MemoryWorth: one side ≥ 20 obs, higher worth wins ─────────────

    #[test]
    fn memory_worth_left_wins() {
        let l = side("left", false, None, 0, 0.0, Some(0.9), Some(25));
        let r = side("right", false, None, 0, 0.0, Some(0.5), Some(20));
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Resolved {
                winner_id: "left".to_string(),
                loser_id: "right".to_string(),
                deciding_factor: PrecedenceFactor::MemoryWorth,
            }
        );
    }

    #[test]
    fn memory_worth_right_wins() {
        let l = side("left", false, None, 0, 0.0, Some(0.3), Some(20));
        let r = side("right", false, None, 0, 0.0, Some(0.7), Some(30));
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Resolved {
                winner_id: "right".to_string(),
                loser_id: "left".to_string(),
                deciding_factor: PrecedenceFactor::MemoryWorth,
            }
        );
    }

    // ── 8. MemoryWorth: one side < 20 obs → InsufficientMemoryWorthData ───

    #[test]
    fn memory_worth_left_insufficient_obs() {
        let l = side("left", false, None, 0, 0.0, Some(0.9), Some(19)); // 19 < 20
        let r = side("right", false, None, 0, 0.0, Some(0.5), Some(20));
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Unresolved {
                conflict_sides: vec!["left".to_string(), "right".to_string()],
                unresolved_reason: UnresolvedReason::InsufficientMemoryWorthData,
            }
        );
    }

    #[test]
    fn memory_worth_right_insufficient_obs() {
        let l = side("left", false, None, 0, 0.0, Some(0.9), Some(20));
        let r = side("right", false, None, 0, 0.0, Some(0.5), Some(15)); // 15 < 20
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Unresolved {
                conflict_sides: vec!["left".to_string(), "right".to_string()],
                unresolved_reason: UnresolvedReason::InsufficientMemoryWorthData,
            }
        );
    }

    #[test]
    fn memory_worth_both_none_obs_insufficient() {
        let l = side("left", false, None, 0, 0.0, None, None);
        let r = side("right", false, None, 0, 0.0, None, None);
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Unresolved {
                conflict_sides: vec!["left".to_string(), "right".to_string()],
                unresolved_reason: UnresolvedReason::InsufficientMemoryWorthData,
            }
        );
    }

    // ── 9. Both tied → Unresolved(EqualWeight) ───────────────────────────

    #[test]
    fn all_factors_tied_equal_weight() {
        let l = side("left", false, None, 0, 0.0, Some(0.5), Some(25));
        let r = side("right", false, None, 0, 0.0, Some(0.5), Some(25));
        let res = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(
            res,
            ContradictionResolution::Unresolved {
                conflict_sides: vec!["left".to_string(), "right".to_string()],
                unresolved_reason: UnresolvedReason::EqualWeight,
            }
        );
    }

    // ── 10. Multi-side → Unresolved(MultipleConflicts) ───────────────────

    #[test]
    fn evaluate_multi_three_sides() {
        let sides = vec![
            side("a", false, None, 0, 0.0, None, None),
            side("b", false, None, 0, 0.0, None, None),
            side("c", false, None, 0, 0.0, None, None),
        ];
        let res = ContradictionEvaluator::evaluate_multi(&sides);
        assert_eq!(
            res,
            ContradictionResolution::Unresolved {
                conflict_sides: vec!["a".to_string(), "b".to_string(), "c".to_string()],
                unresolved_reason: UnresolvedReason::MultipleConflicts,
            }
        );
    }

    #[test]
    fn evaluate_multi_four_sides() {
        let sides = vec![
            side("a", true, None, 0, 0.0, None, None), // would win in 2-way
            side("b", false, None, 0, 0.0, None, None),
            side("c", false, None, 0, 0.0, None, None),
            side("d", false, None, 0, 0.0, None, None),
        ];
        let res = ContradictionEvaluator::evaluate_multi(&sides);
        matches!(
            res,
            ContradictionResolution::Unresolved {
                unresolved_reason: UnresolvedReason::MultipleConflicts,
                ..
            }
        );
    }

    #[test]
    fn evaluate_multi_two_sides_delegates_to_evaluate() {
        let sides = vec![
            side("left", true, None, 0, 0.0, None, None),
            side("right", false, None, 0, 0.0, None, None),
        ];
        let res = ContradictionEvaluator::evaluate_multi(&sides);
        assert_eq!(
            res,
            ContradictionResolution::Resolved {
                winner_id: "left".to_string(),
                loser_id: "right".to_string(),
                deciding_factor: PrecedenceFactor::UserConfirmed,
            }
        );
    }

    #[test]
    fn evaluate_multi_zero_sides_incomparable() {
        let res = ContradictionEvaluator::evaluate_multi(&[]);
        assert_eq!(
            res,
            ContradictionResolution::Unresolved {
                conflict_sides: vec![],
                unresolved_reason: UnresolvedReason::IncomparableEvidence,
            }
        );
    }

    // ── 11. explain() returns PrecedenceFactorResult list ─────────────────

    #[test]
    fn explain_user_confirmed_decisive_has_one_factor() {
        let l = side("left", true, None, 0, 0.0, None, None);
        let r = side("right", false, None, 0, 0.0, None, None);
        let expl = ContradictionEvaluator::explain(&l, &r);
        assert_eq!(expl.factors_checked.len(), 1);
        assert_eq!(
            expl.factors_checked[0].factor,
            PrecedenceFactor::UserConfirmed
        );
        assert!(expl.factors_checked[0].decisive);
        assert!(expl.summary.contains("user-confirmed"));
        assert!(expl.recommended_action.is_none());
    }

    #[test]
    fn explain_unresolved_has_all_four_factors() {
        let l = side("left", false, None, 0, 0.0, Some(0.5), Some(25));
        let r = side("right", false, None, 0, 0.0, Some(0.5), Some(25));
        let expl = ContradictionEvaluator::explain(&l, &r);
        assert_eq!(
            expl.factors_checked.len(),
            4,
            "must check all 4 factors when all tie"
        );
        // None should be decisive
        assert!(expl.factors_checked.iter().all(|f| !f.decisive));
        assert!(expl.recommended_action.is_some());
    }

    #[test]
    fn explain_insufficient_memory_worth_has_four_factors() {
        let l = side("left", false, None, 0, 0.0, None, None);
        let r = side("right", false, None, 0, 0.0, None, None);
        let expl = ContradictionEvaluator::explain(&l, &r);
        assert_eq!(expl.factors_checked.len(), 4);
        let mw = expl
            .factors_checked
            .iter()
            .find(|f| f.factor == PrecedenceFactor::MemoryWorth);
        assert!(mw.is_some());
        assert!(!mw.unwrap().decisive);
        assert!(expl.recommended_action.is_some());
    }

    #[test]
    fn explain_verification_recency_decisive_has_two_factors() {
        let l = side("left", false, Some(ts(2_000)), 0, 0.0, None, None);
        let r = side("right", false, Some(ts(1_000)), 0, 0.0, None, None);
        let expl = ContradictionEvaluator::explain(&l, &r);
        assert_eq!(expl.factors_checked.len(), 2);
        assert!(!expl.factors_checked[0].decisive); // UserConfirmed: not decisive
        assert!(expl.factors_checked[1].decisive); // VerificationRecency: decisive
        assert!(
            expl.summary.contains("recency")
                || expl.summary.contains("Recency")
                || expl.summary.contains("verification")
        );
    }

    // ── 12. Determinism: same inputs always produce same output ───────────

    #[test]
    fn evaluation_is_deterministic() {
        let l = side("left", false, Some(ts(1_500)), 5, 0.7, Some(0.6), Some(22));
        let r = side("right", false, Some(ts(1_200)), 3, 0.8, Some(0.8), Some(30));
        let r1 = ContradictionEvaluator::evaluate(&l, &r);
        let r2 = ContradictionEvaluator::evaluate(&l, &r);
        assert_eq!(r1, r2, "evaluation must be deterministic");
    }

    // ── 13. EvidenceWeight helpers ────────────────────────────────────────

    #[test]
    fn evidence_quality_score_computes_correctly() {
        let w = EvidenceWeight {
            user_confirmed: false,
            last_verified_at: None,
            independent_evidence_count: 5,
            source_diversity_score: 0.8,
            memory_worth: None,
            memory_worth_observations: None,
        };
        let score = w.evidence_quality_score();
        assert!(
            (score - 4.0).abs() < f64::EPSILON,
            "5 * 0.8 = 4.0, got {score}"
        );
    }

    #[test]
    fn memory_worth_is_active_requires_20_obs() {
        let w_inactive = EvidenceWeight {
            user_confirmed: false,
            last_verified_at: None,
            independent_evidence_count: 0,
            source_diversity_score: 0.0,
            memory_worth: Some(0.9),
            memory_worth_observations: Some(19),
        };
        assert!(!w_inactive.memory_worth_is_active(), "19 obs < 20: inert");

        let w_active = EvidenceWeight {
            memory_worth_observations: Some(20),
            ..w_inactive
        };
        assert!(w_active.memory_worth_is_active(), "20 obs: active");

        let w_none = EvidenceWeight {
            memory_worth: None,
            memory_worth_observations: Some(100),
            ..w_active
        };
        assert!(!w_none.memory_worth_is_active(), "no value: inert");
    }

    // ── 14. ClearlySuperseded path (direct struct construction check) ─────

    #[test]
    fn clearly_superseded_is_constructible() {
        let res = ContradictionResolution::ClearlySuperseded {
            active_id: "active".to_string(),
            superseded_id: "old".to_string(),
            reason: "Explicitly retracted by user.".to_string(),
        };
        match res {
            ContradictionResolution::ClearlySuperseded {
                active_id,
                superseded_id,
                ..
            } => {
                assert_eq!(active_id, "active");
                assert_eq!(superseded_id, "old");
            }
            _ => panic!("expected ClearlySuperseded"),
        }
    }
}
