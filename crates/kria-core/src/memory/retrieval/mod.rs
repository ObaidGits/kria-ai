//! Retrieval strategies (design §6.5) — bounded, policy-first candidate
//! generators that feed the fusion engine.
//!
//! Each sub-module exposes one independently testable strategy.  All strategies
//! are read-only, bounded, and degrade gracefully when their backing index is
//! unavailable.  Policy is applied BEFORE every seed resolution and edge
//! expansion.

use std::time::Instant;

/// A simple wall-clock deadline for retrieval strategy calls.
///
/// Strategies check `is_expired()` at safe yield points and return a partial
/// result with `partial = true` when the deadline has passed.  Callers must
/// treat a partial result as a `Partial` trace (design §6.4 gate step 5).
#[derive(Debug, Clone, Copy)]
pub struct StrategyDeadline {
    deadline: Instant,
}

impl StrategyDeadline {
    /// Create a deadline `millis` milliseconds from now.
    pub fn from_millis(millis: u64) -> Self {
        Self {
            deadline: Instant::now() + std::time::Duration::from_millis(millis),
        }
    }

    /// Create a deadline that never expires (useful for tests / no-deadline paths).
    pub fn never() -> Self {
        // Use 100 years (~876,000 hours) — large enough to never fire in practice
        // but safe from Duration overflow on all platforms.
        Self {
            deadline: Instant::now() + std::time::Duration::from_secs(100 * 365 * 24 * 3600),
        }
    }

    /// Returns `true` when the deadline has passed.
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.deadline
    }
}

pub mod classifier;
pub mod diversity_select;
pub mod goal_strategy;
pub mod graph_strategy;
pub mod profile_eval;
pub mod profile_registry;
pub mod retrieval_explanation;
pub mod retrieval_gates;
pub mod rrf_fusion;
pub mod rrf_profile;
pub mod temporal_strategy;
pub mod token_packing;
pub mod trace_builder;
pub mod trace_finalizer;
pub mod trace_store;
pub mod version_gate;
