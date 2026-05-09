//! Failure Analyzer — Learn from mistakes.
//!
//! # Design: Deterministic Root Cause Extraction
//!
//! When a plan fails, the Failure Analyzer extracts the root cause from:
//! 1. Exit codes (exact match against known error codes)
//! 2. Stderr snippets (regex-based pattern matching)
//! 3. Command that failed (binary + args)
//!
//! **NO LLM calls.** Root cause extraction is purely deterministic.
//!
//! # Failure Pattern Matching
//!
//! Before executing a new plan, the Failure Analyzer checks the plan's
//! commands against known failure patterns. If a match is found, the
//! planner is warned and can adjust the plan.

mod types;
mod store;
mod patterns;

pub use types::{FailurePattern, FailureContext, RootCause};
pub use store::FailureAnalyzerStore;
pub use patterns::extract_root_cause;
