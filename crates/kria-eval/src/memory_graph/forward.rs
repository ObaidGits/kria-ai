//! Forward-mapping validation for the Memory Graph Production Redesign spec
//! (task F0.1 / 0.1.2).
//!
//! This module builds on the canonical [`Registry`](crate::memory_graph::Registry)
//! (task 0.1.1) and validates the *forward* half of the traceability contract:
//!
//! 1. **Every requirement (`MGR-*`) and decision (`MGD-*`) maps forward** to all
//!    six governed target categories — a design section, a workstream, a
//!    validation suite, a principal risk, a gate, and an evidence artifact class
//!    — exactly as `traceability.md` §2/§3 define them. A requirement or
//!    decision whose ledger row is missing, or whose row omits any one of the
//!    six categories, is a **mapping gap**.
//! 2. **Every audit finding (`MG-C/H/M/L`) and opportunity (`MG-O`) occurs
//!    exactly once** in the audit ledger (`traceability.md` §4/§5) — no missing
//!    occurrence (a gap in the contiguous per-severity numbering) and no
//!    duplicate occurrence (the same ID laid down on two ledger rows).
//!
//! Diagnostics are deterministic and machine-readable (file/line/ID/category),
//! sorted by ID then category, so the coverage command (task 0.1.5) and CI
//! annotations can consume them stably.
//!
//! ## Scope boundary
//!
//! This task validates *presence and parseability* of the forward mappings as
//! `traceability.md` declares them. It intentionally does **not** verify that a
//! referenced code is *defined* elsewhere (undefined suite/risk/workstream/
//! artifact codes), reverse orphans, duplicate/out-of-range definitions,
//! predecessor-gate gaps, or status-manifest rules — those belong to task
//! 0.1.3. For example, `requirements.md` historically referenced `R-DATA-01`,
//! which had no definition in `risk-analysis.md`; because that reference never
//! appeared in the `traceability.md` ledger rows, it did not affect
//! forward-mapping validation and was left for the reverse-orphan checks in
//! 0.1.3 (task F0.5.3 has since resolved it — MGR-019 now cites the defined
//! `R-WRONG-MERGE, R-POLICY-LEAK`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::registry::{self, IdKind, Registry, RegistryError};

/// The six governed forward-mapping target categories every requirement and
/// decision row must populate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MappingCategory {
    /// A `design.md` section reference (e.g. `§§2,9,11`).
    Design,
    /// A workstream (`W-*`), possibly via the ledger's short code or `all`.
    Workstream,
    /// A validation suite (`V-*`) or a named lint/inventory validation activity.
    Suite,
    /// A principal risk (`R-*`).
    Risk,
    /// A backend-first gate (`F0`..`F6`), possibly a range or list.
    Gate,
    /// An evidence artifact class (`A-*`), possibly `all artifact classes`.
    ArtifactClass,
}

impl MappingCategory {
    /// A short, stable machine code for this category, used in diagnostics.
    pub fn code(self) -> &'static str {
        match self {
            MappingCategory::Design => "design",
            MappingCategory::Workstream => "workstream",
            MappingCategory::Suite => "suite",
            MappingCategory::Risk => "risk",
            MappingCategory::Gate => "gate",
            MappingCategory::ArtifactClass => "artifact_class",
        }
    }

    /// All six categories in canonical order.
    pub fn all() -> [MappingCategory; 6] {
        [
            MappingCategory::Design,
            MappingCategory::Workstream,
            MappingCategory::Suite,
            MappingCategory::Risk,
            MappingCategory::Gate,
            MappingCategory::ArtifactClass,
        ]
    }
}

/// The resolved forward mapping of one requirement or decision to each of the
/// six governed target categories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ForwardMapping {
    /// Canonical identifier (`MGR-001`, `MGD-023`).
    pub id: String,
    /// Whether this is a requirement or a decision.
    pub kind: IdKind,
    /// Source document (`traceability.md`).
    pub source_file: String,
    /// 1-based line number of the ledger row.
    pub line: usize,
    /// Referenced design sections (`design.md` section tokens).
    pub design_sections: Vec<String>,
    /// Canonical workstreams (`W-*`) the row maps to.
    pub workstreams: Vec<String>,
    /// Validation suite / activity tokens the row maps to.
    pub suites: Vec<String>,
    /// Principal risks (`R-*`) the row maps to.
    pub risks: Vec<String>,
    /// Gates (`F0`..`F6`) the row maps to, ranges expanded.
    pub gates: Vec<String>,
    /// Evidence artifact classes (`A-*`) the row maps to.
    pub artifact_classes: Vec<String>,
}

impl ForwardMapping {
    /// The resolved targets for a given category.
    pub fn targets(&self, category: MappingCategory) -> &[String] {
        match category {
            MappingCategory::Design => &self.design_sections,
            MappingCategory::Workstream => &self.workstreams,
            MappingCategory::Suite => &self.suites,
            MappingCategory::Risk => &self.risks,
            MappingCategory::Gate => &self.gates,
            MappingCategory::ArtifactClass => &self.artifact_classes,
        }
    }
}

/// A forward-mapping gap: a requirement/decision that is absent from the ledger
/// or is missing one of the six governed target categories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MappingGap {
    /// The requirement/decision ID with the gap.
    pub id: String,
    /// Whether the ID is a requirement or decision.
    pub kind: IdKind,
    /// Source document (`traceability.md`).
    pub source_file: String,
    /// 1-based line number of the ledger row, or `0` when the row is absent.
    pub line: usize,
    /// The missing category, or `None` when the entire ledger row is absent.
    pub category: Option<MappingCategory>,
    /// Deterministic human-readable diagnostic message.
    pub message: String,
}

/// The kind of audit-ledger occurrence defect detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditIssueKind {
    /// An expected finding/opportunity ID (within contiguous numbering) has no
    /// ledger occurrence.
    Missing,
    /// A finding/opportunity ID occurs on more than one ledger row.
    Duplicate,
}

/// A defect in the single-occurrence guarantee for the audit ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditOccurrenceIssue {
    /// The finding/opportunity ID.
    pub id: String,
    /// The finding/opportunity kind (severity or opportunity).
    pub kind: IdKind,
    /// Source document (`traceability.md`), empty for a missing ID.
    pub source_file: String,
    /// All ledger line numbers where the ID occurs (empty when missing).
    pub lines: Vec<usize>,
    /// Whether the ID is missing or duplicated.
    pub issue: AuditIssueKind,
    /// Deterministic human-readable diagnostic message.
    pub message: String,
}

/// The complete result of forward-mapping validation.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ForwardValidation {
    /// One entry per requirement/decision that has a ledger row.
    pub mappings: Vec<ForwardMapping>,
    /// All forward-mapping gaps, sorted by `(id, category)`.
    pub gaps: Vec<MappingGap>,
    /// All audit-ledger single-occurrence defects, sorted by `(id, issue)`.
    pub audit_issues: Vec<AuditOccurrenceIssue>,
}

impl ForwardValidation {
    /// Whether validation found no gaps and no audit-occurrence defects.
    pub fn is_ok(&self) -> bool {
        self.gaps.is_empty() && self.audit_issues.is_empty()
    }

    /// Build the registry from `spec_dir` and validate forward mappings.
    pub fn from_spec_dir(spec_dir: &Path) -> Result<Self, RegistryError> {
        let registry = Registry::from_spec_dir(spec_dir)?;
        Self::from_registry(spec_dir, &registry)
    }

    /// Validate forward mappings against an already-built [`Registry`].
    ///
    /// Reads `traceability.md` for the ledger mapping columns; the registry
    /// supplies the canonical requirement/decision/finding/opportunity IDs and
    /// the expansion sets for `all` markers.
    pub fn from_registry(spec_dir: &Path, registry: &Registry) -> Result<Self, RegistryError> {
        let file_name = "traceability.md";
        let path: PathBuf = spec_dir.join(file_name);
        let content =
            std::fs::read_to_string(&path).map_err(|source| RegistryError::ReadFailed {
                path: path.display().to_string(),
                source,
            })?;

        Ok(validate(registry, file_name, &content))
    }
}

/// Core forward-mapping validation over already-loaded ledger `content`.
///
/// Split out from [`ForwardValidation::from_registry`] so tests can exercise it
/// with synthetic registries and ledger text without touching the filesystem.
fn validate(registry: &Registry, file_name: &str, content: &str) -> ForwardValidation {
    let rows = parse_ledger_rows(file_name, content);
    let ctx = ExpansionContext::from_registry(registry);

    let mut mappings = Vec::new();
    let mut gaps = Vec::new();

    // Validate every requirement then every decision, in registry order so
    // diagnostics are deterministic and tied to canonical definitions.
    for kind in [IdKind::Requirement, IdKind::Decision] {
        for entry in registry.entries_of_kind(kind) {
            match rows.get(entry.id.as_str()) {
                None => gaps.push(MappingGap {
                    id: entry.id.clone(),
                    kind,
                    source_file: file_name.to_string(),
                    line: 0,
                    category: None,
                    message: format!(
                        "{} has no forward-mapping row in the {} ledger",
                        entry.id, file_name
                    ),
                }),
                Some(row) => {
                    let mapping = resolve_mapping(entry.id.as_str(), kind, row, &ctx);
                    for category in MappingCategory::all() {
                        if mapping.targets(category).is_empty() {
                            gaps.push(MappingGap {
                                id: entry.id.clone(),
                                kind,
                                source_file: file_name.to_string(),
                                line: row.line,
                                category: Some(category),
                                message: format!(
                                    "{} maps to no {} in its {} ledger row",
                                    entry.id,
                                    category.code(),
                                    file_name
                                ),
                            });
                        }
                    }
                    mappings.push(mapping);
                }
            }
        }
    }

    gaps.sort_by(|a, b| {
        a.id.cmp(&b.id).then(
            a.category
                .map(|c| c.code())
                .cmp(&b.category.map(|c| c.code())),
        )
    });

    let audit_issues = audit_occurrence_issues(registry, file_name);

    ForwardValidation {
        mappings,
        gaps,
        audit_issues,
    }
}

// ---------------------------------------------------------------------------
// Ledger row parsing (traceability.md §2 Requirement, §3 Decision)
// ---------------------------------------------------------------------------

/// One raw ledger row keyed by requirement/decision ID.
#[derive(Debug, Clone)]
struct LedgerRow {
    line: usize,
    design: String,
    work: String,
    validation: String,
    risks: String,
    gates: String,
    evidence: String,
}

/// Parse the requirement and decision ledger tables into raw rows keyed by ID.
///
/// Both tables share the same 8-column shape:
/// `| ID+desc | Design | Work | Validation | Risks | Gate(s) | Evidence | Status |`.
/// A row is recognized when the first cell's first whitespace-delimited token is
/// an `MGR-0NN` or `MGD-0NN` identifier.
fn parse_ledger_rows(file: &str, content: &str) -> BTreeMap<String, LedgerRow> {
    let _ = file;
    let mut rows = BTreeMap::new();
    for (line_no, line) in registry::numbered_lines(content) {
        let Some(cells) = registry::table_cells(line) else {
            continue;
        };
        // Need through the Evidence column (index 6).
        if cells.len() < 7 {
            continue;
        }
        let Some(first) = cells.first() else {
            continue;
        };
        let id_token = first.split_whitespace().next().unwrap_or("");
        let is_req = registry::matches_prefix_num(id_token, "MGR-", 3);
        let is_dec = registry::matches_prefix_num(id_token, "MGD-", 3);
        if !is_req && !is_dec {
            continue;
        }
        rows.insert(
            id_token.to_string(),
            LedgerRow {
                line: line_no,
                design: cells[1].to_string(),
                work: cells[2].to_string(),
                validation: cells[3].to_string(),
                risks: cells[4].to_string(),
                gates: cells[5].to_string(),
                evidence: cells[6].to_string(),
            },
        );
    }
    rows
}

// ---------------------------------------------------------------------------
// Expansion context: workstream aliases and `all` expansion sets
// ---------------------------------------------------------------------------

/// Resolution/expansion data derived from the registry: the short-code →
/// canonical `W-*` alias map, and the full suite/workstream/artifact sets used
/// to expand `all` markers.
struct ExpansionContext {
    workstream_alias: BTreeMap<&'static str, &'static str>,
    all_workstreams: Vec<String>,
    all_suites: Vec<String>,
    all_artifact_classes: Vec<String>,
}

impl ExpansionContext {
    fn from_registry(registry: &Registry) -> Self {
        // The ledger tables use short workstream codes; the canonical `W-*` IDs
        // are defined in implementation-roadmap.md §2. This alias map (declared
        // in traceability.md §1) resolves the short codes to canonical IDs.
        let workstream_alias = BTreeMap::from([
            ("WE", "W-EVIDENCE"),
            ("WA", "W-AUTH"),
            ("WS", "W-SEC"),
            ("WL", "W-LIFE"),
            ("WM", "W-SEM"),
            ("WR", "W-RET"),
            ("WC", "W-COG"),
            ("WP", "W-API"),
            ("WH", "W-HUMAN"),
            ("W2", "W-2D"),
            ("WX", "W-RELEASE"),
            ("W3", "W-3D"),
        ]);

        let all_workstreams = registry
            .ids_of_kind(IdKind::Workstream)
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let all_suites = registry
            .ids_of_kind(IdKind::Suite)
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let all_artifact_classes = registry
            .ids_of_kind(IdKind::ArtifactClass)
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();

        ExpansionContext {
            workstream_alias,
            all_workstreams,
            all_suites,
            all_artifact_classes,
        }
    }
}

// ---------------------------------------------------------------------------
// Cell resolution
// ---------------------------------------------------------------------------

/// Resolve one ledger row into a fully parsed [`ForwardMapping`].
fn resolve_mapping(
    id: &str,
    kind: IdKind,
    row: &LedgerRow,
    ctx: &ExpansionContext,
) -> ForwardMapping {
    ForwardMapping {
        id: id.to_string(),
        kind,
        source_file: "traceability.md".to_string(),
        line: row.line,
        design_sections: parse_design_sections(&row.design),
        workstreams: parse_workstreams(&row.work, ctx),
        suites: parse_suites(&row.validation, ctx),
        risks: parse_risks(&row.risks),
        gates: parse_gates(&row.gates),
        artifact_classes: parse_artifact_classes(&row.evidence, ctx),
    }
}

/// Split a cell into trimmed, non-empty comma-separated tokens.
fn comma_tokens(cell: &str) -> Vec<&str> {
    cell.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect()
}

/// Parse design-section references (`§§2,9,11,19.2`) into section tokens.
fn parse_design_sections(cell: &str) -> Vec<String> {
    if !cell.contains('§') {
        return Vec::new();
    }
    let stripped: String = cell.chars().filter(|&c| c != '§').collect();
    stripped
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Parse the Work cell into canonical `W-*` workstreams, resolving short codes
/// and expanding the `all` marker.
fn parse_workstreams(cell: &str, ctx: &ExpansionContext) -> Vec<String> {
    let mut out = Vec::new();
    for token in comma_tokens(cell) {
        if token.eq_ignore_ascii_case("all") {
            out.extend(ctx.all_workstreams.iter().cloned());
        } else if let Some(canonical) = ctx.workstream_alias.get(token) {
            out.push((*canonical).to_string());
        } else if token.starts_with("W-") && token.len() > 2 {
            // Already-canonical form (not used by the current ledger, but
            // accepted for robustness).
            out.push(token.to_string());
        }
    }
    dedup_sorted(out)
}

/// Parse the Validation cell into suite/activity tokens.
///
/// Recognizes explicit `V-*` suites, the `all V-*` marker (expanded to the full
/// suite set), and named lint/inventory validation activities (e.g.
/// `orphan/doc lint`, `coverage/orphan linter`, `claim inventory`) which the
/// spec treats as valid validation coverage for evidence/documentation rows.
fn parse_suites(cell: &str, ctx: &ExpansionContext) -> Vec<String> {
    let mut out = Vec::new();
    let lower = cell.to_ascii_lowercase();
    if lower.contains("all v-") {
        out.extend(ctx.all_suites.iter().cloned());
    }
    for token in comma_tokens(cell) {
        for word in token.split_whitespace() {
            let word = word.trim_matches('`');
            if word.starts_with("V-") && word.len() > 2 {
                out.push(word.to_string());
            }
        }
        // Named validation activities (linters/inventories) count as coverage.
        let tl = token.to_ascii_lowercase();
        if tl.contains("lint") || tl.contains("inventory") {
            out.push(token.to_string());
        }
    }
    dedup_sorted(out)
}

/// Parse the Risk cell into `R-*` tokens.
fn parse_risks(cell: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in comma_tokens(cell) {
        for word in token.split_whitespace() {
            let word = word.trim_matches('`');
            if word.starts_with("R-") && word.len() > 2 {
                out.push(word.to_string());
            }
        }
    }
    dedup_sorted(out)
}

/// Parse the Gate cell into `F0`..`F6` tokens, expanding en-dash ranges and
/// slash lists (e.g. `F0–F5` → F0..F5, `F0/F6` → F0,F6, `F5` → F5).
fn parse_gates(cell: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in cell.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = split_range(part) {
            if let (Some(start), Some(end)) = (gate_num(a), gate_num(b)) {
                let (lo, hi) = if start <= end {
                    (start, end)
                } else {
                    (end, start)
                };
                for n in lo..=hi {
                    out.push(format!("F{n}"));
                }
                continue;
            }
        }
        for sub in part.split('/') {
            if let Some(n) = gate_num(sub.trim()) {
                out.push(format!("F{n}"));
            }
        }
    }
    dedup_sorted(out)
}

/// Split a token on a range separator (en-dash, em-dash, or hyphen between two
/// gate codes), returning the two ends when present.
fn split_range(part: &str) -> Option<(&str, &str)> {
    for sep in ['\u{2013}', '\u{2014}', '-'] {
        if let Some(idx) = part.find(sep) {
            let (a, b) = part.split_at(idx);
            let b = &b[sep.len_utf8()..];
            let (a, b) = (a.trim(), b.trim());
            if !a.is_empty() && !b.is_empty() {
                return Some((a, b));
            }
        }
    }
    None
}

/// Parse a single `F<digit>` gate token into its numeric index.
fn gate_num(token: &str) -> Option<u32> {
    let token = token.trim();
    let rest = token.strip_prefix('F')?;
    if rest.len() == 1 {
        rest.chars().next().and_then(|c| c.to_digit(10))
    } else {
        None
    }
}

/// Parse the Evidence cell into `A-*` artifact-class tokens, expanding the
/// `all artifact classes` marker.
fn parse_artifact_classes(cell: &str, ctx: &ExpansionContext) -> Vec<String> {
    let mut out = Vec::new();
    if cell.to_ascii_lowercase().contains("all artifact classes") {
        out.extend(ctx.all_artifact_classes.iter().cloned());
    }
    for token in comma_tokens(cell) {
        for word in token.split_whitespace() {
            let word = word.trim_matches('`');
            if word.starts_with("A-") && word.len() > 2 {
                out.push(word.to_string());
            }
        }
    }
    dedup_sorted(out)
}

/// Sort and de-duplicate a token list for deterministic output.
fn dedup_sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v.dedup();
    v
}

// ---------------------------------------------------------------------------
// Audit-ledger single-occurrence validation (traceability.md §4 / §5)
// ---------------------------------------------------------------------------

/// Detect missing and duplicate audit-ledger occurrences for findings and
/// opportunities.
///
/// Duplicates are IDs laid down on more than one ledger row. Missing IDs are
/// gaps in the contiguous per-severity numbering `01..=max(present)` — every
/// finding/opportunity family is contiguously numbered, so a hole means an
/// occurrence is absent.
fn audit_occurrence_issues(registry: &Registry, file: &str) -> Vec<AuditOccurrenceIssue> {
    let mut issues = Vec::new();

    let families = [
        (IdKind::FindingCritical, "MG-C"),
        (IdKind::FindingHigh, "MG-H"),
        (IdKind::FindingMedium, "MG-M"),
        (IdKind::FindingLow, "MG-L"),
        (IdKind::Opportunity, "MG-O"),
    ];

    for (kind, prefix) in families {
        // Group occurrences by ID with their line numbers.
        let mut by_id: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for entry in registry.entries_of_kind(kind) {
            by_id.entry(entry.id.clone()).or_default().push(entry.line);
        }

        // Duplicate occurrences: any ID appearing on more than one row.
        for (id, mut lines) in by_id.clone() {
            if lines.len() > 1 {
                lines.sort_unstable();
                issues.push(AuditOccurrenceIssue {
                    id: id.clone(),
                    kind,
                    source_file: file.to_string(),
                    lines: lines.clone(),
                    issue: AuditIssueKind::Duplicate,
                    message: format!(
                        "{} occurs {} times in the audit ledger (lines {:?}); expected exactly one",
                        id,
                        lines.len(),
                        lines
                    ),
                });
            }
        }

        // Missing occurrences: holes in contiguous numbering up to the max seen.
        let max_present = by_id
            .keys()
            .filter_map(|id| id.strip_prefix(prefix))
            .filter_map(|n| n.parse::<u32>().ok())
            .max();
        if let Some(max) = max_present {
            for n in 1..=max {
                let id = format!("{prefix}{n:02}");
                if !by_id.contains_key(&id) {
                    issues.push(AuditOccurrenceIssue {
                        id: id.clone(),
                        kind,
                        source_file: String::new(),
                        lines: Vec::new(),
                        issue: AuditIssueKind::Missing,
                        message: format!(
                            "{id} has no occurrence in the audit ledger (gap in {prefix}01..{prefix}{max:02})"
                        ),
                    });
                }
            }
        }
    }

    issues.sort_by(|a, b| a.id.cmp(&b.id).then((a.issue as u8).cmp(&(b.issue as u8))));
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_graph::registry::RegistryEntry;

    /// Locate the spec directory relative to this crate.
    fn spec_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.kiro/specs/memory-graph-production-redesign")
    }

    fn real_registry() -> Registry {
        Registry::from_spec_dir(&spec_dir()).expect("registry parses")
    }

    fn entry(id: &str, kind: IdKind, line: usize) -> RegistryEntry {
        RegistryEntry {
            id: id.to_string(),
            kind,
            source_file: "traceability.md".to_string(),
            line,
            title: String::new(),
        }
    }

    // -- Real-spec validation -------------------------------------------------

    #[test]
    fn real_spec_forward_mappings_validate_cleanly() {
        let validation = ForwardValidation::from_spec_dir(&spec_dir()).expect("validates");
        assert!(
            validation.is_ok(),
            "unexpected gaps: {:#?}\naudit issues: {:#?}",
            validation.gaps,
            validation.audit_issues
        );
        // 48 requirements + 46 decisions each have a resolved forward mapping.
        assert_eq!(validation.mappings.len(), 48 + 46);
    }

    #[test]
    fn every_requirement_and_decision_maps_to_all_six_categories() {
        let validation = ForwardValidation::from_spec_dir(&spec_dir()).expect("validates");
        for mapping in &validation.mappings {
            for category in MappingCategory::all() {
                assert!(
                    !mapping.targets(category).is_empty(),
                    "{} missing {} mapping",
                    mapping.id,
                    category.code()
                );
            }
        }
    }

    #[test]
    fn workstream_short_codes_resolve_to_canonical() {
        let validation = ForwardValidation::from_spec_dir(&spec_dir()).expect("validates");
        let mgr1 = validation
            .mappings
            .iter()
            .find(|m| m.id == "MGR-001")
            .expect("MGR-001 mapped");
        // MGR-001 Work column is `WE,WM,WH`.
        assert!(mgr1.workstreams.contains(&"W-EVIDENCE".to_string()));
        assert!(mgr1.workstreams.contains(&"W-SEM".to_string()));
        assert!(mgr1.workstreams.contains(&"W-HUMAN".to_string()));
        // MGR-001 gates span `F0–F5`.
        assert_eq!(
            mgr1.gates,
            vec!["F0", "F1", "F2", "F3", "F4", "F5"],
            "F0–F5 range expansion"
        );
        assert!(mgr1.risks.contains(&"R-TRUTH-LAUNDER".to_string()));
        assert!(mgr1.artifact_classes.contains(&"A-MAN".to_string()));
    }

    #[test]
    fn all_workstream_marker_expands_to_full_set() {
        let reg = real_registry();
        let validation = ForwardValidation::from_spec_dir(&spec_dir()).expect("validates");
        // MGR-048 Work column is `all`.
        let mgr48 = validation
            .mappings
            .iter()
            .find(|m| m.id == "MGR-048")
            .expect("MGR-048 mapped");
        let expected: Vec<String> = reg
            .ids_of_kind(IdKind::Workstream)
            .into_iter()
            .map(str::to_string)
            .collect();
        assert_eq!(mgr48.workstreams, expected);
        assert_eq!(mgr48.workstreams.len(), 12);
    }

    #[test]
    fn evidence_and_documentation_rows_accept_lint_activities_as_suites() {
        let validation = ForwardValidation::from_spec_dir(&spec_dir()).expect("validates");
        // MGD-022 Validation column is only `orphan/doc lint` (no V-* suite).
        let mgd22 = validation
            .mappings
            .iter()
            .find(|m| m.id == "MGD-022")
            .expect("MGD-022 mapped");
        assert!(
            !mgd22.suites.is_empty(),
            "MGD-022 lint activity should satisfy the suite mapping"
        );
    }

    #[test]
    fn every_finding_and_opportunity_occurs_exactly_once() {
        let validation = ForwardValidation::from_spec_dir(&spec_dir()).expect("validates");
        assert!(
            validation.audit_issues.is_empty(),
            "audit occurrence issues: {:#?}",
            validation.audit_issues
        );
    }

    // -- Gate parsing ---------------------------------------------------------

    #[test]
    fn parse_gates_handles_ranges_lists_and_singletons() {
        assert_eq!(
            parse_gates("F0\u{2013}F5"),
            vec!["F0", "F1", "F2", "F3", "F4", "F5"]
        );
        assert_eq!(parse_gates("F0/F6"), vec!["F0", "F6"]);
        assert_eq!(parse_gates("F5"), vec!["F5"]);
        assert_eq!(parse_gates("F1\u{2013}F2"), vec!["F1", "F2"]);
        assert_eq!(
            parse_gates("F0\u{2013}F6"),
            vec!["F0", "F1", "F2", "F3", "F4", "F5", "F6"]
        );
        assert!(parse_gates("").is_empty());
    }

    // -- Synthetic gap detection ---------------------------------------------

    /// A minimal expansion-context registry (workstreams only, for `all`).
    fn ws_registry() -> Registry {
        Registry {
            entries: vec![entry("W-EVIDENCE", IdKind::Workstream, 1)],
        }
    }

    #[test]
    fn synthetic_missing_risk_column_is_detected() {
        let mut reg = ws_registry();
        reg.entries.push(entry("MGR-900", IdKind::Requirement, 10));
        // Risk column deliberately left blank.
        let content = "| MGR-900 Test | §§1 | WE | V-X-01 |  | F1 | A-MAN | Planned |\n";
        let validation = validate(&reg, "traceability.md", content);
        let gap = validation
            .gaps
            .iter()
            .find(|g| g.id == "MGR-900")
            .expect("gap reported");
        assert_eq!(gap.category, Some(MappingCategory::Risk));
        assert_eq!(gap.line, 1);
    }

    #[test]
    fn synthetic_missing_ledger_row_is_detected() {
        let mut reg = ws_registry();
        reg.entries.push(entry("MGD-900", IdKind::Decision, 5));
        // Ledger content has no row for MGD-900 at all.
        let content = "no ledger rows here\n";
        let validation = validate(&reg, "traceability.md", content);
        let gap = validation
            .gaps
            .iter()
            .find(|g| g.id == "MGD-900")
            .expect("gap reported");
        assert_eq!(gap.category, None, "entire row missing");
        assert_eq!(gap.line, 0);
    }

    #[test]
    fn synthetic_complete_row_has_no_gaps() {
        let mut reg = ws_registry();
        reg.entries.push(entry("MGR-901", IdKind::Requirement, 3));
        let content =
            "| MGR-901 Test | §§2 | WE | V-Y-01 | R-Z | F2\u{2013}F4 | A-DB | Planned |\n";
        let validation = validate(&reg, "traceability.md", content);
        assert!(
            validation.gaps.is_empty(),
            "unexpected gaps: {:#?}",
            validation.gaps
        );
        let m = &validation.mappings[0];
        assert_eq!(m.gates, vec!["F2", "F3", "F4"]);
    }

    // -- Synthetic audit-occurrence detection --------------------------------

    #[test]
    fn synthetic_duplicate_finding_occurrence_is_detected() {
        let reg = Registry {
            entries: vec![
                entry("MG-C01", IdKind::FindingCritical, 100),
                entry("MG-C01", IdKind::FindingCritical, 205),
            ],
        };
        let issues = audit_occurrence_issues(&reg, "traceability.md");
        let dup = issues
            .iter()
            .find(|i| i.id == "MG-C01")
            .expect("duplicate reported");
        assert_eq!(dup.issue, AuditIssueKind::Duplicate);
        assert_eq!(dup.lines, vec![100, 205]);
    }

    #[test]
    fn synthetic_missing_finding_occurrence_is_detected() {
        // MG-C02 is absent between MG-C01 and MG-C03.
        let reg = Registry {
            entries: vec![
                entry("MG-C01", IdKind::FindingCritical, 100),
                entry("MG-C03", IdKind::FindingCritical, 102),
            ],
        };
        let issues = audit_occurrence_issues(&reg, "traceability.md");
        let missing = issues
            .iter()
            .find(|i| i.id == "MG-C02")
            .expect("missing reported");
        assert_eq!(missing.issue, AuditIssueKind::Missing);
        assert!(missing.lines.is_empty());
    }

    #[test]
    fn synthetic_opportunity_duplicate_is_detected() {
        let reg = Registry {
            entries: vec![
                entry("MG-O01", IdKind::Opportunity, 400),
                entry("MG-O01", IdKind::Opportunity, 460),
            ],
        };
        let issues = audit_occurrence_issues(&reg, "traceability.md");
        assert!(issues
            .iter()
            .any(|i| i.id == "MG-O01" && i.issue == AuditIssueKind::Duplicate));
    }

    // -- Negative golden inputs (task 0.1.4) ---------------------------------
    //
    // On-disk fixtures under `tests/fixtures/memory-graph/`, each carrying one
    // planted forward-mapping / audit-occurrence defect.

    /// Read a negative golden-input fixture fragment by file name.
    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/memory-graph")
            .join(name);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
    }

    /// A registry containing only the requirement `MGR-001`.
    fn mgr001_registry() -> Registry {
        Registry::from_documents(&[(
            "requirements.md".to_string(),
            "### Requirement 1: MGR-001 — Truth Contract\n".to_string(),
        )])
    }

    #[test]
    fn golden_missing_id_fixture_fails() {
        let reg = mgr001_registry();
        let content = fixture("missing-id.traceability.md");
        let v = validate(&reg, "traceability.md", &content);
        assert!(
            v.gaps
                .iter()
                .any(|g| g.id == "MGR-001" && g.category.is_none()),
            "expected an absent-row mapping gap for MGR-001, got: {:#?}",
            v.gaps
        );
    }

    #[test]
    fn golden_forward_mapping_gap_fixture_fails() {
        let reg = mgr001_registry();
        let content = fixture("forward-mapping-gap.traceability.md");
        let v = validate(&reg, "traceability.md", &content);
        assert_eq!(v.gaps.len(), 1, "expected exactly one gap: {:#?}", v.gaps);
        let gap = &v.gaps[0];
        assert_eq!(gap.id, "MGR-001");
        assert_eq!(gap.category, Some(MappingCategory::Risk));
    }

    #[test]
    fn golden_audit_missing_fixture_fails() {
        let content = fixture("audit-missing.traceability.md");
        let reg = Registry::from_documents(&[("traceability.md".to_string(), content.clone())]);
        let v = validate(&reg, "traceability.md", &content);
        assert!(
            v.audit_issues
                .iter()
                .any(|i| i.id == "MG-C02" && i.issue == AuditIssueKind::Missing),
            "expected audit_missing MG-C02, got: {:#?}",
            v.audit_issues
        );
    }

    #[test]
    fn golden_audit_duplicate_fixture_fails() {
        let content = fixture("audit-duplicate.traceability.md");
        let reg = Registry::from_documents(&[("traceability.md".to_string(), content.clone())]);
        let v = validate(&reg, "traceability.md", &content);
        assert!(
            v.audit_issues
                .iter()
                .any(|i| i.id == "MG-C01" && i.issue == AuditIssueKind::Duplicate),
            "expected audit_duplicate MG-C01, got: {:#?}",
            v.audit_issues
        );
    }
}
