//! Real-World Operational Workflow Cognition Eval Framework.
//!
//! ## Purpose
//!
//! This framework transforms KRIA evals from:
//! > "did the tool execute?"
//!
//! into:
//! > "did KRIA complete the real human workflow correctly,
//! >  visibly, semantically, and collaboratively?"
//!
//! ## Success Model
//!
//! Five independently-measured dimensions:
//!
//! ```text
//!  ┌──────────────────────────────────────────────────────────┐
//!  │  Observable   0.30  — user can SEE the result           │  highest weight
//!  │  Semantic     0.40  — user goal was actually achieved   │
//!  │  Workflow     0.15  — all stages completed              │
//!  │  Tool         0.10  — tool executed without error       │  lowest weight
//!  └──────────────────────────────────────────────────────────┘
//!  Collaborative  ±0.05  — orthogonal; recovery quality
//! ```
//!
//! ## Architecture
//!
//! | Component         | Location                | Responsibility                      |
//! |-------------------|-------------------------|-------------------------------------|
//! | Types             | `types`                 | Core data structures                |
//! | Contracts         | `contracts`             | Per-category semantic definitions   |
//! | Safety filter     | `safety_filter`         | Block dangerous operations          |
//! | Scorer            | `scoring`               | Map observations → success levels   |
//! | Failure analysis  | `failure_analysis`      | Diagnostic lineage capture          |
//! | Judge             | `judge`                 | Orchestrate all → verdict           |
//! | Report            | `report`                | Aggregate runs → final report       |
//! | Suites            | `suites`                | All workflow eval cases             |
//!
//! ## Design Invariants
//!
//! 1. **Fail-closed**: unknown/ambiguous cases are treated as failures.
//! 2. **No brittle choreography**: evals test semantic success, not pixel paths.
//! 3. **No fake demos**: all contracts require verifiable evidence.
//! 4. **Safe by default**: dangerous prompts blocked at `SafetyFilter`.
//! 5. **Preserves existing tests**: this framework is additive; it never
//!    weakens `GuiEvalJudge`, verifier authority, or HITL gates.

pub mod contracts;
pub mod failure_analysis;
pub mod judge;
pub mod report;
pub mod runner;
pub mod safety_filter;
pub mod scoring;
pub mod suites;
pub mod types;

pub use contracts::{
    browser_contract, coding_contract, coding_run_and_show_contract, contract_for_category,
    file_management_contract, human_expectation_contract, interruption_recovery_contract,
    multi_app_contract,
};
pub use judge::WorkflowCognitionJudge;
pub use report::{
    DimensionAggregates, ReadinessTier, WorkflowEvalReport, WorkflowEvalReportBuilder,
};
pub use safety_filter::SafetyFilter;
pub use scoring::WorkflowCognitionScorer;
pub use suites::{all_suites, auto_safe_suite, suite_manifest};
pub use types::{
    ArtifactFound, EvalWorkflowCategory, ExpectedRecovery, InterruptionKind, InterruptionScenario,
    ObservableOutputContract, SafetyClass, SemanticCompletionContract, WorkflowEvalCase,
    WorkflowEvalObservation, WorkflowEvalVerdict, WorkflowSuccessLevels, WorkflowVerdictKind,
};
