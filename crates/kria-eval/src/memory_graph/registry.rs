//! Canonical evidence-ID registry parser for the Memory Graph Production
//! Redesign spec (task F0.1 / 0.1.1).
//!
//! This module implements the **single canonical parser** (spec invariant:
//! "One canonical parser; exact ranges") that reads the normative spec
//! documents and produces one normalized in-memory registry of every governed
//! identifier, each annotated with the source file and 1-based line number of
//! its canonical definition.
//!
//! Scope of task 0.1.1 is limited to parsing definitions into the registry.
//! Forward-mapping validation (0.1.2), reverse-orphan/duplicate/range checks
//! (0.1.3), negative golden inputs (0.1.4), and the coverage command (0.1.5)
//! are intentionally *not* implemented here; they build on this data model.
//!
//! Each ID family has exactly one canonical definition document:
//!
//! | Family              | Prefix     | Canonical source              |
//! |---------------------|------------|-------------------------------|
//! | Requirement         | `MGR-`     | `requirements.md`             |
//! | Decision            | `MGD-`     | `decisions.md`                |
//! | Finding (C/H/M/L)   | `MG-C/H/M/L` | `traceability.md`           |
//! | Opportunity         | `MG-O`     | `traceability.md`             |
//! | Artifact class      | `A-`       | `traceability.md`             |
//! | Validation suite    | `V-`       | `validation.md`               |
//! | Command             | `CMD-`     | `validation.md`               |
//! | Fixture             | `mg-*-v2`  | `validation.md`               |
//! | Risk                | `R-`       | `risk-analysis.md`            |
//! | Workstream          | `W-`       | `implementation-roadmap.md`   |
//! | Gate                | `F0`..`F6` | `implementation-roadmap.md`   |

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// The classification of a governed identifier in the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdKind {
    /// Normative requirement, `MGR-001`..`MGR-048`.
    Requirement,
    /// Binding decision, `MGD-001`..`MGD-046`.
    Decision,
    /// Critical audit finding, `MG-C01`..`MG-C07`.
    FindingCritical,
    /// High audit finding, `MG-H01`..`MG-H17`.
    FindingHigh,
    /// Medium audit finding, `MG-M01`..`MG-M28`.
    FindingMedium,
    /// Low audit finding, `MG-L01`..`MG-L13`.
    FindingLow,
    /// Opportunity, `MG-O01`..`MG-O31`.
    Opportunity,
    /// Validation/evidence suite, e.g. `V-AUTH-01`.
    Suite,
    /// Risk register entry, e.g. `R-AUTH-SPLIT`.
    Risk,
    /// Workstream, e.g. `W-EVIDENCE`.
    Workstream,
    /// Evidence artifact class, e.g. `A-MAN`.
    ArtifactClass,
    /// Command catalog entry, e.g. `CMD-MG-EVAL`.
    Command,
    /// Deterministic fixture, e.g. `mg-unit-v2`.
    Fixture,
    /// Backend-first gate, `F0`..`F6`.
    Gate,
}

impl IdKind {
    /// A short, stable machine code for this kind, used in diagnostics/reports.
    pub fn code(self) -> &'static str {
        match self {
            IdKind::Requirement => "requirement",
            IdKind::Decision => "decision",
            IdKind::FindingCritical => "finding_critical",
            IdKind::FindingHigh => "finding_high",
            IdKind::FindingMedium => "finding_medium",
            IdKind::FindingLow => "finding_low",
            IdKind::Opportunity => "opportunity",
            IdKind::Suite => "suite",
            IdKind::Risk => "risk",
            IdKind::Workstream => "workstream",
            IdKind::ArtifactClass => "artifact_class",
            IdKind::Command => "command",
            IdKind::Fixture => "fixture",
            IdKind::Gate => "gate",
        }
    }

    /// Whether this kind is one of the four audit-finding severities.
    pub fn is_finding(self) -> bool {
        matches!(
            self,
            IdKind::FindingCritical
                | IdKind::FindingHigh
                | IdKind::FindingMedium
                | IdKind::FindingLow
        )
    }
}

/// One normalized registry entry: a governed ID with the file and 1-based line
/// number of its canonical definition, plus a short human-readable title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegistryEntry {
    /// Canonical identifier text (e.g. `MGR-001`, `V-AUTH-01`, `F3`).
    pub id: String,
    /// Classification of the identifier.
    pub kind: IdKind,
    /// Source document file name (relative to the spec directory).
    pub source_file: String,
    /// 1-based line number where the definition appears.
    pub line: usize,
    /// Short human-readable title/description extracted alongside the ID.
    pub title: String,
}

/// Errors raised while building the registry.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// A required spec document could not be read.
    #[error("failed to read spec document '{path}': {source}")]
    ReadFailed {
        /// Path that failed to read.
        path: String,
        /// Underlying IO error.
        #[source]
        source: std::io::Error,
    },
}

/// The normalized in-memory registry of all governed identifiers.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Registry {
    /// All parsed definitions, in discovery order (file order, then line order).
    pub entries: Vec<RegistryEntry>,
}

impl Registry {
    /// Build the registry by parsing every normative document in `spec_dir`.
    ///
    /// Returns an error only when a required document cannot be read; parsing
    /// itself is total and records every definition it finds. Duplicate/orphan
    /// detection is deliberately deferred to later tasks.
    pub fn from_spec_dir(spec_dir: &Path) -> Result<Self, RegistryError> {
        let mut docs = Vec::with_capacity(CANONICAL_DOCS.len());
        for name in CANONICAL_DOCS {
            let path: PathBuf = spec_dir.join(name);
            let content =
                std::fs::read_to_string(&path).map_err(|source| RegistryError::ReadFailed {
                    path: path.display().to_string(),
                    source,
                })?;
            docs.push((name.to_string(), content));
        }
        Ok(Self::from_documents(&docs))
    }

    /// Build the registry from already-loaded `(file_name, content)` documents.
    ///
    /// Each document is routed to the extractor for its canonical file name
    /// (unknown file names are ignored). Parsing is total — it records every
    /// definition it finds and never fails — so this is the filesystem-free
    /// entry point used by negative golden-input fixtures (task 0.1.4) and any
    /// caller that already has the document text in memory.
    pub fn from_documents(docs: &[(String, String)]) -> Self {
        let mut entries = Vec::new();
        for (name, content) in docs {
            if let Some(extractor) = extractor_for(name) {
                extractor(name, content, &mut entries);
            }
        }
        Registry { entries }
    }

    /// Total number of parsed definitions.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the registry has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// All entries of a given kind, in discovery order.
    pub fn entries_of_kind(&self, kind: IdKind) -> Vec<&RegistryEntry> {
        self.entries.iter().filter(|e| e.kind == kind).collect()
    }

    /// The number of entries of a given kind.
    pub fn count_of_kind(&self, kind: IdKind) -> usize {
        self.entries.iter().filter(|e| e.kind == kind).count()
    }

    /// The sorted, de-duplicated set of IDs of a given kind.
    pub fn ids_of_kind(&self, kind: IdKind) -> Vec<&str> {
        let mut ids: Vec<&str> = self
            .entries
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| e.id.as_str())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// The first entry whose ID matches `id`, if any.
    pub fn find(&self, id: &str) -> Option<&RegistryEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// A count-by-kind summary, useful for coverage reporting.
    pub fn counts_by_kind(&self) -> BTreeMap<&'static str, usize> {
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        for entry in &self.entries {
            *counts.entry(entry.kind.code()).or_insert(0) += 1;
        }
        counts
    }
}

// ---------------------------------------------------------------------------
// Document parsing plumbing
// ---------------------------------------------------------------------------

/// The six canonical normative documents, in discovery order.
const CANONICAL_DOCS: [&str; 6] = [
    "requirements.md",
    "decisions.md",
    "traceability.md",
    "validation.md",
    "risk-analysis.md",
    "implementation-roadmap.md",
];

/// Route a canonical file name to the extractor that parses its definitions.
fn extractor_for(file_name: &str) -> Option<fn(&str, &str, &mut Vec<RegistryEntry>)> {
    match file_name {
        "requirements.md" => Some(extract_requirements),
        "decisions.md" => Some(extract_decisions),
        "traceability.md" => Some(extract_traceability),
        "validation.md" => Some(extract_validation),
        "risk-analysis.md" => Some(extract_risks),
        "implementation-roadmap.md" => Some(extract_roadmap),
        _ => None,
    }
}

/// Iterate `content` yielding `(line_number_1_based, line_text)`.
pub(crate) fn numbered_lines(content: &str) -> impl Iterator<Item = (usize, &str)> {
    content.lines().enumerate().map(|(i, l)| (i + 1, l))
}

/// Split a markdown table row into trimmed cell contents.
///
/// Returns `None` when the trimmed line is not a table row (does not start with
/// `|`). Leading/trailing empty cells produced by the outer pipes are dropped.
pub(crate) fn table_cells(line: &str) -> Option<Vec<&str>> {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') {
        return None;
    }
    let mut cells: Vec<&str> = trimmed.split('|').map(str::trim).collect();
    if cells.first() == Some(&"") {
        cells.remove(0);
    }
    if cells.last() == Some(&"") {
        cells.pop();
    }
    Some(cells)
}

/// Strip a single surrounding pair of backticks (and whitespace) from a cell.
fn strip_backticks(s: &str) -> &str {
    s.trim().trim_matches('`').trim()
}

/// True when `s` is `prefix` followed by exactly `digits` ASCII digits.
pub(crate) fn matches_prefix_num(s: &str, prefix: &str, digits: usize) -> bool {
    match s.strip_prefix(prefix) {
        Some(rest) => rest.len() == digits && rest.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

/// Strip a leading em-dash (or hyphen) separator and surrounding whitespace.
fn strip_title_separator(s: &str) -> String {
    s.trim()
        .trim_start_matches('—')
        .trim_start_matches('-')
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------------
// requirements.md — MGR-001..048
// ---------------------------------------------------------------------------

/// Canonical MGR definitions are the `### Requirement N: MGR-0NN — Title`
/// headings (not the summary table), one per requirement.
fn extract_requirements(file: &str, content: &str, entries: &mut Vec<RegistryEntry>) {
    for (line_no, line) in numbered_lines(content) {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("### Requirement ") else {
            continue;
        };
        // rest looks like: "1: MGR-001 — Epistemic Truth Contract"
        let Some(colon) = rest.find(':') else {
            continue;
        };
        let after = rest[colon + 1..].trim();
        let id_end = after.find(char::is_whitespace).unwrap_or(after.len());
        let id = &after[..id_end];
        if !matches_prefix_num(id, "MGR-", 3) {
            continue;
        }
        let title = strip_title_separator(&after[id_end..]);
        entries.push(RegistryEntry {
            id: id.to_string(),
            kind: IdKind::Requirement,
            source_file: file.to_string(),
            line: line_no,
            title,
        });
    }
}

// ---------------------------------------------------------------------------
// decisions.md — MGD-001..046
// ---------------------------------------------------------------------------

/// Canonical MGD definitions are table rows whose first cell is exactly an
/// `MGD-0NN` identifier (the preserved and new-decision tables).
fn extract_decisions(file: &str, content: &str, entries: &mut Vec<RegistryEntry>) {
    for (line_no, line) in numbered_lines(content) {
        let Some(cells) = table_cells(line) else {
            continue;
        };
        let Some(first) = cells.first() else {
            continue;
        };
        if !matches_prefix_num(first, "MGD-", 3) {
            continue;
        }
        let title = cells.get(1).map(|c| c.to_string()).unwrap_or_default();
        entries.push(RegistryEntry {
            id: (*first).to_string(),
            kind: IdKind::Decision,
            source_file: file.to_string(),
            line: line_no,
            title,
        });
    }
}

// ---------------------------------------------------------------------------
// traceability.md — findings (C/H/M/L), opportunities (O), artifact classes (A)
// ---------------------------------------------------------------------------

/// Extract audit findings, opportunities, and artifact classes from the ledger.
fn extract_traceability(file: &str, content: &str, entries: &mut Vec<RegistryEntry>) {
    for (line_no, line) in numbered_lines(content) {
        // Artifact classes are defined inline in a single sentence.
        if line.trim_start().starts_with("Artifact classes:") {
            for (id, desc) in extract_artifact_classes(line) {
                entries.push(RegistryEntry {
                    id,
                    kind: IdKind::ArtifactClass,
                    source_file: file.to_string(),
                    line: line_no,
                    title: desc,
                });
            }
            continue;
        }

        // Findings and opportunities are ledger table rows keyed by ID.
        let Some(cells) = table_cells(line) else {
            continue;
        };
        let Some(first) = cells.first() else {
            continue;
        };
        let kind = finding_or_opportunity_kind(first);
        let Some(kind) = kind else {
            continue;
        };
        // Ledger columns: | ID | MGR mapping | disposition | Status |
        let title = cells.get(2).map(|c| c.to_string()).unwrap_or_default();
        entries.push(RegistryEntry {
            id: (*first).to_string(),
            kind,
            source_file: file.to_string(),
            line: line_no,
            title,
        });
    }
}

/// Map a first-cell token to a finding severity or opportunity kind.
fn finding_or_opportunity_kind(cell: &str) -> Option<IdKind> {
    if matches_prefix_num(cell, "MG-C", 2) {
        Some(IdKind::FindingCritical)
    } else if matches_prefix_num(cell, "MG-H", 2) {
        Some(IdKind::FindingHigh)
    } else if matches_prefix_num(cell, "MG-M", 2) {
        Some(IdKind::FindingMedium)
    } else if matches_prefix_num(cell, "MG-L", 2) {
        Some(IdKind::FindingLow)
    } else if matches_prefix_num(cell, "MG-O", 2) {
        Some(IdKind::Opportunity)
    } else {
        None
    }
}

/// Extract `(id, description)` pairs for backticked `A-*` tokens on a line.
fn extract_artifact_classes(line: &str) -> Vec<(String, String)> {
    let parts: Vec<&str> = line.split('`').collect();
    let mut out = Vec::new();
    let mut i = 1;
    while i < parts.len() {
        let token = parts[i];
        if token.starts_with("A-")
            && token.len() > 2
            && token[2..].bytes().all(|b| b.is_ascii_alphanumeric())
        {
            let desc = parts
                .get(i + 1)
                .map(|s| s.split(';').next().unwrap_or("").trim().to_string())
                .unwrap_or_default();
            out.push((token.to_string(), desc));
        }
        i += 2;
    }
    out
}

// ---------------------------------------------------------------------------
// validation.md — suites (V-), commands (CMD-), fixtures (mg-*-v2)
// ---------------------------------------------------------------------------

/// Extract validation suites, command-catalog entries, and fixtures.
fn extract_validation(file: &str, content: &str, entries: &mut Vec<RegistryEntry>) {
    for (line_no, line) in numbered_lines(content) {
        let Some(cells) = table_cells(line) else {
            continue;
        };
        let Some(first) = cells.first() else {
            continue;
        };
        let id = strip_backticks(first);

        if id.starts_with("V-") && id.len() > 2 {
            // Suite matrix columns: | Suite | behavior | ... |
            let title = cells.get(1).map(|c| c.to_string()).unwrap_or_default();
            entries.push(RegistryEntry {
                id: id.to_string(),
                kind: IdKind::Suite,
                source_file: file.to_string(),
                line: line_no,
                title,
            });
        } else if id.starts_with("CMD-") && id.len() > 4 {
            // Command columns: | Command ID | Status | Command/cwd | Intended use |
            let title = cells.get(3).map(|c| c.to_string()).unwrap_or_default();
            entries.push(RegistryEntry {
                id: id.to_string(),
                kind: IdKind::Command,
                source_file: file.to_string(),
                line: line_no,
                title,
            });
        } else if is_fixture_id(id) {
            // Fixture columns: | Fixture ID | Seed | Size/purpose | planted cases |
            let title = cells.get(2).map(|c| c.to_string()).unwrap_or_default();
            entries.push(RegistryEntry {
                id: id.to_string(),
                kind: IdKind::Fixture,
                source_file: file.to_string(),
                line: line_no,
                title,
            });
        }
    }
}

/// True when `s` is a fixture identifier of the form `mg-<slug>-v2`.
fn is_fixture_id(s: &str) -> bool {
    s.starts_with("mg-")
        && s.ends_with("-v2")
        && s.len() > "mg--v2".len()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

// ---------------------------------------------------------------------------
// risk-analysis.md — risks (R-)
// ---------------------------------------------------------------------------

/// Canonical risk definitions are register rows whose first cell is a
/// backticked `R-*` identifier.
fn extract_risks(file: &str, content: &str, entries: &mut Vec<RegistryEntry>) {
    for (line_no, line) in numbered_lines(content) {
        let Some(cells) = table_cells(line) else {
            continue;
        };
        let Some(first) = cells.first() else {
            continue;
        };
        let id = strip_backticks(first);
        if !id.starts_with("R-") || id.len() <= 2 {
            continue;
        }
        // Register columns: | ID | Risk and failure mechanism | ... |
        let title = cells.get(1).map(|c| c.to_string()).unwrap_or_default();
        entries.push(RegistryEntry {
            id: id.to_string(),
            kind: IdKind::Risk,
            source_file: file.to_string(),
            line: line_no,
            title,
        });
    }
}

// ---------------------------------------------------------------------------
// implementation-roadmap.md — workstreams (W-) and gates (F0..F6)
// ---------------------------------------------------------------------------

/// Extract workstream table rows and gate-plan headings.
fn extract_roadmap(file: &str, content: &str, entries: &mut Vec<RegistryEntry>) {
    for (line_no, line) in numbered_lines(content) {
        // Gate headings: "### F0 — Evidence Reset and Contract Freeze".
        if let Some((id, title)) = parse_gate_heading(line) {
            entries.push(RegistryEntry {
                id,
                kind: IdKind::Gate,
                source_file: file.to_string(),
                line: line_no,
                title,
            });
            continue;
        }

        // Workstream table rows: | `W-EVIDENCE` | scope | ... |.
        let Some(cells) = table_cells(line) else {
            continue;
        };
        let Some(first) = cells.first() else {
            continue;
        };
        let id = strip_backticks(first);
        if id.starts_with("W-") && id.len() > 2 {
            let title = cells.get(1).map(|c| c.to_string()).unwrap_or_default();
            entries.push(RegistryEntry {
                id: id.to_string(),
                kind: IdKind::Workstream,
                source_file: file.to_string(),
                line: line_no,
                title,
            });
        }
    }
}

/// Parse a gate heading of the form `### F0 — Title`, returning `(id, title)`.
fn parse_gate_heading(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("### ")?;
    let id_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let id = &rest[..id_end];
    let is_gate = id.len() == 2 && id.starts_with('F') && id.as_bytes()[1].is_ascii_digit();
    if !is_gate {
        return None;
    }
    let title = strip_title_separator(&rest[id_end..]);
    Some((id.to_string(), title))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Locate the spec directory relative to this crate.
    fn spec_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.kiro/specs/memory-graph-production-redesign")
    }

    fn registry() -> Registry {
        Registry::from_spec_dir(&spec_dir()).expect("registry parses")
    }

    #[test]
    fn parses_expected_counts_per_family() {
        let reg = registry();
        assert_eq!(reg.count_of_kind(IdKind::Requirement), 48, "MGR");
        assert_eq!(reg.count_of_kind(IdKind::Decision), 46, "MGD");
        assert_eq!(reg.count_of_kind(IdKind::FindingCritical), 7, "MG-C");
        assert_eq!(reg.count_of_kind(IdKind::FindingHigh), 17, "MG-H");
        assert_eq!(reg.count_of_kind(IdKind::FindingMedium), 28, "MG-M");
        assert_eq!(reg.count_of_kind(IdKind::FindingLow), 13, "MG-L");
        assert_eq!(reg.count_of_kind(IdKind::Opportunity), 31, "MG-O");
        assert_eq!(reg.count_of_kind(IdKind::Suite), 33, "V-*");
        assert_eq!(reg.count_of_kind(IdKind::Risk), 27, "R-*");
        assert_eq!(reg.count_of_kind(IdKind::Workstream), 12, "W-*");
        assert_eq!(reg.count_of_kind(IdKind::ArtifactClass), 14, "A-*");
        assert_eq!(reg.count_of_kind(IdKind::Command), 14, "CMD-*");
        assert_eq!(reg.count_of_kind(IdKind::Fixture), 9, "fixtures");
        assert_eq!(reg.count_of_kind(IdKind::Gate), 7, "F0..F6");
    }

    #[test]
    fn total_registry_size_is_stable() {
        // 48+46+65+31+33+27+12+14+14+9+7
        assert_eq!(registry().len(), 306);
    }

    #[test]
    fn findings_total_sixty_five() {
        let reg = registry();
        let findings: usize = [
            IdKind::FindingCritical,
            IdKind::FindingHigh,
            IdKind::FindingMedium,
            IdKind::FindingLow,
        ]
        .iter()
        .map(|k| reg.count_of_kind(*k))
        .sum();
        assert_eq!(findings, 65);
    }

    #[test]
    fn requirement_definition_has_correct_source_and_line() {
        let reg = registry();
        let mgr1 = reg.find("MGR-001").expect("MGR-001 present");
        assert_eq!(mgr1.kind, IdKind::Requirement);
        assert_eq!(mgr1.source_file, "requirements.md");
        assert_eq!(mgr1.title, "Epistemic Truth Contract");
        // Definition heading is at requirements.md line 162.
        assert_eq!(mgr1.line, 162);

        let mgr48 = reg.find("MGR-048").expect("MGR-048 present");
        assert_eq!(mgr48.source_file, "requirements.md");
        assert!(mgr48.title.starts_with("Backend-First"));
    }

    #[test]
    fn decision_definition_points_to_decisions_file() {
        let reg = registry();
        let mgd23 = reg.find("MGD-023").expect("MGD-023 present");
        assert_eq!(mgd23.kind, IdKind::Decision);
        assert_eq!(mgd23.source_file, "decisions.md");
        assert!(mgd23.line > 0);
        assert!(mgd23.title.contains("SQLite v2"));
    }

    #[test]
    fn suite_command_and_fixture_definitions_resolve_to_validation() {
        let reg = registry();
        for id in ["V-AUTH-01", "CMD-MG-EVAL", "mg-unit-v2"] {
            let entry = reg.find(id).unwrap_or_else(|| panic!("{id} present"));
            assert_eq!(entry.source_file, "validation.md", "{id}");
            assert!(entry.line > 0, "{id}");
        }
        assert_eq!(reg.find("V-AUTH-01").unwrap().kind, IdKind::Suite);
        assert_eq!(reg.find("CMD-MG-EVAL").unwrap().kind, IdKind::Command);
        assert_eq!(reg.find("mg-unit-v2").unwrap().kind, IdKind::Fixture);
    }

    #[test]
    fn risk_workstream_artifact_and_gate_definitions_resolve() {
        let reg = registry();

        let risk = reg.find("R-AUTH-SPLIT").expect("risk present");
        assert_eq!(risk.kind, IdKind::Risk);
        assert_eq!(risk.source_file, "risk-analysis.md");

        let ws = reg.find("W-EVIDENCE").expect("workstream present");
        assert_eq!(ws.kind, IdKind::Workstream);
        assert_eq!(ws.source_file, "implementation-roadmap.md");

        let artifact = reg.find("A-MAN").expect("artifact class present");
        assert_eq!(artifact.kind, IdKind::ArtifactClass);
        assert_eq!(artifact.source_file, "traceability.md");

        let gate = reg.find("F0").expect("gate present");
        assert_eq!(gate.kind, IdKind::Gate);
        assert_eq!(gate.source_file, "implementation-roadmap.md");
        assert!(gate.title.starts_with("Evidence Reset"));
    }

    #[test]
    fn all_expected_requirement_ids_present_in_range() {
        let reg = registry();
        for n in 1..=48 {
            let id = format!("MGR-{n:03}");
            assert!(reg.find(&id).is_some(), "missing {id}");
        }
    }

    #[test]
    fn all_expected_gate_ids_present() {
        let reg = registry();
        for n in 0..=6 {
            let id = format!("F{n}");
            assert!(reg.find(&id).is_some(), "missing gate {id}");
        }
    }

    #[test]
    fn every_entry_has_source_line_and_nonempty_id() {
        let reg = registry();
        for entry in &reg.entries {
            assert!(!entry.id.is_empty(), "empty id at {}", entry.line);
            assert!(entry.line > 0, "zero line for {}", entry.id);
            assert!(
                !entry.source_file.is_empty(),
                "empty source for {}",
                entry.id
            );
        }
    }

    #[test]
    fn registry_serializes_to_json() {
        let reg = registry();
        let json = serde_json::to_string(&reg).expect("serializes");
        assert!(json.contains("\"MGR-001\""));
        assert!(json.contains("\"requirement\""));
    }
}
