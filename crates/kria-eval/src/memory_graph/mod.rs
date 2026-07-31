//! Memory Graph Production Redesign evidence tooling (spec `F0`).
//!
//! This module hosts the canonical evidence-ID registry and the coverage/orphan
//! linter. Task 0.1.1 introduces [`registry`], the single canonical parser that
//! normalizes every governed identifier in the spec into an in-memory registry
//! annotated with source file and line; tasks 0.1.2–0.1.3 add forward/integrity
//! validation, 0.1.4 the combined [`report`] schema, and 0.1.5 the documented,
//! read-only [`command`] (`CMD-MG-COVERAGE`) that emits totals and fails closed.

pub mod baseline;
pub mod catalog;
pub mod command;
pub mod fixtures;
pub mod forward;
pub mod integrity;
pub mod judged_eval;
pub mod manifest;
pub mod promotion;
pub mod registry;
pub mod report;
pub mod retrieval_eval;

pub use baseline::{
    BaselineEnvironment, HardwareFingerprint, PercentileSummary, ReferenceHardwareId,
    SampleProtocol, HARDWARE_ID_PREFIX, PERCENTILES, SAMPLE_ITERATIONS, UNKNOWN, WARMUP_ITERATIONS,
};
pub use catalog::{
    catalog, execute_step, find as find_catalog_command, repo_root as catalog_repo_root,
    Availability, CatalogCommand, CommandStep, DeclaredStatus, RunOutcome,
};
pub use command::{
    evaluate as evaluate_coverage, run as run_coverage, CommandRecord, CoverageOutcome,
    CoverageThresholds, CoverageTotals, RunConfig, COMMAND_ID, EXIT_GATE_FAILED,
    EXIT_INTERNAL_ERROR, EXIT_SUCCESS,
};
pub use fixtures::{
    FixtureGenerator, FixtureManifest, FixturePackage, LinkKind, MemoryMode, Policy, RecordKind,
    TruthState, FIXTURE_MANIFEST_SCHEMA, GENERATOR_VERSION,
};
pub use forward::{
    AuditIssueKind, AuditOccurrenceIssue, ForwardMapping, ForwardValidation, MappingCategory,
    MappingGap,
};
pub use integrity::{IntegrityIssue, IntegrityIssueKind, IntegrityValidation};
pub use manifest::{
    Accessibility, ArtifactReference, AssertionTotals, BuildEnvironment, CommandInvocation,
    Counterexample, EnvironmentState, EvidenceManifest, FixtureRef, Gate, GitProvenance,
    ManifestDiagnostic, ManifestDiagnosticKind, ManifestValidation, MeasurementProtocol,
    MetricSeries, ReferenceHardware, ReviewRecord, RunStatus, VersionSet, Waiver,
    MANIFEST_SCHEMA_VERSION,
};
pub use promotion::GatePromotion;
pub use registry::{IdKind, Registry, RegistryEntry, RegistryError};
pub use report::{CoverageReport, CoverageSummary, Diagnostic, Severity, REPORT_SCHEMA_VERSION};
pub use judged_eval::{
    bootstrap_ci, run_campaign, AblationResult, AssertionSummary, BootstrapCI, ClassBreakdown,
    ConfidenceIntervals, JudgedEvalResults, JudgmentProvenance, OverallMetrics, QueryResult,
    RegressionCheck, RetrievalQualityReport, StratumBreakdown, ThresholdRecord,
};
pub use retrieval_eval::{
    check_exclusion, compute_ndcg_at_k, compute_recall_at_k, evaluate_batch,
    evaluate_batch_with_thresholds, EvalThresholds, QueryClass, RetrievalEvalResult,
    RetrievalMetrics,
};
