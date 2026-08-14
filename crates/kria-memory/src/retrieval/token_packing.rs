//! Greedy token packing for the retrieval pipeline (design §6.4 step 7, task F3.5.4).
//!
//! Packs diversity-selected candidates into the caller token budget, reserving
//! 10% for exact identifiers and 10% for active-goal context when present.
//!
//! # Design invariants
//! * Never exceeds total token budget.
//! * 10% identifier reserve applied BEFORE general packing.
//! * 10% goal reserve applied BEFORE general packing.
//! * Greedy marginal-utility-per-token ordering: highest score/token first.
//! * Identifier candidates are NOT counted against the goal reserve and vice versa.

// ── Types ─────────────────────────────────────────────────────────────────────

/// One candidate available for token packing.
#[derive(Debug, Clone)]
pub struct PackingCandidate {
    /// Stable semantic identifier.
    pub semantic_id: String,
    /// Number of tokens required to include this candidate.
    pub token_cost: usize,
    /// The relevance score (RRF score or similar). Higher = more useful.
    pub score: f32,
    /// Whether this candidate comes from an identifier query class (affects reserve).
    pub is_identifier: bool,
    /// Whether this candidate comes from an active-goal strategy (affects reserve).
    pub is_active_goal: bool,
}

/// The result of packing for one candidate.
#[derive(Debug, Clone, PartialEq)]
pub enum PackingOutcome {
    /// Candidate was allocated tokens and included in context.
    Allocated { allocated_tokens: usize },
    /// Candidate was excluded due to budget exhaustion.
    BudgetExhausted,
}

/// Summary of a packing run.
#[derive(Debug, Clone)]
pub struct PackingResult {
    /// Per-candidate outcomes in input order.
    pub outcomes: Vec<PackingOutcome>,
    /// Total tokens allocated.
    pub total_allocated: usize,
    /// Remaining budget after packing.
    pub remaining_budget: usize,
    /// Tokens reserved for identifiers (10% of total budget).
    pub identifier_reserve: usize,
    /// Tokens reserved for active-goal context (10% of total budget).
    pub goal_reserve: usize,
}

// ── Marginal utility ──────────────────────────────────────────────────────────

/// Marginal utility per token: `score / max(token_cost, 1)`.
///
/// Dividing by `max(token_cost, 1)` guards against zero-cost candidates
/// producing infinity or NaN.
#[inline]
fn marginal_utility(score: f32, token_cost: usize) -> f32 {
    score / (token_cost.max(1) as f32)
}

// ── Packing function ──────────────────────────────────────────────────────────

/// Greedily pack candidates into the caller's token budget.
///
/// # Budget allocation
/// * `identifier_reserve = budget / 10` (floor division)
/// * `goal_reserve = budget / 10` (floor division)
/// * `general_budget = budget - identifier_reserve - goal_reserve`
///
/// # Algorithm
/// 1. Sort identifier candidates by score/token DESC (marginal utility).
///    Fill `identifier_reserve` greedily from identifier candidates.
/// 2. Sort goal candidates by score/token DESC.
///    Fill `goal_reserve` greedily from goal candidates.
/// 3. Sort remaining unallocated candidates by score/token DESC.
///    Fill `general_budget` greedily.
/// 4. Any unfilled reserve is added to the general budget (not wasted).
///    (i.e., if there are no identifier candidates, their 10% goes to general.)
/// 5. A candidate is allocated if its token_cost <= remaining space in its pool.
///    If it doesn't fit, it is BudgetExhausted (skip; try next candidate).
///
/// # Score-per-token
/// `marginal_utility = score / max(token_cost, 1)` (avoid division by zero)
///
/// # Hard cap
/// `total_allocated` never exceeds `budget`.
pub fn pack_tokens(candidates: &[PackingCandidate], budget: usize) -> PackingResult {
    let n = candidates.len();

    // Fast path: empty input or zero budget.
    if n == 0 {
        return PackingResult {
            outcomes: vec![],
            total_allocated: 0,
            remaining_budget: budget,
            identifier_reserve: budget / 10,
            goal_reserve: budget / 10,
        };
    }

    let identifier_reserve = budget / 10;
    let goal_reserve = budget / 10;
    let general_budget_base = budget - identifier_reserve - goal_reserve;

    // Track allocation state per original index.
    let mut allocated = vec![false; n];
    let mut outcomes = vec![PackingOutcome::BudgetExhausted; n];

    // ── Phase 1: fill identifier reserve ─────────────────────────────────────

    // Build sorted index list for identifier candidates.
    let mut id_indices: Vec<usize> = (0..n).filter(|&i| candidates[i].is_identifier).collect();
    id_indices.sort_by(|&a, &b| {
        let ua = marginal_utility(candidates[a].score, candidates[a].token_cost);
        let ub = marginal_utility(candidates[b].score, candidates[b].token_cost);
        ub.partial_cmp(&ua).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut id_remaining = identifier_reserve;
    for &i in &id_indices {
        let cost = candidates[i].token_cost;
        if cost <= id_remaining {
            id_remaining -= cost;
            allocated[i] = true;
            outcomes[i] = PackingOutcome::Allocated {
                allocated_tokens: cost,
            };
        }
        // Skip (BudgetExhausted) if it doesn't fit; continue trying smaller ones.
    }
    // Unused identifier reserve rolls into general.
    let general_budget = general_budget_base + id_remaining;

    // ── Phase 2: fill goal reserve ────────────────────────────────────────────

    let mut goal_indices: Vec<usize> = (0..n)
        .filter(|&i| candidates[i].is_active_goal && !allocated[i])
        .collect();
    goal_indices.sort_by(|&a, &b| {
        let ua = marginal_utility(candidates[a].score, candidates[a].token_cost);
        let ub = marginal_utility(candidates[b].score, candidates[b].token_cost);
        ub.partial_cmp(&ua).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut goal_remaining = goal_reserve;
    for &i in &goal_indices {
        let cost = candidates[i].token_cost;
        if cost <= goal_remaining {
            goal_remaining -= cost;
            allocated[i] = true;
            outcomes[i] = PackingOutcome::Allocated {
                allocated_tokens: cost,
            };
        }
    }
    // Unused goal reserve rolls into general.
    let general_budget = general_budget + goal_remaining;

    // ── Phase 3: fill general budget with remaining unallocated candidates ────

    let mut general_indices: Vec<usize> = (0..n).filter(|&i| !allocated[i]).collect();
    general_indices.sort_by(|&a, &b| {
        let ua = marginal_utility(candidates[a].score, candidates[a].token_cost);
        let ub = marginal_utility(candidates[b].score, candidates[b].token_cost);
        ub.partial_cmp(&ua).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut gen_remaining = general_budget;
    for &i in &general_indices {
        let cost = candidates[i].token_cost;
        if cost <= gen_remaining {
            gen_remaining -= cost;
            allocated[i] = true;
            outcomes[i] = PackingOutcome::Allocated {
                allocated_tokens: cost,
            };
        }
    }

    // ── Summarise ─────────────────────────────────────────────────────────────

    let total_allocated: usize = outcomes
        .iter()
        .map(|o| match o {
            PackingOutcome::Allocated { allocated_tokens } => *allocated_tokens,
            PackingOutcome::BudgetExhausted => 0,
        })
        .sum();

    // Hard cap assertion: must never exceed budget.
    debug_assert!(
        total_allocated <= budget,
        "token packing exceeded budget: allocated={total_allocated} budget={budget}"
    );

    PackingResult {
        outcomes,
        total_allocated,
        remaining_budget: budget - total_allocated,
        identifier_reserve,
        goal_reserve,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a plain (non-identifier, non-goal) candidate.
    fn plain(semantic_id: &str, token_cost: usize, score: f32) -> PackingCandidate {
        PackingCandidate {
            semantic_id: semantic_id.to_string(),
            token_cost,
            score,
            is_identifier: false,
            is_active_goal: false,
        }
    }

    fn identifier(semantic_id: &str, token_cost: usize, score: f32) -> PackingCandidate {
        PackingCandidate {
            semantic_id: semantic_id.to_string(),
            token_cost,
            score,
            is_identifier: true,
            is_active_goal: false,
        }
    }

    fn goal(semantic_id: &str, token_cost: usize, score: f32) -> PackingCandidate {
        PackingCandidate {
            semantic_id: semantic_id.to_string(),
            token_cost,
            score,
            is_identifier: false,
            is_active_goal: true,
        }
    }

    // ── Test 1 ────────────────────────────────────────────────────────────────

    #[test]
    fn empty_candidates_returns_empty() {
        let result = pack_tokens(&[], 1000);
        assert!(result.outcomes.is_empty());
        assert_eq!(result.total_allocated, 0);
        assert_eq!(result.remaining_budget, 1000);
    }

    // ── Test 2 ────────────────────────────────────────────────────────────────

    #[test]
    fn single_candidate_within_budget() {
        let candidates = vec![plain("a", 50, 1.0)];
        let result = pack_tokens(&candidates, 100);
        assert_eq!(
            result.outcomes[0],
            PackingOutcome::Allocated {
                allocated_tokens: 50
            }
        );
        assert_eq!(result.total_allocated, 50);
    }

    // ── Test 3 ────────────────────────────────────────────────────────────────

    #[test]
    fn single_candidate_exceeds_budget() {
        let candidates = vec![plain("a", 200, 1.0)];
        let result = pack_tokens(&candidates, 100);
        assert_eq!(result.outcomes[0], PackingOutcome::BudgetExhausted);
        assert_eq!(result.total_allocated, 0);
    }

    // ── Test 4 ────────────────────────────────────────────────────────────────

    #[test]
    fn total_allocated_never_exceeds_budget() {
        let budget = 200;
        let candidates: Vec<PackingCandidate> = (0..20)
            .map(|i| plain(&format!("c{i}"), 15 + i, 1.0 + i as f32 * 0.1))
            .collect();
        let result = pack_tokens(&candidates, budget);
        assert!(
            result.total_allocated <= budget,
            "allocated {} > budget {}",
            result.total_allocated,
            budget
        );
    }

    // ── Test 5 ────────────────────────────────────────────────────────────────

    #[test]
    fn identifier_reserve_is_10_percent() {
        let result = pack_tokens(&[], 200);
        assert_eq!(result.identifier_reserve, 20); // 200 / 10
    }

    // ── Test 6 ────────────────────────────────────────────────────────────────

    #[test]
    fn goal_reserve_is_10_percent() {
        let result = pack_tokens(&[], 200);
        assert_eq!(result.goal_reserve, 20); // 200 / 10
    }

    // ── Test 7 ────────────────────────────────────────────────────────────────

    #[test]
    fn identifier_candidates_fill_from_reserve() {
        // budget=100, identifier_reserve=10, goal_reserve=10, general=80
        // identifier with cost=10 should fit exactly in reserve
        let candidates = vec![identifier("id1", 10, 5.0)];
        let result = pack_tokens(&candidates, 100);
        assert_eq!(
            result.outcomes[0],
            PackingOutcome::Allocated {
                allocated_tokens: 10
            }
        );
        assert_eq!(result.total_allocated, 10);
        // reserve was 10 and the identifier cost 10, so identifier pool is exactly spent
        assert_eq!(result.identifier_reserve, 10);
    }

    // ── Test 8 ────────────────────────────────────────────────────────────────

    #[test]
    fn general_candidates_fill_from_general_budget() {
        // budget=100, identifier_reserve=10, goal_reserve=10, general=80
        // plain candidate with cost=40 should fit in general budget
        let candidates = vec![plain("g1", 40, 2.0)];
        let result = pack_tokens(&candidates, 100);
        assert_eq!(
            result.outcomes[0],
            PackingOutcome::Allocated {
                allocated_tokens: 40
            }
        );
        assert_eq!(result.total_allocated, 40);
    }

    // ── Test 9 ────────────────────────────────────────────────────────────────

    #[test]
    fn unused_identifier_reserve_rolls_into_general() {
        // budget=100: identifier_reserve=10, goal_reserve=10, general_base=80
        // No identifier candidates → 10 rolls into general → effective general=90
        // Two plain candidates each costing 45 should both fit (45+45=90)
        let candidates = vec![plain("g1", 45, 2.0), plain("g2", 45, 1.5)];
        let result = pack_tokens(&candidates, 100);
        assert_eq!(
            result.outcomes[0],
            PackingOutcome::Allocated {
                allocated_tokens: 45
            }
        );
        assert_eq!(
            result.outcomes[1],
            PackingOutcome::Allocated {
                allocated_tokens: 45
            }
        );
        assert_eq!(result.total_allocated, 90);
    }

    // ── Test 10 ───────────────────────────────────────────────────────────────

    #[test]
    fn greedy_ordering_higher_utility_first() {
        // budget=100, no reserves used for plain candidates
        // candidate A: cost=50, score=10.0 → utility=0.2
        // candidate B: cost=10, score=10.0 → utility=1.0
        // With general budget of 80 (100 - 10 - 10), B should be allocated first,
        // then A if space remains. Total cost = 60 ≤ 80; both should be allocated.
        // But the ORDER of selection matters: B should be picked before A.
        let candidates = vec![
            plain("A", 50, 10.0), // utility = 0.2
            plain("B", 10, 10.0), // utility = 1.0
        ];
        let result = pack_tokens(&candidates, 100);
        // Both fit in general budget (80), so both are allocated.
        assert_eq!(
            result.outcomes[0],
            PackingOutcome::Allocated {
                allocated_tokens: 50
            }
        );
        assert_eq!(
            result.outcomes[1],
            PackingOutcome::Allocated {
                allocated_tokens: 10
            }
        );
        assert_eq!(result.total_allocated, 60);
    }

    #[test]
    fn greedy_ordering_only_high_utility_fits() {
        // budget=60: identifier_reserve=6, goal_reserve=6, general_base=48.
        // No identifier or goal candidates → both reserves roll into general → general=60.
        // candidate A: cost=45, score=1.0  → utility≈0.022
        // candidate B: cost=10, score=10.0 → utility=1.0
        // B is allocated first (10≤60, remaining=50). A is allocated next (45≤50).
        // Both fit. Use a tighter budget where A won't fit after B.
        // budget=20: id_reserve=2, goal_reserve=2, general_base=16; rolls→general=20.
        // B (cost=10) fits (10≤20, remaining=10). A (cost=15) > 10 → BudgetExhausted.
        let candidates = vec![
            plain("A", 15, 1.0),  // low utility, high cost
            plain("B", 10, 10.0), // high utility
        ];
        let result = pack_tokens(&candidates, 20);
        assert_eq!(result.outcomes[0], PackingOutcome::BudgetExhausted);
        assert_eq!(
            result.outcomes[1],
            PackingOutcome::Allocated {
                allocated_tokens: 10
            }
        );
    }

    // ── Test 11 ───────────────────────────────────────────────────────────────

    #[test]
    fn candidate_skipped_if_too_large_for_remaining() {
        // budget=50: identifier_reserve=5, goal_reserve=5, general_base=40.
        // No id/goal candidates → both reserves (5+5=10) roll into general → general=50.
        // A single plain candidate of cost=51 exceeds even the full budget → BudgetExhausted.
        let candidates = vec![plain("X", 51, 10.0)];
        let result = pack_tokens(&candidates, 50);
        assert_eq!(result.outcomes[0], PackingOutcome::BudgetExhausted);
        assert_eq!(result.total_allocated, 0);
    }

    // ── Test 12 ───────────────────────────────────────────────────────────────

    #[test]
    fn zero_budget_all_exhausted() {
        let candidates = vec![plain("a", 1, 1.0), plain("b", 1, 2.0), goal("g", 1, 3.0)];
        let result = pack_tokens(&candidates, 0);
        for outcome in &result.outcomes {
            assert_eq!(*outcome, PackingOutcome::BudgetExhausted);
        }
        assert_eq!(result.total_allocated, 0);
    }

    // ── Test 13 ───────────────────────────────────────────────────────────────

    #[test]
    fn outcomes_length_equals_input_length() {
        let candidates = vec![
            plain("a", 10, 1.0),
            identifier("b", 5, 2.0),
            goal("c", 8, 3.0),
            plain("d", 20, 0.5),
        ];
        let result = pack_tokens(&candidates, 100);
        assert_eq!(result.outcomes.len(), candidates.len());
    }

    // ── Extra: goal reserve also rolls into general ───────────────────────────

    #[test]
    fn unused_goal_reserve_rolls_into_general() {
        // budget=100: id_reserve=10, goal_reserve=10, general_base=80
        // No goal candidates → 10 also rolls into general → effective general=100
        // A plain candidate of cost=95 should fit.
        let candidates = vec![plain("big", 95, 1.0)];
        let result = pack_tokens(&candidates, 100);
        assert_eq!(
            result.outcomes[0],
            PackingOutcome::Allocated {
                allocated_tokens: 95
            }
        );
    }

    // ── Extra: no double-allocation ───────────────────────────────────────────

    #[test]
    fn no_double_allocation() {
        // An identifier candidate: allocated from identifier reserve.
        // It must NOT appear again in general-budget phase.
        // budget=100: id_reserve=10, goal_reserve=10, general=80
        // id candidate cost=10 → allocated from reserve (total=10)
        // general phase must not re-allocate it
        let candidates = vec![identifier("id1", 10, 5.0)];
        let result = pack_tokens(&candidates, 100);
        let alloc_count = result
            .outcomes
            .iter()
            .filter(|o| matches!(o, PackingOutcome::Allocated { .. }))
            .count();
        assert_eq!(alloc_count, 1);
        assert_eq!(result.total_allocated, 10);
    }
}
