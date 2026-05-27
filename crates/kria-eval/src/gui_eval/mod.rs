//! Production-grade GUI Automation Evaluation Framework
//!
//! This module validates KRIA's GUI execution cognition, workflow intelligence,
//! application-state reasoning, runtime orchestration, verification, recovery,
//! and truthful execution reporting.
//!
//! ## Architecture
//!
//! The eval framework is organized into:
//! - [`types`]: Core data structures for GUI eval cases, observations, verdicts
//! - [`runner`]: Executes GUI eval cases through the real substrate pipeline
//! - [`judge`]: Deterministic + structural verdict engine (no LLM required)
//! - [`report`]: Structured report generation with root-cause analysis
//! - [`suites`]: Categorized test suites covering all failure modes
//! - [`lifecycle`]: App lifecycle helpers (running-app detection, session reuse)
//!
//! ## Design Principles
//!
//! 1. **No fake success**: Every verdict is backed by a verifiable artifact or
//!    explicit evidence string. The framework never claims PASS without proof.
//! 2. **Substrate-aware**: Tests understand the difference between FileWriteThenOpen,
//!    AppOpenOnly, BrowserNavigate, and Keystroke substrates.
//! 3. **Wayland/X11 aware**: Tests are tagged with their display-server requirements
//!    and skip gracefully when the required backend is unavailable.
//! 4. **Architectural diagnostics**: Failures are classified into root-cause
//!    categories (parsing, resolution, verification, lifecycle, retrieval-leak, etc.)
//!    so the report becomes an actionable blueprint.

pub mod az_suite;
pub mod chaos_suite;
pub mod destructive_safety;
pub mod expanded_gui_evals;
pub mod governance;
pub mod gui_cognition_suite;
pub mod gui_hardening;
pub mod hitl_timeline;
pub mod invariants;
pub mod judge;
pub mod lifecycle;
pub mod llm_cognition_matrix;
pub mod matrix;
pub mod observability;
pub mod observability_score;
pub mod production_gui_workflows;
pub mod readiness_summary;
pub mod report;
pub mod runner;
pub mod suites;
pub mod types;
pub mod workflow_fidelity;

pub use judge::GuiEvalJudge;
pub use report::{GuiEvalReport, GuiEvalReportBuilder};
pub use runner::GuiEvalRunner;
pub use types::{
    DisplayServerRequirement, FailureCategory, GuiEvalCase, GuiEvalObservation, GuiEvalVerdict,
    GuiEvalVerdictKind, GuiWorkflowTrace,
};
