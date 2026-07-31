//! Documented, read-only coverage command for the Memory Graph Production
//! Redesign spec (task F0.1 / 0.1.5). Command ID: **`CMD-MG-COVERAGE`**.
//!
//! This module exposes the single, documented command that closes out task
//! F0.1. It composes the registry (0.1.1), forward validation (0.1.2),
//! integrity validation (0.1.3), and the combined report schema (0.1.4) into
//! one runnable entry point with a well-defined, *fail-closed* exit policy.
//!
//! ## Contract
//!
//! 1. **Writes no spec status.** The command is strictly read-only over the
//!    spec directory: it never touches `tasks.md` checkboxes or any normative
//!    spec document. It only *reads* the spec and *writes* machine-readable
//!    report artifacts to a caller-specified output directory (or to the
//!    supplied writer/stdout when no output directory is given). See
//!    [`run`] and the `writes_no_spec_files` test.
//! 2. **Emits totals.** Every invocation prints the coverage totals
//!    (requirements / decisions / findings / opportunities) and reverse-orphan
//!    count derived from the [`CoverageReport`].
//! 3. **Fail-closed exit policy.** The command exits `0` **only** when *all* of
//!    the following hold, and exits nonzero otherwise:
//!    * coverage is *exactly* requirements `48/48`, decisions `46/46`,
//!      findings `65/65`, opportunities `31/31`;
//!    * there are **zero** reverse-orphan diagnostics; and
//!    * there are **zero** error-severity diagnostics of any class.
//!
//!    The third clause is deliberately stricter than the literal task wording
//!    ("exact totals + zero reverse orphans"): a spec can hit the exact totals
//!    and have no reverse orphans yet still carry a fail-closed defect (e.g. an
//!    `undefined_code` reference). Per the F0.1 invariant "no best-effort
//!    pass", any error diagnostic must block the gate. Historically this policy
//!    made the command *fail* on the real spec because of the `R-DATA-01`
//!    undefined-code defect; task F0.5.3 resolved that defect, so the real spec
//!    now passes cleanly.
//!
//! ## Behavior on the current real spec
//!
//! Against the repository's current `memory-graph-production-redesign` spec the
//! command reports totals `48/48`, `46/46`, `65/65`, `31/31`, **zero** reverse
//! orphans, and **zero** error diagnostics, so the gate **passes** (exit `0`).
//! The previously-known `R-DATA-01` `undefined_code` defect was resolved in
//! task F0.5.3: `requirements.md` MGR-019 now references the defined risks
//! `R-WRONG-MERGE, R-POLICY-LEAK` (consistent with the `traceability.md`
//! ledger) instead of the never-defined `R-DATA-01`.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::registry::{IdKind, RegistryError};
use super::report::{CoverageReport, Severity};

/// The command ID this entry point implements (see `validation.md` §4).
pub const COMMAND_ID: &str = "CMD-MG-COVERAGE";

/// Process exit code returned when the coverage gate passes.
pub const EXIT_SUCCESS: i32 = 0;
/// Process exit code returned when the coverage gate fails closed.
pub const EXIT_GATE_FAILED: i32 = 1;
/// Process exit code returned when the command could not run (I/O/parse error).
pub const EXIT_INTERNAL_ERROR: i32 = 2;

/// The exact, non-negotiable coverage thresholds required for a passing gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CoverageThresholds {
    /// Required requirement count (`MGR-001..048`).
    pub requirements: usize,
    /// Required decision count (`MGD-001..046`).
    pub decisions: usize,
    /// Required finding count (`MG-C/H/M/L`, summed).
    pub findings: usize,
    /// Required opportunity count (`MG-O01..031`).
    pub opportunities: usize,
}

impl CoverageThresholds {
    /// The canonical F0.1 thresholds: `48/48`, `46/46`, `65/65`, `31/31`.
    pub const CANONICAL: CoverageThresholds = CoverageThresholds {
        requirements: 48,
        decisions: 46,
        findings: 65,
        opportunities: 31,
    };
}

impl Default for CoverageThresholds {
    fn default() -> Self {
        Self::CANONICAL
    }
}

/// The observed coverage totals derived from a [`CoverageReport`] summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CoverageTotals {
    /// Observed requirement definitions.
    pub requirements: usize,
    /// Observed decision definitions.
    pub decisions: usize,
    /// Observed finding definitions (all four severities summed).
    pub findings: usize,
    /// Observed opportunity definitions.
    pub opportunities: usize,
}

impl CoverageTotals {
    /// Extract the gated totals from a report summary's `counts_by_kind` map.
    pub fn from_report(report: &CoverageReport) -> Self {
        let count = |code: &str| {
            report
                .summary
                .counts_by_kind
                .get(code)
                .copied()
                .unwrap_or(0)
        };
        let findings = count(IdKind::FindingCritical.code())
            + count(IdKind::FindingHigh.code())
            + count(IdKind::FindingMedium.code())
            + count(IdKind::FindingLow.code());
        CoverageTotals {
            requirements: count(IdKind::Requirement.code()),
            decisions: count(IdKind::Decision.code()),
            findings,
            opportunities: count(IdKind::Opportunity.code()),
        }
    }

    /// Whether these totals *exactly* match the given thresholds.
    pub fn meets(&self, thresholds: &CoverageThresholds) -> bool {
        self.requirements == thresholds.requirements
            && self.decisions == thresholds.decisions
            && self.findings == thresholds.findings
            && self.opportunities == thresholds.opportunities
    }
}

/// The full outcome of evaluating a [`CoverageReport`] against the gate policy.
#[derive(Debug, Clone, Serialize)]
pub struct CoverageOutcome {
    /// The thresholds used for this evaluation.
    pub thresholds: CoverageThresholds,
    /// The observed coverage totals.
    pub totals: CoverageTotals,
    /// Whether the totals exactly meet the thresholds.
    pub thresholds_met: bool,
    /// Number of `reverse_orphan` diagnostics observed.
    pub reverse_orphans: usize,
    /// Number of error-severity diagnostics of any class.
    pub error_diagnostics: usize,
    /// Whether the gate passed (see module-level exit policy).
    pub passed: bool,
    /// The process exit code the command should return.
    pub exit_code: i32,
    /// Deterministic, human-readable reasons the gate failed (empty on pass).
    pub failures: Vec<String>,
}

/// The stable machine code for reverse-orphan diagnostics in the report schema.
const REVERSE_ORPHAN_KIND: &str = "reverse_orphan";

/// Evaluate a [`CoverageReport`] against the canonical thresholds and the
/// fail-closed exit policy. Pure: performs no I/O and is fully deterministic.
pub fn evaluate(report: &CoverageReport) -> CoverageOutcome {
    evaluate_with(report, &CoverageThresholds::CANONICAL)
}

/// Evaluate a report against explicit thresholds (used by tests).
pub fn evaluate_with(report: &CoverageReport, thresholds: &CoverageThresholds) -> CoverageOutcome {
    let totals = CoverageTotals::from_report(report);
    let thresholds_met = totals.meets(thresholds);

    let reverse_orphans = report
        .summary
        .diagnostics_by_kind
        .get(REVERSE_ORPHAN_KIND)
        .copied()
        .unwrap_or(0);
    let error_diagnostics = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();

    let mut failures = Vec::new();
    if totals.requirements != thresholds.requirements {
        failures.push(format!(
            "requirements coverage {}/{} (expected exactly {})",
            totals.requirements, thresholds.requirements, thresholds.requirements
        ));
    }
    if totals.decisions != thresholds.decisions {
        failures.push(format!(
            "decisions coverage {}/{} (expected exactly {})",
            totals.decisions, thresholds.decisions, thresholds.decisions
        ));
    }
    if totals.findings != thresholds.findings {
        failures.push(format!(
            "findings coverage {}/{} (expected exactly {})",
            totals.findings, thresholds.findings, thresholds.findings
        ));
    }
    if totals.opportunities != thresholds.opportunities {
        failures.push(format!(
            "opportunities coverage {}/{} (expected exactly {})",
            totals.opportunities, thresholds.opportunities, thresholds.opportunities
        ));
    }
    if reverse_orphans > 0 {
        failures.push(format!(
            "{reverse_orphans} reverse-orphan diagnostic(s) present (expected zero)"
        ));
    }
    // Fail-closed: any remaining error-severity defect (e.g. undefined_code)
    // blocks the gate even when totals + reverse orphans are clean.
    let non_orphan_errors = error_diagnostics.saturating_sub(reverse_orphans);
    if non_orphan_errors > 0 {
        failures.push(format!(
            "{non_orphan_errors} other error-severity diagnostic(s) present (fail-closed; expected zero)"
        ));
    }

    let passed = failures.is_empty();
    CoverageOutcome {
        thresholds: *thresholds,
        totals,
        thresholds_met,
        reverse_orphans,
        error_diagnostics,
        passed,
        exit_code: if passed {
            EXIT_SUCCESS
        } else {
            EXIT_GATE_FAILED
        },
        failures,
    }
}

/// Runtime configuration for one command invocation.
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// The spec directory to lint (read-only).
    pub spec_dir: PathBuf,
    /// Optional evidence output directory. When set, the command writes the
    /// report artifacts under it. It MUST NOT be a spec document path.
    pub out_dir: Option<PathBuf>,
    /// Run identifier recorded in the command evidence record.
    pub run_id: String,
    /// Suppress the human-readable totals banner (JSON-only mode).
    pub quiet: bool,
}

impl RunConfig {
    /// The default spec directory, resolved relative to this crate.
    pub fn default_spec_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.kiro/specs/memory-graph-production-redesign")
    }

    /// A config that lints the default spec dir and writes nothing but stdout.
    pub fn for_default_spec(run_id: impl Into<String>) -> Self {
        RunConfig {
            spec_dir: Self::default_spec_dir(),
            out_dir: None,
            run_id: run_id.into(),
            quiet: false,
        }
    }
}

/// The command evidence record written to `commands/CMD-MG-COVERAGE.json`.
#[derive(Debug, Clone, Serialize)]
pub struct CommandRecord {
    /// Always [`COMMAND_ID`].
    pub command_id: &'static str,
    /// Report schema this command emits.
    pub schema_version: String,
    /// The run identifier.
    pub run_id: String,
    /// The spec directory that was linted (display form).
    pub spec_dir: String,
    /// The exit code the command returned.
    pub exit_code: i32,
    /// Whether the gate passed.
    pub passed: bool,
    /// The evaluated outcome (totals, thresholds, failures).
    pub outcome: CoverageOutcome,
}

/// Run the coverage command: build the report, evaluate the gate, emit totals
/// to `writer`, and (when `out_dir` is set) write the machine-readable report
/// artifacts. Returns the [`CoverageOutcome`] carrying the process exit code.
///
/// This function performs **no writes to the spec directory** — it only reads
/// spec documents and writes to `config.out_dir` when provided.
pub fn run<W: Write>(config: &RunConfig, writer: &mut W) -> Result<CoverageOutcome, RegistryError> {
    let report = CoverageReport::from_spec_dir(&config.spec_dir)?;
    let outcome = evaluate(&report);

    if !config.quiet {
        write_totals_banner(writer, config, &report, &outcome);
    }

    if let Some(out_dir) = &config.out_dir {
        write_artifacts(out_dir, config, &report, &outcome)?;
        if !config.quiet {
            let _ = writeln!(
                writer,
                "Report artifacts written under: {}",
                out_dir.display()
            );
        }
    } else {
        // No evidence dir: emit the machine-readable report to the writer so
        // the JSON is never lost and the command remains scriptable.
        let json = report.to_json_pretty().unwrap_or_else(|_| "{}".to_string());
        let _ = writeln!(writer, "{json}");
    }

    Ok(outcome)
}

/// Print the human-readable totals + verdict banner.
fn write_totals_banner<W: Write>(
    writer: &mut W,
    config: &RunConfig,
    report: &CoverageReport,
    outcome: &CoverageOutcome,
) {
    let t = &outcome.totals;
    let th = &outcome.thresholds;
    let _ = writeln!(writer, "{COMMAND_ID} — Memory Graph coverage/orphan gate");
    let _ = writeln!(writer, "spec: {}", config.spec_dir.display());
    let _ = writeln!(writer, "schema: {}", report.schema_version);
    let _ = writeln!(
        writer,
        "requirements: {}/{}  decisions: {}/{}  findings: {}/{}  opportunities: {}/{}",
        t.requirements,
        th.requirements,
        t.decisions,
        th.decisions,
        t.findings,
        th.findings,
        t.opportunities,
        th.opportunities
    );
    let _ = writeln!(
        writer,
        "reverse orphans: {}   total diagnostics: {}   error diagnostics: {}",
        outcome.reverse_orphans, report.summary.total_diagnostics, outcome.error_diagnostics
    );
    if outcome.passed {
        let _ = writeln!(writer, "verdict: PASS (exit {})", outcome.exit_code);
    } else {
        let _ = writeln!(writer, "verdict: FAIL (exit {})", outcome.exit_code);
        for reason in &outcome.failures {
            let _ = writeln!(writer, "  - {reason}");
        }
    }
}

/// Write the four report artifacts under `out_dir`.
///
/// Layout mirrors the F0.1 evidence contract:
/// `reports/{id-inventory,coverage,reverse-orphans}.json` and
/// `commands/CMD-MG-COVERAGE.json`.
fn write_artifacts(
    out_dir: &Path,
    config: &RunConfig,
    report: &CoverageReport,
    outcome: &CoverageOutcome,
) -> Result<(), RegistryError> {
    let reports_dir = out_dir.join("reports");
    let commands_dir = out_dir.join("commands");
    create_dir(&reports_dir)?;
    create_dir(&commands_dir)?;

    // id-inventory: the registry counts-by-kind and gated totals.
    let inventory = serde_json::json!({
        "schema_version": report.schema_version,
        "registry_total": report.summary.registry_total,
        "counts_by_kind": report.summary.counts_by_kind,
        "totals": outcome.totals,
        "thresholds": outcome.thresholds,
    });
    write_json(&reports_dir.join("id-inventory.json"), &inventory)?;

    // coverage: the full combined report.
    write_json(&reports_dir.join("coverage.json"), report)?;

    // reverse-orphans: just the reverse-orphan diagnostics.
    let orphans: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.kind == REVERSE_ORPHAN_KIND)
        .collect();
    let orphan_doc = serde_json::json!({
        "schema_version": report.schema_version,
        "reverse_orphans": outcome.reverse_orphans,
        "diagnostics": orphans,
    });
    write_json(&reports_dir.join("reverse-orphans.json"), &orphan_doc)?;

    // command record.
    let record = CommandRecord {
        command_id: COMMAND_ID,
        schema_version: report.schema_version.clone(),
        run_id: config.run_id.clone(),
        spec_dir: config.spec_dir.display().to_string(),
        exit_code: outcome.exit_code,
        passed: outcome.passed,
        outcome: outcome.clone(),
    };
    write_json(&commands_dir.join(format!("{COMMAND_ID}.json")), &record)?;

    Ok(())
}

/// Create a directory (and parents), mapping errors into [`RegistryError`].
fn create_dir(path: &Path) -> Result<(), RegistryError> {
    std::fs::create_dir_all(path).map_err(|source| RegistryError::ReadFailed {
        path: path.display().to_string(),
        source,
    })
}

/// Serialize `value` as stable pretty JSON to `path`.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), RegistryError> {
    let json = serde_json::to_string_pretty(value).map_err(|e| RegistryError::ReadFailed {
        path: path.display().to_string(),
        source: std::io::Error::other(e),
    })?;
    std::fs::write(path, json).map_err(|source| RegistryError::ReadFailed {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_graph::forward::ForwardValidation;
    use crate::memory_graph::integrity::{IntegrityIssue, IntegrityIssueKind, IntegrityValidation};
    use crate::memory_graph::registry::{Registry, RegistryEntry};
    use std::path::PathBuf;

    fn spec_dir() -> PathBuf {
        RunConfig::default_spec_dir()
    }

    fn entry(id: &str, kind: IdKind) -> RegistryEntry {
        RegistryEntry {
            id: id.to_string(),
            kind,
            source_file: "synthetic.md".to_string(),
            line: 1,
            title: String::new(),
        }
    }

    /// Build a synthetic registry that hits exactly the canonical totals.
    fn full_registry() -> Registry {
        let mut entries = Vec::new();
        for n in 1..=48 {
            entries.push(entry(&format!("MGR-{n:03}"), IdKind::Requirement));
        }
        for n in 1..=46 {
            entries.push(entry(&format!("MGD-{n:03}"), IdKind::Decision));
        }
        // 65 findings split across the four severities (7+17+28+13).
        for n in 1..=7 {
            entries.push(entry(&format!("MG-C{n:02}"), IdKind::FindingCritical));
        }
        for n in 1..=17 {
            entries.push(entry(&format!("MG-H{n:02}"), IdKind::FindingHigh));
        }
        for n in 1..=28 {
            entries.push(entry(&format!("MG-M{n:02}"), IdKind::FindingMedium));
        }
        for n in 1..=13 {
            entries.push(entry(&format!("MG-L{n:02}"), IdKind::FindingLow));
        }
        for n in 1..=31 {
            entries.push(entry(&format!("MG-O{n:02}"), IdKind::Opportunity));
        }
        Registry { entries }
    }

    fn report_from(registry: &Registry, integrity: &IntegrityValidation) -> CoverageReport {
        CoverageReport::from_validations(registry, &ForwardValidation::default(), integrity)
    }

    #[test]
    fn passes_only_on_exact_totals_and_zero_orphans() {
        let report = report_from(&full_registry(), &IntegrityValidation::default());
        let outcome = evaluate(&report);
        assert!(outcome.passed, "failures: {:?}", outcome.failures);
        assert_eq!(outcome.exit_code, EXIT_SUCCESS);
        assert!(outcome.thresholds_met);
        assert_eq!(outcome.reverse_orphans, 0);
        assert_eq!(outcome.error_diagnostics, 0);
        assert_eq!(outcome.totals.requirements, 48);
        assert_eq!(outcome.totals.decisions, 46);
        assert_eq!(outcome.totals.findings, 65);
        assert_eq!(outcome.totals.opportunities, 31);
    }

    #[test]
    fn fails_on_synthetic_missing_requirement() {
        let mut registry = full_registry();
        // Drop one requirement -> 47/48.
        let idx = registry
            .entries
            .iter()
            .position(|e| e.id == "MGR-048")
            .expect("has MGR-048");
        registry.entries.remove(idx);

        let outcome = evaluate(&report_from(&registry, &IntegrityValidation::default()));
        assert!(!outcome.passed);
        assert_eq!(outcome.exit_code, EXIT_GATE_FAILED);
        assert!(!outcome.thresholds_met);
        assert_eq!(outcome.totals.requirements, 47);
        assert!(outcome
            .failures
            .iter()
            .any(|f| f.contains("requirements coverage 47/48")));
    }

    #[test]
    fn fails_on_reverse_orphan() {
        let integrity = IntegrityValidation {
            issues: vec![IntegrityIssue {
                kind: IntegrityIssueKind::ReverseOrphan,
                id: "V-GHOST-01".to_string(),
                source_file: "validation.md".to_string(),
                line: 42,
                reason: "defined but governed by no MGR/MGD ledger row".to_string(),
            }],
        };
        let outcome = evaluate(&report_from(&full_registry(), &integrity));
        assert!(!outcome.passed);
        assert_eq!(outcome.exit_code, EXIT_GATE_FAILED);
        // Totals are still exact; the orphan is what fails the gate.
        assert!(outcome.thresholds_met);
        assert_eq!(outcome.reverse_orphans, 1);
        assert!(outcome
            .failures
            .iter()
            .any(|f| f.contains("reverse-orphan")));
    }

    #[test]
    fn fails_closed_on_non_orphan_error_even_with_exact_totals() {
        let integrity = IntegrityValidation {
            issues: vec![IntegrityIssue {
                kind: IntegrityIssueKind::UndefinedCode,
                id: "R-GHOST-01".to_string(),
                source_file: "requirements.md".to_string(),
                line: 10,
                reason: "referenced but has no canonical definition".to_string(),
            }],
        };
        let outcome = evaluate(&report_from(&full_registry(), &integrity));
        assert!(!outcome.passed);
        assert_eq!(outcome.exit_code, EXIT_GATE_FAILED);
        assert!(outcome.thresholds_met);
        assert_eq!(outcome.reverse_orphans, 0);
        assert_eq!(outcome.error_diagnostics, 1);
        assert!(outcome
            .failures
            .iter()
            .any(|f| f.contains("other error-severity")));
    }

    #[test]
    fn real_spec_passes_gate_after_r_data_01_resolution() {
        // F0.5.3 resolved the previously-known `R-DATA-01` undefined-code
        // defect (requirements.md MGR-019 now cites the defined risks
        // `R-WRONG-MERGE, R-POLICY-LEAK`). The real spec must now pass the
        // gate: exact totals, zero reverse orphans, and zero error diagnostics.
        let report = CoverageReport::from_spec_dir(&spec_dir()).expect("report builds");
        let outcome = evaluate(&report);
        assert!(outcome.thresholds_met, "totals: {:?}", outcome.totals);
        assert_eq!(outcome.totals.requirements, 48);
        assert_eq!(outcome.totals.decisions, 46);
        assert_eq!(outcome.totals.findings, 65);
        assert_eq!(outcome.totals.opportunities, 31);
        assert_eq!(outcome.reverse_orphans, 0);
        assert_eq!(
            outcome.error_diagnostics, 0,
            "failures: {:?}",
            outcome.failures
        );
        assert!(
            outcome.passed,
            "expected clean pass: {:?}",
            outcome.failures
        );
        assert_eq!(outcome.exit_code, EXIT_SUCCESS);
    }

    #[test]
    fn run_emits_totals_and_returns_outcome() {
        let config = RunConfig {
            spec_dir: spec_dir(),
            out_dir: None,
            run_id: "test-run".to_string(),
            quiet: false,
        };
        let mut buf: Vec<u8> = Vec::new();
        let outcome = run(&config, &mut buf).expect("runs");
        let text = String::from_utf8(buf).expect("utf8");
        assert!(text.contains("requirements: 48/48"));
        assert!(text.contains("decisions: 46/46"));
        assert!(text.contains("findings: 65/65"));
        assert!(text.contains("opportunities: 31/31"));
        assert!(text.contains(COMMAND_ID));
        // Real spec is clean after F0.5.3 resolved R-DATA-01: gate passes.
        assert_eq!(outcome.exit_code, EXIT_SUCCESS);
    }

    #[test]
    fn writes_no_spec_files() {
        use std::collections::BTreeMap;

        // Snapshot every spec file's bytes before running.
        let dir = spec_dir();
        let mut before: BTreeMap<PathBuf, Vec<u8>> = BTreeMap::new();
        for entry in std::fs::read_dir(&dir).expect("read spec dir") {
            let path = entry.expect("entry").path();
            if path.is_file() {
                before.insert(path.clone(), std::fs::read(&path).expect("read file"));
            }
        }

        // Run with the evidence out-dir pointed OUTSIDE the spec directory.
        let out = tempfile::tempdir().expect("tempdir");
        let config = RunConfig {
            spec_dir: dir.clone(),
            out_dir: Some(out.path().to_path_buf()),
            run_id: "no-write-test".to_string(),
            quiet: true,
        };
        let mut sink: Vec<u8> = Vec::new();
        let _ = run(&config, &mut sink).expect("runs");

        // Every spec file is byte-for-byte unchanged.
        for (path, bytes) in &before {
            let now = std::fs::read(path).expect("re-read file");
            assert_eq!(&now, bytes, "spec file mutated: {}", path.display());
        }

        // The artifacts landed in the out-dir, not the spec dir.
        assert!(out.path().join("reports/coverage.json").is_file());
        assert!(out.path().join("reports/id-inventory.json").is_file());
        assert!(out.path().join("reports/reverse-orphans.json").is_file());
        assert!(out
            .path()
            .join(format!("commands/{COMMAND_ID}.json"))
            .is_file());
    }
}
