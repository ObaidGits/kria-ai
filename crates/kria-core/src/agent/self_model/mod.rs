//! SelfModel — Capability awareness with Bayesian success rates.
//!
//! # Statistical Approach: Beta(α, β) Posterior
//!
//! Previous approach: raw percentage (success / total)
//! Problem: 1 success = 100%, massive small-sample bias
//!
//! New approach: Beta(α, β) posterior estimation
//! - Prior: Beta(1, 1) — uniform prior, starts at 0.50
//! - Update: success → α += 1, failure → β += 1
//! - Posterior mean: α / (α + β)
//!
//! This naturally handles:
//! - Unknown tools start at 0.50 (neutral)
//! - 1 success → (1+1)/(1+2) = 0.67 (not 1.00)
//! - 10 successes, 0 failures → (10+1)/(10+2) = 0.92 (high confidence)
//! - 1 success, 1 failure → (1+1)/(1+1+2) = 0.50 (neutral)

pub mod tool_stats;

pub use tool_stats::{SelfModel, ToolStats};
