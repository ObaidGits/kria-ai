//! Weighted RRF fusion engine (design §6.3, task F3.4.3).
//!
//! Fuses one-based ranked candidate lists from multiple retrieval strategies
//! using weighted Reciprocal Rank Fusion. Missing strategies contribute zero
//! score (never redistributed). Deduplicates by semantic_id+content_version
//! before fusion. Breaks score ties by semantic_id ASC.
//!
//! # Design invariants
//! * Ranks are one-based (rank 1 = best candidate from that strategy).
//! * availability ∈ {0, 1}: unavailable strategy contributes exactly 0.
//! * No missing-weight redistribution to other strategies.
//! * Stable semantic-ID dedup: only the best (lowest) rank per strategy per ID.
//! * Tie-break: score DESC, then semantic_id ASC (deterministic).

use std::collections::HashMap;

use crate::retrieval::rrf_profile::FusionProfile;

// ── Input types ───────────────────────────────────────────────────────────────

/// One candidate from a single strategy's ranked result list.
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyCandidate {
    /// Stable semantic identifier (UUID string).
    pub semantic_id: String,
    /// Content version hash for deduplication (may be empty for non-versioned candidates).
    pub content_version: String,
    /// One-based rank from this strategy (1 = best). Must be ≥ 1.
    pub rank: u32,
}

/// Availability of one strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyAvailability {
    /// Strategy is available; its candidates contribute to fusion.
    Available,
    /// Strategy is unavailable (offline, degraded, or cancelled).
    /// Contributes exactly 0 to the fused score — weight is NOT redistributed.
    Unavailable,
}

/// Input to the RRF fusion engine for one strategy.
#[derive(Debug, Clone)]
pub struct StrategyInput {
    /// Which strategy this is.
    pub strategy: StrategyKind,
    /// Whether this strategy is available.
    pub availability: StrategyAvailability,
    /// Ranked candidates from this strategy (may be empty when unavailable).
    pub candidates: Vec<StrategyCandidate>,
}

/// The five retrieval strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StrategyKind {
    Fts,
    Vector,
    Graph,
    Temporal,
    Goal,
}

/// One fused result after weighted RRF.
#[derive(Debug, Clone, PartialEq)]
pub struct FusedCandidate {
    /// Stable semantic identifier.
    pub semantic_id: String,
    /// Content version (from the first strategy that contributed this candidate).
    pub content_version: String,
    /// Fused RRF score (sum of per-strategy contributions).
    pub rrf_score: f32,
    /// Per-strategy contribution to the fused score (zero for unavailable/absent strategies).
    pub contributions: StrategyContributions,
}

/// Per-strategy breakdown of a candidate's RRF score contributions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrategyContributions {
    pub fts: f32,
    pub vector: f32,
    pub graph: f32,
    pub temporal: f32,
    pub goal: f32,
}

impl StrategyContributions {
    /// Sum of all contributions (equals rrf_score for valid fusion).
    pub fn total(&self) -> f32 {
        self.fts + self.vector + self.graph + self.temporal + self.goal
    }
}

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum RrfError {
    /// A candidate had rank=0 (ranks must be one-based).
    ZeroRank {
        strategy: StrategyKind,
        semantic_id: String,
    },
}

// ── Fusion engine ─────────────────────────────────────────────────────────────

/// Fuse ranked candidates from multiple strategies using weighted RRF.
///
/// # Contract
/// * Ranks must be one-based (≥ 1); zero-rank candidates are rejected with an error.
/// * Unavailable strategies contribute 0 — not redistributed.
/// * Candidates deduplicated by (semantic_id, content_version) before fusion:
///   only the best (lowest) rank per strategy per (id, version) pair is used.
/// * Results sorted by rrf_score DESC, then semantic_id ASC (stable).
/// * No truncation (caller applies budget cap from `rrf_profile::HARD_UNIQUE_CANDIDATE_CAP`).
pub fn fuse_candidates(
    strategies: &[StrategyInput],
    profile: &FusionProfile,
) -> Result<Vec<FusedCandidate>, RrfError> {
    // Step 1: Validate all ranks up-front and build per-strategy deduped maps.
    // Map: strategy → { (semantic_id, content_version) → best_rank }
    let mut strategy_deduped: Vec<(
        StrategyKind,
        StrategyAvailability,
        HashMap<(String, String), u32>,
    )> = Vec::with_capacity(strategies.len());

    for input in strategies {
        let mut best_ranks: HashMap<(String, String), u32> = HashMap::new();

        for candidate in &input.candidates {
            // Reject zero ranks regardless of availability (fail fast).
            if candidate.rank == 0 {
                return Err(RrfError::ZeroRank {
                    strategy: input.strategy,
                    semantic_id: candidate.semantic_id.clone(),
                });
            }
            let key = (
                candidate.semantic_id.clone(),
                candidate.content_version.clone(),
            );
            best_ranks
                .entry(key)
                .and_modify(|existing| {
                    if candidate.rank < *existing {
                        *existing = candidate.rank;
                    }
                })
                .or_insert(candidate.rank);
        }

        strategy_deduped.push((input.strategy, input.availability, best_ranks));
    }

    // Step 2: Accumulate contributions into a map keyed by (semantic_id, content_version).
    // Dedup key matches the accumulation key so different content versions of the same
    // semantic_id are tracked as independent entries (as specified in the design).
    let mut acc: HashMap<(String, String), StrategyContributions> = HashMap::new();

    for (strategy_kind, availability, best_ranks) in &strategy_deduped {
        // Unavailable strategies contribute exactly 0 — skip accumulation.
        if *availability == StrategyAvailability::Unavailable {
            continue;
        }

        let weight = weight_for_strategy(*strategy_kind, profile);
        let k = profile.k;

        for ((semantic_id, content_version), &rank) in best_ranks {
            let contribution = weight / (k + rank as f32);

            let entry = acc
                .entry((semantic_id.clone(), content_version.clone()))
                .or_insert(StrategyContributions {
                    fts: 0.0,
                    vector: 0.0,
                    graph: 0.0,
                    temporal: 0.0,
                    goal: 0.0,
                });

            apply_contribution(entry, *strategy_kind, contribution);
        }
    }

    // Step 3: Build the final sorted result list.
    let mut results: Vec<FusedCandidate> = acc
        .into_iter()
        .map(|((semantic_id, content_version), contributions)| {
            let rrf_score = contributions.total();
            FusedCandidate {
                semantic_id,
                content_version,
                rrf_score,
                contributions,
            }
        })
        .collect();

    // Sort: rrf_score DESC, then semantic_id ASC (deterministic tie-break).
    results.sort_unstable_by(|a, b| {
        b.rrf_score
            .partial_cmp(&a.rrf_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.semantic_id.cmp(&b.semantic_id))
    });

    Ok(results)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Return the profile weight for a given strategy kind.
#[inline]
fn weight_for_strategy(kind: StrategyKind, profile: &FusionProfile) -> f32 {
    match kind {
        StrategyKind::Fts => profile.weights.fts,
        StrategyKind::Vector => profile.weights.vector,
        StrategyKind::Graph => profile.weights.graph,
        StrategyKind::Temporal => profile.weights.temporal,
        StrategyKind::Goal => profile.weights.goal,
    }
}

/// Accumulate `contribution` into the correct field of `contributions`.
#[inline]
fn apply_contribution(c: &mut StrategyContributions, kind: StrategyKind, value: f32) {
    match kind {
        StrategyKind::Fts => c.fts += value,
        StrategyKind::Vector => c.vector += value,
        StrategyKind::Graph => c.graph += value,
        StrategyKind::Temporal => c.temporal += value,
        StrategyKind::Goal => c.goal += value,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retrieval::rrf_profile::{DEFAULT_RRF_K, PROFILE_EXPLORATORY};

    /// Helper: build a single-candidate StrategyInput.
    fn si(
        kind: StrategyKind,
        avail: StrategyAvailability,
        id: &str,
        version: &str,
        rank: u32,
    ) -> StrategyInput {
        StrategyInput {
            strategy: kind,
            availability: avail,
            candidates: vec![StrategyCandidate {
                semantic_id: id.to_string(),
                content_version: version.to_string(),
                rank,
            }],
        }
    }

    /// Helper: build an available StrategyInput with multiple candidates.
    fn si_multi(kind: StrategyKind, candidates: Vec<(&str, &str, u32)>) -> StrategyInput {
        StrategyInput {
            strategy: kind,
            availability: StrategyAvailability::Available,
            candidates: candidates
                .into_iter()
                .map(|(id, ver, rank)| StrategyCandidate {
                    semantic_id: id.to_string(),
                    content_version: ver.to_string(),
                    rank,
                })
                .collect(),
        }
    }

    // ── 1. Basic correctness ──────────────────────────────────────────────────

    #[test]
    fn single_strategy_score_formula_is_correct() {
        // rank=1, fts weight from PROFILE_EXPLORATORY = 1.0, k = 60.0
        // expected score = 1.0 / (60.0 + 1.0) = 1.0 / 61.0
        let profile = &PROFILE_EXPLORATORY;
        let strategies = vec![si(
            StrategyKind::Fts,
            StrategyAvailability::Available,
            "id-a",
            "v1",
            1,
        )];
        let results = fuse_candidates(&strategies, profile).unwrap();
        assert_eq!(results.len(), 1);
        let expected = profile.weights.fts / (DEFAULT_RRF_K + 1.0);
        assert!(
            (results[0].rrf_score - expected).abs() < 1e-6,
            "expected {expected}, got {}",
            results[0].rrf_score
        );
        assert!((results[0].contributions.fts - expected).abs() < 1e-6);
    }

    #[test]
    fn two_strategies_scores_sum_correctly() {
        // FTS rank=1, Vector rank=2 for the same candidate.
        // score = fts_w/(k+1) + vec_w/(k+2)
        let profile = &PROFILE_EXPLORATORY;
        let strategies = vec![
            si(
                StrategyKind::Fts,
                StrategyAvailability::Available,
                "id-a",
                "v1",
                1,
            ),
            si(
                StrategyKind::Vector,
                StrategyAvailability::Available,
                "id-a",
                "v1",
                2,
            ),
        ];
        let results = fuse_candidates(&strategies, profile).unwrap();
        assert_eq!(results.len(), 1);
        let expected_fts = profile.weights.fts / (DEFAULT_RRF_K + 1.0);
        let expected_vec = profile.weights.vector / (DEFAULT_RRF_K + 2.0);
        let expected_total = expected_fts + expected_vec;
        assert!(
            (results[0].rrf_score - expected_total).abs() < 1e-6,
            "expected {expected_total}, got {}",
            results[0].rrf_score
        );
        assert!((results[0].contributions.fts - expected_fts).abs() < 1e-6);
        assert!((results[0].contributions.vector - expected_vec).abs() < 1e-6);
    }

    #[test]
    fn unavailable_strategy_contributes_zero() {
        // FTS available, Vector unavailable.
        let profile = &PROFILE_EXPLORATORY;
        let strategies = vec![
            si(
                StrategyKind::Fts,
                StrategyAvailability::Available,
                "id-a",
                "v1",
                1,
            ),
            si(
                StrategyKind::Vector,
                StrategyAvailability::Unavailable,
                "id-a",
                "v1",
                1,
            ),
        ];
        let results = fuse_candidates(&strategies, profile).unwrap();
        assert_eq!(results.len(), 1);
        let expected_fts = profile.weights.fts / (DEFAULT_RRF_K + 1.0);
        assert!(
            (results[0].rrf_score - expected_fts).abs() < 1e-6,
            "score should equal only fts contribution"
        );
        assert_eq!(results[0].contributions.vector, 0.0);
    }

    #[test]
    fn missing_strategy_candidate_contributes_zero() {
        // FTS has candidate X, Vector doesn't include candidate X.
        let profile = &PROFILE_EXPLORATORY;
        let strategies = vec![
            si(
                StrategyKind::Fts,
                StrategyAvailability::Available,
                "id-x",
                "v1",
                1,
            ),
            // Vector is available but returns a different candidate.
            si(
                StrategyKind::Vector,
                StrategyAvailability::Available,
                "id-y",
                "v1",
                1,
            ),
        ];
        let results = fuse_candidates(&strategies, profile).unwrap();
        let candidate_x = results.iter().find(|c| c.semantic_id == "id-x").unwrap();
        assert_eq!(
            candidate_x.contributions.vector, 0.0,
            "vector contribution for id-x must be 0 when vector strategy doesn't include it"
        );
    }

    #[test]
    fn zero_rank_returns_error() {
        let profile = &PROFILE_EXPLORATORY;
        let strategies = vec![StrategyInput {
            strategy: StrategyKind::Fts,
            availability: StrategyAvailability::Available,
            candidates: vec![StrategyCandidate {
                semantic_id: "id-a".to_string(),
                content_version: "v1".to_string(),
                rank: 0,
            }],
        }];
        let result = fuse_candidates(&strategies, profile);
        assert_eq!(
            result,
            Err(RrfError::ZeroRank {
                strategy: StrategyKind::Fts,
                semantic_id: "id-a".to_string(),
            })
        );
    }

    // ── 2. Deduplication ──────────────────────────────────────────────────────

    #[test]
    fn duplicate_semantic_id_same_strategy_uses_best_rank() {
        // Same semantic_id at rank 3 and rank 1 in FTS → uses rank 1.
        let profile = &PROFILE_EXPLORATORY;
        let strategies = vec![StrategyInput {
            strategy: StrategyKind::Fts,
            availability: StrategyAvailability::Available,
            candidates: vec![
                StrategyCandidate {
                    semantic_id: "id-a".to_string(),
                    content_version: "v1".to_string(),
                    rank: 3,
                },
                StrategyCandidate {
                    semantic_id: "id-a".to_string(),
                    content_version: "v1".to_string(),
                    rank: 1,
                },
            ],
        }];
        let results = fuse_candidates(&strategies, profile).unwrap();
        assert_eq!(
            results.len(),
            1,
            "duplicates must be collapsed to one entry"
        );
        let expected = profile.weights.fts / (DEFAULT_RRF_K + 1.0); // rank 1 used
        assert!(
            (results[0].rrf_score - expected).abs() < 1e-6,
            "best rank (1) should be used, expected {expected}, got {}",
            results[0].rrf_score
        );
    }

    #[test]
    fn different_content_version_same_id_treated_as_separate() {
        // Same semantic_id but different content_version → two separate fused entries.
        let profile = &PROFILE_EXPLORATORY;
        let strategies = vec![StrategyInput {
            strategy: StrategyKind::Fts,
            availability: StrategyAvailability::Available,
            candidates: vec![
                StrategyCandidate {
                    semantic_id: "id-a".to_string(),
                    content_version: "v1".to_string(),
                    rank: 1,
                },
                StrategyCandidate {
                    semantic_id: "id-a".to_string(),
                    content_version: "v2".to_string(),
                    rank: 2,
                },
            ],
        }];
        let results = fuse_candidates(&strategies, profile).unwrap();
        assert_eq!(
            results.len(),
            2,
            "different content_versions of the same semantic_id are separate entries"
        );
    }

    // ── 3. Tie-breaking ───────────────────────────────────────────────────────

    #[test]
    fn equal_score_sorted_by_semantic_id_asc() {
        // Two candidates with identical scores (same rank, same weight) from the same strategy.
        // They must appear sorted by semantic_id ASC.
        let profile = &PROFILE_EXPLORATORY;
        let strategies = vec![StrategyInput {
            strategy: StrategyKind::Fts,
            availability: StrategyAvailability::Available,
            candidates: vec![
                StrategyCandidate {
                    semantic_id: "zzz-id".to_string(),
                    content_version: "v1".to_string(),
                    rank: 1,
                },
                StrategyCandidate {
                    semantic_id: "aaa-id".to_string(),
                    content_version: "v1".to_string(),
                    rank: 1,
                },
            ],
        }];
        let results = fuse_candidates(&strategies, profile).unwrap();
        assert_eq!(results.len(), 2);
        // Both have score w/(k+1); tie-break is semantic_id ASC.
        assert_eq!(results[0].semantic_id, "aaa-id");
        assert_eq!(results[1].semantic_id, "zzz-id");
    }

    // ── 4. Multi-strategy ─────────────────────────────────────────────────────

    #[test]
    fn higher_rank_lower_score() {
        // rank=1 scores higher than rank=5 for the same strategy+weight.
        let profile = &PROFILE_EXPLORATORY;
        let strategies = vec![StrategyInput {
            strategy: StrategyKind::Fts,
            availability: StrategyAvailability::Available,
            candidates: vec![
                StrategyCandidate {
                    semantic_id: "id-rank1".to_string(),
                    content_version: "v1".to_string(),
                    rank: 1,
                },
                StrategyCandidate {
                    semantic_id: "id-rank5".to_string(),
                    content_version: "v1".to_string(),
                    rank: 5,
                },
            ],
        }];
        let results = fuse_candidates(&strategies, profile).unwrap();
        let r1 = results
            .iter()
            .find(|c| c.semantic_id == "id-rank1")
            .unwrap();
        let r5 = results
            .iter()
            .find(|c| c.semantic_id == "id-rank5")
            .unwrap();
        assert!(
            r1.rrf_score > r5.rrf_score,
            "rank=1 ({}) should score higher than rank=5 ({})",
            r1.rrf_score,
            r5.rrf_score
        );
        // Verify formula: w/(k+1) vs w/(k+5)
        let w = profile.weights.fts;
        let k = profile.k;
        assert!((r1.rrf_score - w / (k + 1.0)).abs() < 1e-6);
        assert!((r5.rrf_score - w / (k + 5.0)).abs() < 1e-6);
    }

    #[test]
    fn unavailable_weight_not_redistributed() {
        // Vector is unavailable: FTS weight must remain at profile.weights.fts, not be scaled up.
        let profile = &PROFILE_EXPLORATORY;
        let strategies = vec![
            si(
                StrategyKind::Fts,
                StrategyAvailability::Available,
                "id-a",
                "v1",
                1,
            ),
            si(
                StrategyKind::Vector,
                StrategyAvailability::Unavailable,
                "id-a",
                "v1",
                1,
            ),
        ];
        let results = fuse_candidates(&strategies, profile).unwrap();
        assert_eq!(results.len(), 1);
        // fts contribution must exactly equal fts_weight / (k + 1), not any inflated value
        let expected_fts = profile.weights.fts / (DEFAULT_RRF_K + 1.0);
        assert!(
            (results[0].contributions.fts - expected_fts).abs() < 1e-6,
            "FTS weight must NOT be redistributed: expected {expected_fts}, got {}",
            results[0].contributions.fts
        );
        // total score equals only fts contribution
        assert!((results[0].rrf_score - expected_fts).abs() < 1e-6);
    }

    // ── 5. Edge cases ─────────────────────────────────────────────────────────

    #[test]
    fn empty_strategy_list_returns_empty() {
        let profile = &PROFILE_EXPLORATORY;
        let results = fuse_candidates(&[], profile).unwrap();
        assert!(
            results.is_empty(),
            "empty strategy list must yield empty results"
        );
    }

    #[test]
    fn all_unavailable_returns_empty() {
        let profile = &PROFILE_EXPLORATORY;
        let strategies = vec![
            StrategyInput {
                strategy: StrategyKind::Fts,
                availability: StrategyAvailability::Unavailable,
                candidates: vec![],
            },
            StrategyInput {
                strategy: StrategyKind::Vector,
                availability: StrategyAvailability::Unavailable,
                candidates: vec![],
            },
        ];
        let results = fuse_candidates(&strategies, profile).unwrap();
        assert!(
            results.is_empty(),
            "all unavailable strategies must yield empty results"
        );
    }

    #[test]
    fn results_sorted_by_score_descending() {
        // Three candidates with different ranks → scores must appear in DESC order.
        let profile = &PROFILE_EXPLORATORY;
        let strategies = vec![si_multi(
            StrategyKind::Vector,
            vec![
                ("id-rank5", "v1", 5),
                ("id-rank1", "v1", 1),
                ("id-rank3", "v1", 3),
            ],
        )];
        let results = fuse_candidates(&strategies, profile).unwrap();
        assert_eq!(results.len(), 3);
        // Verify descending order.
        assert!(results[0].rrf_score >= results[1].rrf_score);
        assert!(results[1].rrf_score >= results[2].rrf_score);
        // rank=1 should be first
        assert_eq!(results[0].semantic_id, "id-rank1");
    }

    #[test]
    fn contributions_sum_to_rrf_score() {
        // For all results, contributions.total() ≈ rrf_score.
        let profile = &PROFILE_EXPLORATORY;
        let strategies = vec![
            si_multi(
                StrategyKind::Fts,
                vec![("id-a", "v1", 1), ("id-b", "v1", 3), ("id-c", "v1", 5)],
            ),
            si_multi(
                StrategyKind::Vector,
                vec![("id-a", "v1", 2), ("id-c", "v1", 1)],
            ),
            si_multi(StrategyKind::Graph, vec![("id-b", "v1", 1)]),
        ];
        let results = fuse_candidates(&strategies, profile).unwrap();
        for candidate in &results {
            let diff = (candidate.contributions.total() - candidate.rrf_score).abs();
            assert!(
                diff < 1e-6,
                "contributions.total()={} != rrf_score={} for id={}",
                candidate.contributions.total(),
                candidate.rrf_score,
                candidate.semantic_id
            );
        }
    }
}
