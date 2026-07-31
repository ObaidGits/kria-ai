//! Combined, CI-annotation-ready coverage report for the Memory Graph
//! Production Redesign spec (task F0.1 / 0.1.4).
//!
//! This module composes the three validators built in tasks 0.1.1–0.1.3 into a
//! single, stable, versioned JSON report:
//!
//! * the canonical [`Registry`](crate::memory_graph::Registry) (0.1.1) supplies
//!   count-by-kind totals,
//! * [`ForwardValidation`](crate::memory_graph::ForwardValidation) (0.1.2)
//!   contributes forward-mapping gaps and audit-occurrence defects, and
//! * [`IntegrityValidation`](crate::memory_graph::IntegrityValidation) (0.1.3)
//!   contributes reverse-orphan/duplicate/range/undefined/predecessor/status
//!   defects.
//!
//! Every defect is flattened into a uniform [`Diagnostic`] carrying the fields a
//! CI annotation needs — `severity`, `kind`, `id`, `file`, `line`, an optional
//! `category`, and a human-readable `reason`. Diagnostics are sorted by
//! `(severity, kind, id, file, line)` and the summary maps use `BTreeMap`, so
//! the serialized report is byte-stable and round-trips exactly.
//!
//! ## Scope boundary
//!
//! This task defines the schema and the serialization of a combined result. It
//! deliberately does **not** implement the documented coverage *command*, its
//! exit codes, or the exact `48/48`-style pass thresholds — those belong to task
//! 0.1.5, which consumes the [`CoverageReport`] value this module produces.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::forward::{AuditIssueKind, ForwardValidation};
use super::integrity::IntegrityValidation;
use super::registry::{Registry, RegistryError};

/// The stable schema identifier embedded in every serialized report. Bump the
/// version suffix on any breaking change to the field shape below.
pub const REPORT_SCHEMA_VERSION: &str = "memory-graph-coverage/v1";

/// The severity of a diagnostic. Every current defect fails the gate closed and
/// is therefore an [`Severity::Error`]; the enum leaves room for advisory
/// [`Severity::Warning`] annotations without a schema break.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// A fail-closed defect: the coverage gate must not pass while it stands.
    Error,
    /// A non-blocking advisory annotation.
    Warning,
}

/// One flattened, machine-readable diagnostic suitable for a CI annotation.
///
/// The `file`/`line`/`id`/`kind`/`reason`/`severity` fields are exactly the
/// coordinates a CI system needs to annotate a source location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Blocking (`error`) or advisory (`warning`).
    pub severity: Severity,
    /// Stable machine code for the defect class (e.g. `reverse_orphan`,
    /// `mapping_gap`, `audit_missing`, `duplicate_id`).
    pub kind: String,
    /// The offending identifier or reference token.
    pub id: String,
    /// Source document where the defect is observed (may be empty for defects
    /// that are the *absence* of an expected definition, e.g. a range gap).
    pub file: String,
    /// 1-based line number, or `0` when the defect is not line-bound.
    pub line: usize,
    /// Optional sub-classification (e.g. the missing forward-mapping category).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Deterministic human-readable diagnostic message.
    pub reason: String,
}

impl Diagnostic {
    /// Deterministic ordering key: `(severity, kind, id, file, line)`.
    fn sort_key(&self) -> (Severity, &str, &str, &str, usize) {
        (
            self.severity,
            self.kind.as_str(),
            self.id.as_str(),
            self.file.as_str(),
            self.line,
        )
    }
}

/// Aggregate totals describing the registry and the combined defect counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageSummary {
    /// Total number of parsed registry definitions.
    pub registry_total: usize,
    /// Count of registry definitions by kind code (sorted, stable).
    pub counts_by_kind: BTreeMap<String, usize>,
    /// Number of requirement/decision rows with a resolved forward mapping.
    pub forward_mappings: usize,
    /// Number of forward-mapping gaps.
    pub mapping_gaps: usize,
    /// Number of audit-ledger occurrence defects (missing + duplicate).
    pub audit_issues: usize,
    /// Number of structural integrity defects.
    pub integrity_issues: usize,
    /// Total diagnostics across all validators.
    pub total_diagnostics: usize,
    /// Count of diagnostics by kind code (sorted, stable).
    pub diagnostics_by_kind: BTreeMap<String, usize>,
    /// Whether the combined result is defect-free (no error diagnostics).
    pub ok: bool,
}

/// The complete, versioned, machine-readable coverage report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageReport {
    /// Stable schema identifier (see [`REPORT_SCHEMA_VERSION`]).
    pub schema_version: String,
    /// Aggregate totals.
    pub summary: CoverageSummary,
    /// All diagnostics, sorted by `(severity, kind, id, file, line)`.
    pub diagnostics: Vec<Diagnostic>,
}

impl CoverageReport {
    /// Build the registry and both validations from `spec_dir`, then compose the
    /// report. This is the convenience entry point the coverage command (0.1.5)
    /// can call directly.
    pub fn from_spec_dir(spec_dir: &Path) -> Result<Self, RegistryError> {
        let registry = Registry::from_spec_dir(spec_dir)?;
        let forward = ForwardValidation::from_registry(spec_dir, &registry)?;
        let integrity = IntegrityValidation::from_registry(spec_dir, &registry)?;
        Ok(Self::from_validations(&registry, &forward, &integrity))
    }

    /// Compose a report from already-computed validations.
    ///
    /// Keeping this separate from any I/O keeps the schema composable: callers
    /// (and 0.1.5) can validate once and render the report without re-reading
    /// the spec.
    pub fn from_validations(
        registry: &Registry,
        forward: &ForwardValidation,
        integrity: &IntegrityValidation,
    ) -> Self {
        let mut diagnostics = Vec::new();

        // Forward-mapping gaps.
        for gap in &forward.gaps {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                kind: "mapping_gap".to_string(),
                id: gap.id.clone(),
                file: gap.source_file.clone(),
                line: gap.line,
                category: gap.category.map(|c| c.code().to_string()),
                reason: gap.message.clone(),
            });
        }

        // Audit-ledger occurrence defects.
        for issue in &forward.audit_issues {
            let kind = match issue.issue {
                AuditIssueKind::Missing => "audit_missing",
                AuditIssueKind::Duplicate => "audit_duplicate",
            };
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                kind: kind.to_string(),
                id: issue.id.clone(),
                file: issue.source_file.clone(),
                line: issue.lines.first().copied().unwrap_or(0),
                category: None,
                reason: issue.message.clone(),
            });
        }

        // Structural integrity defects.
        for issue in &integrity.issues {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                kind: issue.kind.code().to_string(),
                id: issue.id.clone(),
                file: issue.source_file.clone(),
                line: issue.line,
                category: None,
                reason: issue.reason.clone(),
            });
        }

        diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

        let mut diagnostics_by_kind: BTreeMap<String, usize> = BTreeMap::new();
        for diag in &diagnostics {
            *diagnostics_by_kind.entry(diag.kind.clone()).or_insert(0) += 1;
        }

        let counts_by_kind = registry
            .counts_by_kind()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();

        let summary = CoverageSummary {
            registry_total: registry.len(),
            counts_by_kind,
            forward_mappings: forward.mappings.len(),
            mapping_gaps: forward.gaps.len(),
            audit_issues: forward.audit_issues.len(),
            integrity_issues: integrity.issues.len(),
            total_diagnostics: diagnostics.len(),
            diagnostics_by_kind,
            ok: diagnostics.is_empty(),
        };

        CoverageReport {
            schema_version: REPORT_SCHEMA_VERSION.to_string(),
            summary,
            diagnostics,
        }
    }

    /// Whether the report is defect-free.
    pub fn is_ok(&self) -> bool {
        self.summary.ok
    }

    /// Serialize to a stable, pretty-printed JSON string.
    pub fn to_json_pretty(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    /// Serialize to a stable, compact JSON string.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_graph::registry::Registry;
    use std::path::PathBuf;

    /// Locate the spec directory relative to this crate.
    fn spec_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.kiro/specs/memory-graph-production-redesign")
    }

    fn report() -> CoverageReport {
        CoverageReport::from_spec_dir(&spec_dir()).expect("report builds")
    }

    #[test]
    fn report_carries_schema_version() {
        assert_eq!(report().schema_version, REPORT_SCHEMA_VERSION);
        assert_eq!(REPORT_SCHEMA_VERSION, "memory-graph-coverage/v1");
    }

    #[test]
    fn summary_reflects_registry_totals() {
        let r = report();
        assert_eq!(r.summary.registry_total, 306);
        // 48 requirements + 46 decisions each have a resolved forward mapping.
        assert_eq!(r.summary.forward_mappings, 48 + 46);
        assert_eq!(r.summary.counts_by_kind.get("requirement"), Some(&48));
        assert_eq!(r.summary.counts_by_kind.get("decision"), Some(&46));
    }

    #[test]
    fn real_spec_report_is_clean_after_r_data_01_resolution() {
        let r = report();
        // F0.5.3 resolved the previously-known `R-DATA-01` undefined-code
        // defect (requirements.md MGR-019 now cites the defined risks
        // `R-WRONG-MERGE, R-POLICY-LEAK`). The report must now be clean.
        assert_eq!(r.summary.total_diagnostics, 0, "{:#?}", r.diagnostics);
        assert!(r.is_ok());
        assert!(
            !r.diagnostics.iter().any(|diag| diag.id == "R-DATA-01"),
            "R-DATA-01 must no longer appear in diagnostics"
        );
    }

    #[test]
    fn diagnostics_are_sorted_deterministically() {
        let r = report();
        let mut sorted = r.diagnostics.clone();
        sorted.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        assert_eq!(r.diagnostics, sorted);
    }

    #[test]
    fn json_serialization_is_stable_and_round_trips() {
        let r = report();
        let first = r.to_json_pretty().expect("serializes");
        let second = r.to_json_pretty().expect("serializes");
        // Byte-stable across repeated serialization.
        assert_eq!(first, second);
        // Schema version is present in the payload.
        assert!(first.contains("\"schema_version\""));
        assert!(first.contains("memory-graph-coverage/v1"));
        // Round-trips back to an identical value.
        let parsed: CoverageReport = serde_json::from_str(&first).expect("deserializes");
        assert_eq!(parsed, r);
        // And re-serializes to the identical bytes.
        assert_eq!(parsed.to_json_pretty().expect("re-serializes"), first);
    }

    #[test]
    fn empty_validations_produce_a_clean_report() {
        let registry = Registry::default();
        let forward = ForwardValidation::default();
        let integrity = IntegrityValidation::default();
        let r = CoverageReport::from_validations(&registry, &forward, &integrity);
        assert!(r.is_ok());
        assert_eq!(r.summary.total_diagnostics, 0);
        assert!(r.diagnostics.is_empty());
        assert_eq!(r.schema_version, REPORT_SCHEMA_VERSION);
    }
}
