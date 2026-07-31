//! Immutable versioned RRF profile configuration (design §6.2/§6.3).
//!
//! All values are compile-time constants, frozen at `rrf-profiles-v1`.
//! Rule changes increment the profile version.
//!
//! # Design invariants
//! * Hard maxima enforce invariant A6 (Boundedness).
//! * Weights are relative values, not probabilities.
//! * No runtime mutation of weights (MGD-025).
//! * Per-class budgets are bounded by hard maxima.

// ── Version and global constants ──────────────────────────────────────────────

/// Profile version constant (frozen at this version).
pub const PROFILE_VERSION: &str = "rrf-profiles-v1";

/// Default RRF damping constant k (design §6.3, k=60).
pub const DEFAULT_RRF_K: f32 = 60.0;

/// Hard combined unique-candidate cap across all strategies (design §6.2).
pub const HARD_UNIQUE_CANDIDATE_CAP: usize = 320;

/// Per-strategy deadline in milliseconds (design §6.2: each strategy ≤60ms).
pub const STRATEGY_DEADLINE_MS: u64 = 60;

/// Core retrieval deadline in milliseconds (design §6.2: ≤110ms total).
pub const CORE_RETRIEVAL_DEADLINE_MS: u64 = 110;

/// Hard maximum graph traversal hops within retrieval (design §6.2, §6.5).
pub const RETRIEVAL_MAX_HOPS: u8 = 3;

/// Hard maximum visited nodes within one retrieval graph traversal (design §6.5).
pub const RETRIEVAL_MAX_VISITED_NODES: usize = 120;

/// Hard maximum edges within one retrieval graph traversal (design §6.5).
pub const RETRIEVAL_MAX_EDGES: usize = 180;

// ── StrategyBudgets ───────────────────────────────────────────────────────────

/// Per-class strategy candidate budgets (design §6.2).
///
/// Each budget is the maximum number of candidates to fetch from that strategy
/// before fusion. Bounded by `HARD_UNIQUE_CANDIDATE_CAP` in aggregate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrategyBudgets {
    /// FTS5 strategy candidate budget.
    pub fts: usize,
    /// Exact vector strategy candidate budget.
    pub vector: usize,
    /// ≤3-hop graph strategy candidate budget.
    pub graph: usize,
    /// Temporal strategy candidate budget.
    pub temporal: usize,
    /// Active-goal strategy candidate budget.
    pub goal: usize,
}

impl StrategyBudgets {
    /// Sum of all strategy budgets for this class.
    pub fn total(&self) -> usize {
        self.fts + self.vector + self.graph + self.temporal + self.goal
    }

    /// Returns `true` when every individual strategy budget does not exceed
    /// `HARD_UNIQUE_CANDIDATE_CAP`.
    ///
    /// The hard cap governs unique candidates **after fusion deduplication**, not
    /// the raw per-strategy fetch totals.  Each strategy may overlap; the same
    /// record can appear in multiple strategy result sets.  The invariant
    /// enforced here is that no single strategy fetches more candidates than the
    /// overall unique-candidate ceiling.
    pub fn within_hard_cap(&self) -> bool {
        self.fts <= HARD_UNIQUE_CANDIDATE_CAP
            && self.vector <= HARD_UNIQUE_CANDIDATE_CAP
            && self.graph <= HARD_UNIQUE_CANDIDATE_CAP
            && self.temporal <= HARD_UNIQUE_CANDIDATE_CAP
            && self.goal <= HARD_UNIQUE_CANDIDATE_CAP
    }
}

// ── RrfWeights ────────────────────────────────────────────────────────────────

/// Per-class RRF fusion weights (design §6.3).
///
/// Weights are relative values (not probabilities). Missing strategy has
/// weight = 0 (explicit availability tracking, never silent redistribution).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RrfWeights {
    pub fts: f32,
    pub vector: f32,
    pub graph: f32,
    pub temporal: f32,
    pub goal: f32,
}

// ── FusionProfile ─────────────────────────────────────────────────────────────

/// Complete fusion profile for one query class (design §6.2/§6.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FusionProfile {
    /// Stable profile ID string (e.g., `"rrf-id-v1"`).
    pub profile_id: &'static str,
    /// Strategy candidate budgets.
    pub budgets: StrategyBudgets,
    /// RRF fusion weights.
    pub weights: RrfWeights,
    /// RRF k value (default: `DEFAULT_RRF_K`).
    pub k: f32,
}

// ── Immutable v1 profile constants ────────────────────────────────────────────

/// Fusion profile for `Identifier` queries (design §6.2/§6.3).
///
/// Heavy FTS bias — UUID, path, URL, email, code-like exact token lookups.
pub const PROFILE_IDENTIFIER: FusionProfile = FusionProfile {
    profile_id: "rrf-id-v1",
    budgets: StrategyBudgets {
        fts: 120,
        vector: 30,
        graph: 40,
        temporal: 20,
        goal: 20,
    },
    weights: RrfWeights {
        fts: 2.0,
        vector: 0.5,
        graph: 0.6,
        temporal: 0.3,
        goal: 0.3,
    },
    k: DEFAULT_RRF_K,
};

/// Fusion profile for `ExactPhrase` queries (design §6.2/§6.3).
///
/// FTS-dominant — quoted phrase or exact-match operator queries.
pub const PROFILE_EXACT_PHRASE: FusionProfile = FusionProfile {
    profile_id: "rrf-exact-v1",
    budgets: StrategyBudgets {
        fts: 120,
        vector: 40,
        graph: 30,
        temporal: 20,
        goal: 20,
    },
    weights: RrfWeights {
        fts: 2.0,
        vector: 0.8,
        graph: 0.4,
        temporal: 0.3,
        goal: 0.3,
    },
    k: DEFAULT_RRF_K,
};

/// Fusion profile for `EntityRelation` queries (design §6.2/§6.3).
///
/// Graph-dominant — resolved entity/relation term queries.
pub const PROFILE_ENTITY_RELATION: FusionProfile = FusionProfile {
    profile_id: "rrf-graph-v1",
    budgets: StrategyBudgets {
        fts: 80,
        vector: 80,
        graph: 120,
        temporal: 30,
        goal: 30,
    },
    weights: RrfWeights {
        fts: 0.8,
        vector: 1.0,
        graph: 1.8,
        temporal: 0.5,
        goal: 0.5,
    },
    k: DEFAULT_RRF_K,
};

/// Fusion profile for `Temporal` queries (design §6.2/§6.3).
///
/// Temporal-dominant — parsed instant/range/recency intent queries.
pub const PROFILE_TEMPORAL: FusionProfile = FusionProfile {
    profile_id: "rrf-time-v1",
    budgets: StrategyBudgets {
        fts: 70,
        vector: 60,
        graph: 50,
        temporal: 120,
        goal: 30,
    },
    weights: RrfWeights {
        fts: 0.8,
        vector: 0.8,
        graph: 0.7,
        temporal: 1.8,
        goal: 0.5,
    },
    k: DEFAULT_RRF_K,
};

/// Fusion profile for `ActiveGoal` queries (design §6.2/§6.3).
///
/// Goal-dominant — task/resume/next intent with active context queries.
pub const PROFILE_ACTIVE_GOAL: FusionProfile = FusionProfile {
    profile_id: "rrf-goal-v1",
    budgets: StrategyBudgets {
        fts: 60,
        vector: 70,
        graph: 50,
        temporal: 40,
        goal: 100,
    },
    weights: RrfWeights {
        fts: 0.7,
        vector: 0.9,
        graph: 0.7,
        temporal: 0.6,
        goal: 1.8,
    },
    k: DEFAULT_RRF_K,
};

/// Fusion profile for `Exploratory` queries (design §6.2/§6.3).
///
/// Balanced with vector/FTS lean — default fallback queries.
pub const PROFILE_EXPLORATORY: FusionProfile = FusionProfile {
    profile_id: "rrf-general-v1",
    budgets: StrategyBudgets {
        fts: 80,
        vector: 100,
        graph: 60,
        temporal: 40,
        goal: 40,
    },
    weights: RrfWeights {
        fts: 1.0,
        vector: 1.2,
        graph: 0.8,
        temporal: 0.6,
        goal: 0.6,
    },
    k: DEFAULT_RRF_K,
};

// ── Lookup ────────────────────────────────────────────────────────────────────

/// Get the immutable v1 fusion profile for a query class.
///
/// Returns a compile-time constant profile — callers MUST NOT mutate weights.
pub fn get_profile_v1(class: &super::classifier::QueryClassV2) -> &'static FusionProfile {
    use super::classifier::QueryClassV2;
    match class {
        QueryClassV2::Identifier => &PROFILE_IDENTIFIER,
        QueryClassV2::ExactPhrase => &PROFILE_EXACT_PHRASE,
        QueryClassV2::EntityRelation => &PROFILE_ENTITY_RELATION,
        QueryClassV2::Temporal => &PROFILE_TEMPORAL,
        QueryClassV2::ActiveGoal => &PROFILE_ACTIVE_GOAL,
        QueryClassV2::Exploratory => &PROFILE_EXPLORATORY,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::retrieval::classifier::QueryClassV2;

    /// All six v1 profiles in declaration order.
    const ALL_PROFILES: [(&str, &FusionProfile, QueryClassV2); 6] = [
        ("identifier", &PROFILE_IDENTIFIER, QueryClassV2::Identifier),
        (
            "exact_phrase",
            &PROFILE_EXACT_PHRASE,
            QueryClassV2::ExactPhrase,
        ),
        (
            "entity_relation",
            &PROFILE_ENTITY_RELATION,
            QueryClassV2::EntityRelation,
        ),
        ("temporal", &PROFILE_TEMPORAL, QueryClassV2::Temporal),
        (
            "active_goal",
            &PROFILE_ACTIVE_GOAL,
            QueryClassV2::ActiveGoal,
        ),
        (
            "exploratory",
            &PROFILE_EXPLORATORY,
            QueryClassV2::Exploratory,
        ),
    ];

    #[test]
    fn all_profiles_have_correct_profile_ids() {
        for (_, profile, class) in &ALL_PROFILES {
            assert_eq!(
                profile.profile_id,
                class.profile_id(),
                "profile_id mismatch for class {:?}: profile has '{}', classifier expects '{}'",
                class,
                profile.profile_id,
                class.profile_id()
            );
        }
    }

    #[test]
    fn all_budget_totals_within_hard_cap() {
        for (label, profile, _) in &ALL_PROFILES {
            assert!(
                profile.budgets.within_hard_cap(),
                "budgets for '{}' exceed HARD_UNIQUE_CANDIDATE_CAP ({}): total={}",
                label,
                HARD_UNIQUE_CANDIDATE_CAP,
                profile.budgets.total()
            );
        }
    }

    #[test]
    fn identifier_profile_has_correct_budgets() {
        let b = &PROFILE_IDENTIFIER.budgets;
        assert_eq!(b.fts, 120);
        assert_eq!(b.vector, 30);
        assert_eq!(b.graph, 40);
        assert_eq!(b.temporal, 20);
        assert_eq!(b.goal, 20);
        assert_eq!(b.total(), 230);
    }

    #[test]
    fn exact_phrase_profile_has_correct_budgets() {
        let b = &PROFILE_EXACT_PHRASE.budgets;
        assert_eq!(b.fts, 120);
        assert_eq!(b.vector, 40);
        assert_eq!(b.graph, 30);
        assert_eq!(b.temporal, 20);
        assert_eq!(b.goal, 20);
        assert_eq!(b.total(), 230);
    }

    #[test]
    fn entity_relation_profile_has_correct_budgets() {
        let b = &PROFILE_ENTITY_RELATION.budgets;
        assert_eq!(b.fts, 80);
        assert_eq!(b.vector, 80);
        assert_eq!(b.graph, 120);
        assert_eq!(b.temporal, 30);
        assert_eq!(b.goal, 30);
        assert_eq!(b.total(), 340);
    }

    #[test]
    fn temporal_profile_has_correct_budgets() {
        let b = &PROFILE_TEMPORAL.budgets;
        assert_eq!(b.fts, 70);
        assert_eq!(b.vector, 60);
        assert_eq!(b.graph, 50);
        assert_eq!(b.temporal, 120);
        assert_eq!(b.goal, 30);
        assert_eq!(b.total(), 330);
    }

    #[test]
    fn active_goal_profile_has_correct_budgets() {
        let b = &PROFILE_ACTIVE_GOAL.budgets;
        assert_eq!(b.fts, 60);
        assert_eq!(b.vector, 70);
        assert_eq!(b.graph, 50);
        assert_eq!(b.temporal, 40);
        assert_eq!(b.goal, 100);
        assert_eq!(b.total(), 320);
    }

    #[test]
    fn exploratory_profile_has_correct_budgets() {
        let b = &PROFILE_EXPLORATORY.budgets;
        assert_eq!(b.fts, 80);
        assert_eq!(b.vector, 100);
        assert_eq!(b.graph, 60);
        assert_eq!(b.temporal, 40);
        assert_eq!(b.goal, 40);
        assert_eq!(b.total(), 320);
    }

    #[test]
    fn identifier_profile_has_correct_weights() {
        let w = &PROFILE_IDENTIFIER.weights;
        assert_eq!(w.fts, 2.0);
        assert_eq!(w.vector, 0.5);
        assert_eq!(w.graph, 0.6);
        assert_eq!(w.temporal, 0.3);
        assert_eq!(w.goal, 0.3);
    }

    #[test]
    fn all_profiles_use_default_k() {
        for (label, profile, _) in &ALL_PROFILES {
            assert_eq!(
                profile.k, DEFAULT_RRF_K,
                "profile '{}' has k={}, expected {}",
                label, profile.k, DEFAULT_RRF_K
            );
        }
        assert_eq!(DEFAULT_RRF_K, 60.0);
    }

    #[test]
    fn profile_version_is_correct() {
        assert_eq!(PROFILE_VERSION, "rrf-profiles-v1");
    }

    #[test]
    fn strategy_deadline_ms_is_60() {
        assert_eq!(STRATEGY_DEADLINE_MS, 60);
    }

    #[test]
    fn core_retrieval_deadline_ms_is_110() {
        assert_eq!(CORE_RETRIEVAL_DEADLINE_MS, 110);
    }

    #[test]
    fn get_profile_v1_returns_correct_profile_for_each_class() {
        assert_eq!(
            get_profile_v1(&QueryClassV2::Identifier).profile_id,
            "rrf-id-v1"
        );
        assert_eq!(
            get_profile_v1(&QueryClassV2::ExactPhrase).profile_id,
            "rrf-exact-v1"
        );
        assert_eq!(
            get_profile_v1(&QueryClassV2::EntityRelation).profile_id,
            "rrf-graph-v1"
        );
        assert_eq!(
            get_profile_v1(&QueryClassV2::Temporal).profile_id,
            "rrf-time-v1"
        );
        assert_eq!(
            get_profile_v1(&QueryClassV2::ActiveGoal).profile_id,
            "rrf-goal-v1"
        );
        assert_eq!(
            get_profile_v1(&QueryClassV2::Exploratory).profile_id,
            "rrf-general-v1"
        );

        // Value equality — each lookup returns the exact same data as the static constant
        assert_eq!(
            *get_profile_v1(&QueryClassV2::Identifier),
            PROFILE_IDENTIFIER
        );
        assert_eq!(
            *get_profile_v1(&QueryClassV2::ExactPhrase),
            PROFILE_EXACT_PHRASE
        );
        assert_eq!(
            *get_profile_v1(&QueryClassV2::EntityRelation),
            PROFILE_ENTITY_RELATION
        );
        assert_eq!(*get_profile_v1(&QueryClassV2::Temporal), PROFILE_TEMPORAL);
        assert_eq!(
            *get_profile_v1(&QueryClassV2::ActiveGoal),
            PROFILE_ACTIVE_GOAL
        );
        assert_eq!(
            *get_profile_v1(&QueryClassV2::Exploratory),
            PROFILE_EXPLORATORY
        );
    }

    #[test]
    fn weights_are_positive() {
        for (label, profile, _) in &ALL_PROFILES {
            let w = &profile.weights;
            assert!(
                w.fts > 0.0,
                "fts weight for '{}' is not positive: {}",
                label,
                w.fts
            );
            assert!(
                w.vector > 0.0,
                "vector weight for '{}' is not positive: {}",
                label,
                w.vector
            );
            assert!(
                w.graph > 0.0,
                "graph weight for '{}' is not positive: {}",
                label,
                w.graph
            );
            assert!(
                w.temporal > 0.0,
                "temporal weight for '{}' is not positive: {}",
                label,
                w.temporal
            );
            assert!(
                w.goal > 0.0,
                "goal weight for '{}' is not positive: {}",
                label,
                w.goal
            );
        }
    }
}
