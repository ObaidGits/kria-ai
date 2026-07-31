//! Closed-world integrity validation for the Memory Graph Production Redesign
//! spec (task F0.1 / 0.1.3).
//!
//! This module builds on the canonical [`Registry`](crate::memory_graph::Registry)
//! (task 0.1.1) and the forward-mapping resolver
//! [`ForwardValidation`](crate::memory_graph::ForwardValidation) (task 0.1.2)
//! and implements the *reverse* and *structural* half of the traceability
//! contract described in `traceability.md` §6. It fails closed on:
//!
//! 1. **Reverse orphans** — a *defined* suite (`V-*`), risk (`R-*`), artifact
//!    class (`A-*`), or workstream (`W-*`) that no `MGR`/`MGD` ledger row maps
//!    to (i.e. defined but never governed by a requirement or decision).
//! 2. **Duplicate IDs** — the same canonical identifier defined more than once.
//! 3. **Invalid ranges** — an ID whose number falls outside its declared
//!    contiguous range, or a gap (a missing number) inside that range:
//!    `MGR-001..048`, `MGD-001..046`, `MG-C01..07`, `MG-H01..17`,
//!    `MG-M01..28`, `MG-L01..13`, `MG-O01..31`, gates `F0..F6`.
//! 4. **Undefined codes** — a reference to a governed code (valid prefix) that
//!    has no canonical definition anywhere. This is the class that originally
//!    caught `R-DATA-01`, referenced in `requirements.md` but never defined in
//!    `risk-analysis.md`; task F0.5.3 resolved that defect (MGR-019 now cites
//!    the defined `R-WRONG-MERGE, R-POLICY-LEAK`), so the real spec is clean.
//! 5. **Later-gate predecessor gaps** — the backend-first chain is strictly
//!    `F0 → F1 → … → F6`; a defined gate whose predecessor in that chain is
//!    undefined cannot have satisfied predecessor evidence.
//! 6. **Non-`Planned` status without a manifest** — any ledger status other
//!    than `Planned`/`Unverified` must point to an existing valid manifest
//!    path/hash; a row that claims a stronger status without one fails.
//!
//! Diagnostics are deterministic and machine-readable (kind/ID/file/line/reason)
//! and sorted by `(kind, id, source_file, line)` so the CI-facing report schema
//! (task 0.1.4) and coverage command (task 0.1.5) can consume them stably.
//!
//! ## Scope boundary
//!
//! This task validates structural integrity of the *definitions and their
//! references*. It intentionally does not build negative golden-input fixtures
//! (task 0.1.4) or wire the exit-code-bearing coverage command (task 0.1.5);
//! both consume the [`IntegrityValidation`] value this module produces.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::forward::ForwardValidation;
use super::registry::{self, IdKind, Registry, RegistryError};

/// The class of integrity defect detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityIssueKind {
    /// A defined suite/risk/artifact-class/workstream governed by no MGR/MGD.
    ReverseOrphan,
    /// The same canonical ID defined more than once.
    DuplicateId,
    /// An ID outside its declared contiguous range, or a gap inside it.
    InvalidRange,
    /// A reference to a governed code that has no definition.
    UndefinedCode,
    /// A gate whose predecessor in the `F0..F6` chain is undefined.
    PredecessorGap,
    /// A non-`Planned`/`Unverified` status with no valid manifest path/hash.
    StatusWithoutManifest,
}

impl IntegrityIssueKind {
    /// A short, stable machine code for this kind, used in diagnostics/reports.
    pub fn code(self) -> &'static str {
        match self {
            IntegrityIssueKind::ReverseOrphan => "reverse_orphan",
            IntegrityIssueKind::DuplicateId => "duplicate_id",
            IntegrityIssueKind::InvalidRange => "invalid_range",
            IntegrityIssueKind::UndefinedCode => "undefined_code",
            IntegrityIssueKind::PredecessorGap => "predecessor_gap",
            IntegrityIssueKind::StatusWithoutManifest => "status_without_manifest",
        }
    }
}

/// One integrity defect: a machine-readable, deterministic diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntegrityIssue {
    /// The class of defect.
    pub kind: IntegrityIssueKind,
    /// The offending identifier or reference token.
    pub id: String,
    /// Source document where the defect is observed (definition or reference).
    pub source_file: String,
    /// 1-based line number of the observation, or `0` when not line-bound.
    pub line: usize,
    /// Deterministic human-readable diagnostic message.
    pub reason: String,
}

impl IntegrityIssue {
    /// Deterministic ordering key: `(kind, id, source_file, line)`.
    fn sort_key(&self) -> (IntegrityIssueKind, &str, &str, usize) {
        (
            self.kind,
            self.id.as_str(),
            self.source_file.as_str(),
            self.line,
        )
    }
}

/// The complete result of integrity validation.
#[derive(Debug, Clone, Default, Serialize)]
pub struct IntegrityValidation {
    /// All detected issues, sorted by `(kind, id, source_file, line)`.
    pub issues: Vec<IntegrityIssue>,
}

impl IntegrityValidation {
    /// Whether validation found no integrity defects.
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }

    /// All issues of a given kind, preserving sorted order.
    pub fn issues_of_kind(&self, kind: IntegrityIssueKind) -> Vec<&IntegrityIssue> {
        self.issues.iter().filter(|i| i.kind == kind).collect()
    }

    /// Build the registry from `spec_dir` and run all integrity checks.
    pub fn from_spec_dir(spec_dir: &Path) -> Result<Self, RegistryError> {
        let registry = Registry::from_spec_dir(spec_dir)?;
        Self::from_registry(spec_dir, &registry)
    }

    /// Run all integrity checks against an already-built [`Registry`].
    ///
    /// Reads every normative document for references (undefined-code scan and
    /// status/manifest scan) and reuses [`ForwardValidation`] to determine which
    /// codes are governed by an `MGR`/`MGD` ledger row (reverse-orphan scan).
    pub fn from_registry(spec_dir: &Path, registry: &Registry) -> Result<Self, RegistryError> {
        let docs = read_reference_docs(spec_dir)?;
        let forward = ForwardValidation::from_registry(spec_dir, registry)?;
        let ledger = read_ledger(spec_dir)?;
        Ok(validate(registry, &forward, &docs, &ledger))
    }
}

/// The set of normative documents scanned for governed-code references.
///
/// This is the closed world of the spec: `design.md` is included (the contract
/// requires "design §20" participation) even though the registry parses its
/// definitions from the other six.
const REFERENCE_DOCS: [&str; 7] = [
    "requirements.md",
    "decisions.md",
    "traceability.md",
    "validation.md",
    "risk-analysis.md",
    "implementation-roadmap.md",
    "design.md",
];

/// Read every reference document, returning `(file_name, content)` pairs.
fn read_reference_docs(spec_dir: &Path) -> Result<Vec<(String, String)>, RegistryError> {
    let mut docs = Vec::with_capacity(REFERENCE_DOCS.len());
    for name in REFERENCE_DOCS {
        docs.push((name.to_string(), read_doc(spec_dir, name)?));
    }
    Ok(docs)
}

/// Read the traceability ledger content for the status/manifest scan.
fn read_ledger(spec_dir: &Path) -> Result<String, RegistryError> {
    read_doc(spec_dir, "traceability.md")
}

/// Read one document, surfacing IO errors as [`RegistryError::ReadFailed`].
fn read_doc(spec_dir: &Path, name: &str) -> Result<String, RegistryError> {
    let path: PathBuf = spec_dir.join(name);
    std::fs::read_to_string(&path).map_err(|source| RegistryError::ReadFailed {
        path: path.display().to_string(),
        source,
    })
}

// ---------------------------------------------------------------------------
// Top-level validation
// ---------------------------------------------------------------------------

/// Run every integrity check and return a sorted, deterministic result.
///
/// Split out from [`IntegrityValidation::from_registry`] so tests can drive it
/// with synthetic registries, ledger text, and forward validations without
/// touching the filesystem.
fn validate(
    registry: &Registry,
    forward: &ForwardValidation,
    docs: &[(String, String)],
    ledger: &str,
) -> IntegrityValidation {
    let mut issues = Vec::new();

    issues.extend(check_duplicates(registry));
    issues.extend(check_ranges(registry));
    issues.extend(check_predecessor_gaps(registry));
    issues.extend(check_reverse_orphans(registry, forward));
    issues.extend(check_undefined_codes(registry, docs));
    issues.extend(check_status_manifests("traceability.md", ledger));

    issues.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    issues.dedup();

    IntegrityValidation { issues }
}

// ---------------------------------------------------------------------------
// 1. Duplicate IDs
// ---------------------------------------------------------------------------

/// Detect any canonical ID defined on more than one registry entry.
fn check_duplicates(registry: &Registry) -> Vec<IntegrityIssue> {
    let mut by_id: BTreeMap<&str, Vec<&super::registry::RegistryEntry>> = BTreeMap::new();
    for entry in &registry.entries {
        by_id.entry(entry.id.as_str()).or_default().push(entry);
    }

    let mut issues = Vec::new();
    for (id, entries) in by_id {
        if entries.len() < 2 {
            continue;
        }
        let mut locations: Vec<(String, usize)> = entries
            .iter()
            .map(|e| (e.source_file.clone(), e.line))
            .collect();
        locations.sort();
        for (file, line) in &locations {
            issues.push(IntegrityIssue {
                kind: IntegrityIssueKind::DuplicateId,
                id: id.to_string(),
                source_file: file.clone(),
                line: *line,
                reason: format!(
                    "{id} is defined {} times (locations {:?}); expected exactly one canonical definition",
                    locations.len(),
                    locations
                ),
            });
        }
    }
    issues
}

// ---------------------------------------------------------------------------
// 2. Invalid ranges (out-of-range numbers and gaps)
// ---------------------------------------------------------------------------

/// A declared contiguous numeric range for one identifier family.
struct RangeSpec {
    kind: IdKind,
    prefix: &'static str,
    width: usize,
    min: u32,
    max: u32,
}

/// The declared contiguous ranges for every numbered family (exact ranges).
const RANGE_SPECS: [RangeSpec; 7] = [
    RangeSpec {
        kind: IdKind::Requirement,
        prefix: "MGR-",
        width: 3,
        min: 1,
        max: 48,
    },
    RangeSpec {
        kind: IdKind::Decision,
        prefix: "MGD-",
        width: 3,
        min: 1,
        max: 46,
    },
    RangeSpec {
        kind: IdKind::FindingCritical,
        prefix: "MG-C",
        width: 2,
        min: 1,
        max: 7,
    },
    RangeSpec {
        kind: IdKind::FindingHigh,
        prefix: "MG-H",
        width: 2,
        min: 1,
        max: 17,
    },
    RangeSpec {
        kind: IdKind::FindingMedium,
        prefix: "MG-M",
        width: 2,
        min: 1,
        max: 28,
    },
    RangeSpec {
        kind: IdKind::FindingLow,
        prefix: "MG-L",
        width: 2,
        min: 1,
        max: 13,
    },
    RangeSpec {
        kind: IdKind::Opportunity,
        prefix: "MG-O",
        width: 2,
        min: 1,
        max: 31,
    },
];

/// Parse the numeric suffix of an ID with the given prefix.
fn suffix_number(id: &str, prefix: &str) -> Option<u32> {
    id.strip_prefix(prefix)?.parse::<u32>().ok()
}

/// Detect out-of-range definitions and gaps inside each declared range.
fn check_ranges(registry: &Registry) -> Vec<IntegrityIssue> {
    let mut issues = Vec::new();

    for spec in &RANGE_SPECS {
        // Map present number -> first (file, line) for deterministic diagnostics.
        let mut present: BTreeMap<u32, (String, usize)> = BTreeMap::new();
        for entry in registry.entries_of_kind(spec.kind) {
            match suffix_number(&entry.id, spec.prefix) {
                Some(n) => {
                    present
                        .entry(n)
                        .or_insert_with(|| (entry.source_file.clone(), entry.line));
                    if n < spec.min || n > spec.max {
                        issues.push(IntegrityIssue {
                            kind: IntegrityIssueKind::InvalidRange,
                            id: entry.id.clone(),
                            source_file: entry.source_file.clone(),
                            line: entry.line,
                            reason: format!(
                                "{} is out of range {}{:0width$}..{}{:0width$}",
                                entry.id,
                                spec.prefix,
                                spec.min,
                                spec.prefix,
                                spec.max,
                                width = spec.width
                            ),
                        });
                    }
                }
                None => issues.push(IntegrityIssue {
                    kind: IntegrityIssueKind::InvalidRange,
                    id: entry.id.clone(),
                    source_file: entry.source_file.clone(),
                    line: entry.line,
                    reason: format!(
                        "{} does not carry a parseable {} number",
                        entry.id, spec.prefix
                    ),
                }),
            }
        }

        // Gaps: any number in the declared range with no definition.
        for n in spec.min..=spec.max {
            if !present.contains_key(&n) {
                let id = format!("{}{:0width$}", spec.prefix, n, width = spec.width);
                issues.push(IntegrityIssue {
                    kind: IntegrityIssueKind::InvalidRange,
                    id,
                    source_file: String::new(),
                    line: 0,
                    reason: format!(
                        "gap in {}{:0width$}..{}{:0width$}: {}{:0width$} has no definition",
                        spec.prefix,
                        spec.min,
                        spec.prefix,
                        spec.max,
                        spec.prefix,
                        n,
                        width = spec.width
                    ),
                });
            }
        }
    }

    // Gates: valid single-digit indices 0..=6.
    for entry in registry.entries_of_kind(IdKind::Gate) {
        match gate_index(&entry.id) {
            Some(n) if n <= 6 => {}
            _ => issues.push(IntegrityIssue {
                kind: IntegrityIssueKind::InvalidRange,
                id: entry.id.clone(),
                source_file: entry.source_file.clone(),
                line: entry.line,
                reason: format!("{} is out of the gate range F0..F6", entry.id),
            }),
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// 3. Later-gate predecessor gaps
// ---------------------------------------------------------------------------

/// Parse a gate id `F<digit>` into its numeric index.
fn gate_index(id: &str) -> Option<u32> {
    let rest = id.strip_prefix('F')?;
    if rest.len() == 1 {
        rest.chars().next().and_then(|c| c.to_digit(10))
    } else {
        None
    }
}

/// Detect gates whose predecessor in the strict `F0 → … → F6` chain is
/// undefined; such a later gate cannot have satisfied predecessor evidence.
fn check_predecessor_gaps(registry: &Registry) -> Vec<IntegrityIssue> {
    let defined: BTreeSet<u32> = registry
        .entries_of_kind(IdKind::Gate)
        .iter()
        .filter_map(|e| gate_index(&e.id))
        .filter(|n| *n <= 6)
        .collect();

    let mut issues = Vec::new();
    for entry in registry.entries_of_kind(IdKind::Gate) {
        let Some(n) = gate_index(&entry.id).filter(|n| *n <= 6) else {
            continue;
        };
        // Every gate below n must be defined for the chain to be satisfiable.
        let missing: Vec<u32> = (0..n).filter(|p| !defined.contains(p)).collect();
        if !missing.is_empty() {
            let missing_ids: Vec<String> = missing.iter().map(|p| format!("F{p}")).collect();
            issues.push(IntegrityIssue {
                kind: IntegrityIssueKind::PredecessorGap,
                id: entry.id.clone(),
                source_file: entry.source_file.clone(),
                line: entry.line,
                reason: format!(
                    "{} has no predecessor evidence: {:?} undefined in the F0..F6 chain",
                    entry.id, missing_ids
                ),
            });
        }
    }
    issues
}

// ---------------------------------------------------------------------------
// 4. Reverse orphans (defined but governed by no MGR/MGD)
// ---------------------------------------------------------------------------

/// Detect defined suites/risks/artifact-classes/workstreams that no `MGR`/`MGD`
/// ledger row maps to.
fn check_reverse_orphans(registry: &Registry, forward: &ForwardValidation) -> Vec<IntegrityIssue> {
    // The union of every code governed by a requirement/decision ledger row.
    let mut governed: BTreeSet<&str> = BTreeSet::new();
    for mapping in &forward.mappings {
        for code in mapping
            .workstreams
            .iter()
            .chain(&mapping.suites)
            .chain(&mapping.risks)
            .chain(&mapping.artifact_classes)
        {
            governed.insert(code.as_str());
        }
    }

    let mut issues = Vec::new();
    for kind in [
        IdKind::Suite,
        IdKind::Risk,
        IdKind::ArtifactClass,
        IdKind::Workstream,
    ] {
        for entry in registry.entries_of_kind(kind) {
            if !governed.contains(entry.id.as_str()) {
                issues.push(IntegrityIssue {
                    kind: IntegrityIssueKind::ReverseOrphan,
                    id: entry.id.clone(),
                    source_file: entry.source_file.clone(),
                    line: entry.line,
                    reason: format!(
                        "{} ({}) is defined but governed by no MGR/MGD ledger row",
                        entry.id,
                        kind.code()
                    ),
                });
            }
        }
    }
    issues
}

// ---------------------------------------------------------------------------
// 5. Undefined codes (references with a governed prefix but no definition)
// ---------------------------------------------------------------------------

/// Detect references to governed codes that have no canonical definition.
///
/// Suite references may be *family* references (e.g. `V-AUTH` covering
/// `V-AUTH-01..03`); such a reference is satisfied when any defined suite id
/// equals it or begins with `"{token}-"`.
fn check_undefined_codes(registry: &Registry, docs: &[(String, String)]) -> Vec<IntegrityIssue> {
    let defined: BTreeSet<&str> = registry.entries.iter().map(|e| e.id.as_str()).collect();
    let defined_suites: Vec<&str> = registry
        .entries
        .iter()
        .filter(|e| e.kind == IdKind::Suite)
        .map(|e| e.id.as_str())
        .collect();

    // De-duplicate identical (id, file, line) observations across scans.
    let mut seen: BTreeSet<(String, String, usize)> = BTreeSet::new();
    let mut issues = Vec::new();

    for (file, content) in docs {
        for (line_no, line) in registry::numbered_lines(content) {
            for token in scan_tokens(line) {
                let Some(family) = classify_reference(token) else {
                    continue;
                };
                let resolved = match family {
                    RefFamily::Suite => {
                        defined.contains(token) || suite_family_defined(token, &defined_suites)
                    }
                    _ => defined.contains(token),
                };
                if resolved {
                    continue;
                }
                let key = (token.to_string(), file.clone(), line_no);
                if !seen.insert(key) {
                    continue;
                }
                issues.push(IntegrityIssue {
                    kind: IntegrityIssueKind::UndefinedCode,
                    id: token.to_string(),
                    source_file: file.clone(),
                    line: line_no,
                    reason: format!(
                        "{token} ({}) is referenced but has no canonical definition",
                        family.code()
                    ),
                });
            }
        }
    }
    issues
}

/// The family a governed reference token belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefFamily {
    Requirement,
    Decision,
    Finding,
    Opportunity,
    Suite,
    Risk,
    Workstream,
    ArtifactClass,
    Command,
    Gate,
    Fixture,
}

impl RefFamily {
    fn code(self) -> &'static str {
        match self {
            RefFamily::Requirement => "requirement",
            RefFamily::Decision => "decision",
            RefFamily::Finding => "finding",
            RefFamily::Opportunity => "opportunity",
            RefFamily::Suite => "suite",
            RefFamily::Risk => "risk",
            RefFamily::Workstream => "workstream",
            RefFamily::ArtifactClass => "artifact_class",
            RefFamily::Command => "command",
            RefFamily::Gate => "gate",
            RefFamily::Fixture => "fixture",
        }
    }
}

/// Split a line into candidate ID tokens (runs of ASCII alphanumerics and `-`).
///
/// Range separators such as en/em dashes, slashes, commas, and whitespace are
/// not part of an ID token, so `MG-C01–C04` yields `MG-C01` and `C04`, and
/// `V-RET-01..03` yields `V-RET-01` and `03`.
fn scan_tokens(line: &str) -> Vec<&str> {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter(|t| !t.is_empty())
        .collect()
}

/// Classify a raw token as a governed reference family, if it is one.
fn classify_reference(t: &str) -> Option<RefFamily> {
    if registry::matches_prefix_num(t, "MGR-", 3) {
        return Some(RefFamily::Requirement);
    }
    if registry::matches_prefix_num(t, "MGD-", 3) {
        return Some(RefFamily::Decision);
    }
    for p in ["MG-C", "MG-H", "MG-M", "MG-L"] {
        if registry::matches_prefix_num(t, p, 2) {
            return Some(RefFamily::Finding);
        }
    }
    if registry::matches_prefix_num(t, "MG-O", 2) {
        return Some(RefFamily::Opportunity);
    }
    if is_command(t) {
        return Some(RefFamily::Command);
    }
    if is_suite(t) {
        return Some(RefFamily::Suite);
    }
    if is_risk(t) {
        return Some(RefFamily::Risk);
    }
    if is_workstream(t) {
        return Some(RefFamily::Workstream);
    }
    if is_artifact(t) {
        return Some(RefFamily::ArtifactClass);
    }
    if is_gate(t) {
        return Some(RefFamily::Gate);
    }
    if is_fixture(t) {
        return Some(RefFamily::Fixture);
    }
    None
}

/// True when the suffix after `prefix` is a well-formed uppercase code: made of
/// uppercase letters, digits, and single interior `-` separators, with at least
/// one alphanumeric character. Rejects malformed tokens such as `R--` (a
/// mermaid arrow) or codes with leading/trailing/doubled hyphens.
fn upper_suffix(t: &str, prefix: &str) -> bool {
    let Some(rest) = t.strip_prefix(prefix) else {
        return false;
    };
    if rest.is_empty() || rest.starts_with('-') || rest.ends_with('-') || rest.contains("--") {
        return false;
    }
    let all_valid = rest
        .bytes()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-');
    let has_alnum = rest
        .bytes()
        .any(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
    all_valid && has_alnum
}

fn is_suite(t: &str) -> bool {
    upper_suffix(t, "V-")
}

fn is_risk(t: &str) -> bool {
    upper_suffix(t, "R-")
}

fn is_workstream(t: &str) -> bool {
    upper_suffix(t, "W-")
}

fn is_command(t: &str) -> bool {
    upper_suffix(t, "CMD-")
}

/// Artifact classes are `A-` followed by an uppercase letter then alphanumerics.
fn is_artifact(t: &str) -> bool {
    match t.strip_prefix("A-") {
        Some(rest) => {
            rest.bytes().next().is_some_and(|b| b.is_ascii_uppercase())
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        }
        None => false,
    }
}

/// Gates are exactly `F` followed by a single digit.
fn is_gate(t: &str) -> bool {
    let b = t.as_bytes();
    b.len() == 2 && b[0] == b'F' && b[1].is_ascii_digit()
}

/// Fixtures are `mg-<slug>-v2` in lowercase.
fn is_fixture(t: &str) -> bool {
    t.starts_with("mg-")
        && t.ends_with("-v2")
        && t.len() > "mg--v2".len()
        && t.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// True when `token` is a suite family covering at least one defined suite id.
fn suite_family_defined(token: &str, defined_suites: &[&str]) -> bool {
    let family_prefix = format!("{token}-");
    defined_suites
        .iter()
        .any(|s| *s == token || s.starts_with(&family_prefix))
}

// ---------------------------------------------------------------------------
// 6. Non-`Planned` status without a valid manifest path/hash
// ---------------------------------------------------------------------------

/// Statuses that are allowed to exist without a linked manifest.
fn is_planned_status(status: &str) -> bool {
    let s = status.trim().to_ascii_lowercase();
    matches!(s.as_str(), "planned" | "unverified" | "planned/unverified")
}

/// True when `line` carries a plausible manifest path or content hash.
fn has_manifest_reference(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if lower.contains("manifest") || lower.contains("evidence/") {
        return true;
    }
    // A 40+ char hex run looks like a content hash (sha-1/256).
    let mut run = 0usize;
    for b in line.bytes() {
        if b.is_ascii_hexdigit() {
            run += 1;
            if run >= 40 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// True when the first cell of a ledger row keys a governed status-bearing ID.
fn is_status_row_id(cell: &str) -> bool {
    let token = cell.split_whitespace().next().unwrap_or("");
    registry::matches_prefix_num(token, "MGR-", 3)
        || registry::matches_prefix_num(token, "MGD-", 3)
        || ["MG-C", "MG-H", "MG-M", "MG-L"]
            .iter()
            .any(|p| registry::matches_prefix_num(token, p, 2))
        || registry::matches_prefix_num(token, "MG-O", 2)
}

/// Detect ledger rows whose status exceeds `Planned`/`Unverified` yet carry no
/// valid manifest path/hash.
fn check_status_manifests(file: &str, content: &str) -> Vec<IntegrityIssue> {
    let mut issues = Vec::new();
    for (line_no, line) in registry::numbered_lines(content) {
        let Some(cells) = registry::table_cells(line) else {
            continue;
        };
        let (Some(first), Some(status)) = (cells.first(), cells.last()) else {
            continue;
        };
        if !is_status_row_id(first) {
            continue;
        }
        if is_planned_status(status) {
            continue;
        }
        if has_manifest_reference(line) {
            continue;
        }
        let id = first.split_whitespace().next().unwrap_or("").to_string();
        issues.push(IntegrityIssue {
            kind: IntegrityIssueKind::StatusWithoutManifest,
            id,
            source_file: file.to_string(),
            line: line_no,
            reason: format!(
                "status '{}' is not Planned/Unverified and links no valid manifest path/hash",
                status.trim()
            ),
        });
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_graph::forward::{ForwardMapping, ForwardValidation};
    use crate::memory_graph::registry::RegistryEntry;

    /// Locate the spec directory relative to this crate.
    fn spec_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.kiro/specs/memory-graph-production-redesign")
    }

    fn entry(id: &str, kind: IdKind, line: usize) -> RegistryEntry {
        RegistryEntry {
            id: id.to_string(),
            kind,
            source_file: "test.md".to_string(),
            line,
            title: String::new(),
        }
    }

    /// A forward validation whose single mapping governs the given codes.
    fn forward_governing(
        workstreams: &[&str],
        suites: &[&str],
        risks: &[&str],
        artifacts: &[&str],
    ) -> ForwardValidation {
        let to_vec = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        ForwardValidation {
            mappings: vec![ForwardMapping {
                id: "MGR-001".to_string(),
                kind: IdKind::Requirement,
                source_file: "traceability.md".to_string(),
                line: 1,
                design_sections: vec!["1".to_string()],
                workstreams: to_vec(workstreams),
                suites: to_vec(suites),
                risks: to_vec(risks),
                gates: vec!["F0".to_string()],
                artifact_classes: to_vec(artifacts),
            }],
            ..Default::default()
        }
    }

    // -- Real-spec validation -------------------------------------------------

    #[test]
    fn real_spec_is_integrity_clean_after_r_data_01_resolution() {
        let v = IntegrityValidation::from_spec_dir(&spec_dir()).expect("validates");
        // F0.5.3 resolved the previously-known `R-DATA-01` undefined-code
        // reverse orphan: `requirements.md` MGR-019 now references the defined
        // risks `R-WRONG-MERGE, R-POLICY-LEAK` (consistent with the
        // traceability.md ledger row) instead of the never-defined `R-DATA-01`.
        // The real spec must therefore validate with zero integrity issues.
        assert!(
            v.is_ok(),
            "real spec should be integrity-clean after R-DATA-01 resolution: {:#?}",
            v.issues
        );
        assert!(
            v.issues.is_empty(),
            "unexpected integrity issues: {:#?}",
            v.issues
        );
        // The stale `R-DATA-01` reference must be fully gone.
        assert!(
            !v.issues.iter().any(|issue| issue.id == "R-DATA-01"),
            "R-DATA-01 must no longer be referenced anywhere in the spec"
        );
    }

    #[test]
    fn real_spec_has_no_duplicate_range_predecessor_or_status_defects() {
        let v = IntegrityValidation::from_spec_dir(&spec_dir()).expect("validates");
        assert!(v.issues_of_kind(IntegrityIssueKind::DuplicateId).is_empty());
        assert!(v
            .issues_of_kind(IntegrityIssueKind::InvalidRange)
            .is_empty());
        assert!(v
            .issues_of_kind(IntegrityIssueKind::PredecessorGap)
            .is_empty());
        assert!(v
            .issues_of_kind(IntegrityIssueKind::ReverseOrphan)
            .is_empty());
        assert!(v
            .issues_of_kind(IntegrityIssueKind::StatusWithoutManifest)
            .is_empty());
    }

    #[test]
    fn issues_are_sorted_deterministically() {
        let v = IntegrityValidation::from_spec_dir(&spec_dir()).expect("validates");
        let mut sorted = v.issues.clone();
        sorted.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        assert_eq!(v.issues, sorted, "issues must be emitted in sorted order");
    }

    // -- Duplicate IDs --------------------------------------------------------

    #[test]
    fn synthetic_duplicate_id_is_detected() {
        let reg = Registry {
            entries: vec![
                entry("MGD-005", IdKind::Decision, 10),
                entry("MGD-005", IdKind::Decision, 42),
            ],
        };
        let issues = check_duplicates(&reg);
        assert!(!issues.is_empty());
        assert!(issues.iter().all(|i| i.id == "MGD-005"));
        assert!(issues
            .iter()
            .all(|i| i.kind == IntegrityIssueKind::DuplicateId));
        assert_eq!(issues.len(), 2, "one issue per duplicate location");
    }

    #[test]
    fn unique_ids_produce_no_duplicate_issues() {
        let reg = Registry {
            entries: vec![
                entry("MGR-001", IdKind::Requirement, 1),
                entry("MGR-002", IdKind::Requirement, 2),
            ],
        };
        assert!(check_duplicates(&reg).is_empty());
    }

    // -- Invalid ranges -------------------------------------------------------

    #[test]
    fn synthetic_out_of_range_number_is_detected() {
        // Provide the full MG-C family plus an out-of-range MG-C08.
        let mut entries: Vec<RegistryEntry> = (1..=7)
            .map(|n| entry(&format!("MG-C{n:02}"), IdKind::FindingCritical, n as usize))
            .collect();
        entries.push(entry("MG-C08", IdKind::FindingCritical, 100));
        let reg = Registry { entries };
        let issues = check_ranges(&reg);
        let oor: Vec<_> = issues.iter().filter(|i| i.id == "MG-C08").collect();
        assert_eq!(oor.len(), 1, "MG-C08 out of range flagged once");
        assert_eq!(oor[0].kind, IntegrityIssueKind::InvalidRange);
        // No gaps because MG-C01..07 are all present.
        assert!(issues.iter().all(|i| i.id != "MG-C01"));
    }

    #[test]
    fn synthetic_range_gap_is_detected() {
        // Full MG-C family except MG-C04.
        let entries: Vec<RegistryEntry> = (1..=7)
            .filter(|n| *n != 4)
            .map(|n| entry(&format!("MG-C{n:02}"), IdKind::FindingCritical, n as usize))
            .collect();
        let reg = Registry { entries };
        let issues = check_ranges(&reg);
        let gap: Vec<_> = issues.iter().filter(|i| i.id == "MG-C04").collect();
        assert_eq!(gap.len(), 1, "MG-C04 gap flagged");
        assert_eq!(gap[0].kind, IntegrityIssueKind::InvalidRange);
        assert_eq!(gap[0].line, 0, "gap has no definition line");
    }

    // -- Predecessor gaps -----------------------------------------------------

    #[test]
    fn synthetic_bad_gate_order_is_detected() {
        // Gates F0, F1, F3 present — F2 missing, so F3 has a predecessor gap.
        let reg = Registry {
            entries: vec![
                entry("F0", IdKind::Gate, 1),
                entry("F1", IdKind::Gate, 2),
                entry("F3", IdKind::Gate, 3),
            ],
        };
        let issues = check_predecessor_gaps(&reg);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, "F3");
        assert_eq!(issues[0].kind, IntegrityIssueKind::PredecessorGap);
        assert!(issues[0].reason.contains("F2"));
    }

    #[test]
    fn contiguous_gate_chain_has_no_predecessor_gaps() {
        let reg = Registry {
            entries: (0..=6)
                .map(|n| entry(&format!("F{n}"), IdKind::Gate, n as usize + 1))
                .collect(),
        };
        assert!(check_predecessor_gaps(&reg).is_empty());
    }

    // -- Reverse orphans ------------------------------------------------------

    #[test]
    fn synthetic_reverse_orphan_suite_is_detected() {
        let reg = Registry {
            entries: vec![
                entry("V-GOV-01", IdKind::Suite, 1),
                entry("V-ORPH-01", IdKind::Suite, 2),
                entry("R-GOV", IdKind::Risk, 3),
                entry("W-GOV", IdKind::Workstream, 4),
                entry("A-GOV", IdKind::ArtifactClass, 5),
            ],
        };
        // Only V-GOV-01, R-GOV, W-GOV, A-GOV are governed by an MGR row.
        let forward = forward_governing(&["W-GOV"], &["V-GOV-01"], &["R-GOV"], &["A-GOV"]);
        let issues = check_reverse_orphans(&reg, &forward);
        assert_eq!(issues.len(), 1, "only V-ORPH-01 is ungoverned");
        assert_eq!(issues[0].id, "V-ORPH-01");
        assert_eq!(issues[0].kind, IntegrityIssueKind::ReverseOrphan);
    }

    // -- Undefined codes ------------------------------------------------------

    #[test]
    fn synthetic_undefined_reference_is_detected() {
        let reg = Registry {
            entries: vec![entry("R-REAL", IdKind::Risk, 1)],
        };
        let docs = vec![(
            "requirements.md".to_string(),
            "maps to R-REAL and stray R-GHOST-01 reference\n".to_string(),
        )];
        let issues = check_undefined_codes(&reg, &docs);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, "R-GHOST-01");
        assert_eq!(issues[0].kind, IntegrityIssueKind::UndefinedCode);
        assert_eq!(issues[0].source_file, "requirements.md");
        assert_eq!(issues[0].line, 1);
    }

    #[test]
    fn suite_family_reference_resolves_to_numbered_definition() {
        // Roadmap-style family reference `V-AUTH` must resolve to `V-AUTH-01`.
        let reg = Registry {
            entries: vec![
                entry("V-AUTH-01", IdKind::Suite, 1),
                entry("V-AUTH-02", IdKind::Suite, 2),
            ],
        };
        let docs = vec![(
            "implementation-roadmap.md".to_string(),
            "exit artifacts: V-AUTH, V-AUTH-02, V-GHOST\n".to_string(),
        )];
        let issues = check_undefined_codes(&reg, &docs);
        assert_eq!(issues.len(), 1, "only V-GHOST is undefined: {issues:#?}");
        assert_eq!(issues[0].id, "V-GHOST");
    }

    #[test]
    fn mermaid_arrow_token_is_not_a_governed_reference() {
        // `R-->>P` (a mermaid sequence arrow) must not be read as a risk code.
        assert!(classify_reference("R--").is_none());
        assert!(classify_reference("R-").is_none());
        assert!(classify_reference("V--").is_none());
        assert_eq!(classify_reference("R-DATA-01"), Some(RefFamily::Risk));
        assert_eq!(classify_reference("V-AUTH-01"), Some(RefFamily::Suite));
        assert_eq!(classify_reference("A-MAN"), Some(RefFamily::ArtifactClass));
        assert_eq!(classify_reference("F3"), Some(RefFamily::Gate));
        assert_eq!(classify_reference("mg-unit-v2"), Some(RefFamily::Fixture));
    }

    // -- Status without manifest ---------------------------------------------

    #[test]
    fn synthetic_nonplanned_status_without_manifest_is_detected() {
        let content = concat!(
            "| MGR-001 x | d | w | v | r | F0 | A-MAN | Verified |\n",
            "| MGR-002 x | d | w | v | r | F0 | A-MAN | Planned/Unverified |\n",
        );
        let issues = check_status_manifests("traceability.md", content);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, "MGR-001");
        assert_eq!(issues[0].kind, IntegrityIssueKind::StatusWithoutManifest);
    }

    #[test]
    fn nonplanned_status_with_manifest_reference_passes() {
        let content =
            "| MGR-001 x | d | w | v | r | F0 | evidence/F1/run-1/manifest.json | Verified |\n";
        assert!(check_status_manifests("traceability.md", content).is_empty());
    }

    // -- Clean synthetic world passes ----------------------------------------

    /// Build a complete, self-consistent registry with no integrity defects.
    fn clean_registry() -> Registry {
        let mut entries = Vec::new();
        let mut push_family = |prefix: &str, width: usize, kind: IdKind, max: u32| {
            for n in 1..=max {
                let id = if width == 3 {
                    format!("{prefix}{n:03}")
                } else {
                    format!("{prefix}{n:02}")
                };
                entries.push(entry(&id, kind, entries.len() + 1));
            }
        };
        push_family("MGR-", 3, IdKind::Requirement, 48);
        push_family("MGD-", 3, IdKind::Decision, 46);
        push_family("MG-C", 2, IdKind::FindingCritical, 7);
        push_family("MG-H", 2, IdKind::FindingHigh, 17);
        push_family("MG-M", 2, IdKind::FindingMedium, 28);
        push_family("MG-L", 2, IdKind::FindingLow, 13);
        push_family("MG-O", 2, IdKind::Opportunity, 31);
        for n in 0..=6 {
            entries.push(entry(&format!("F{n}"), IdKind::Gate, entries.len() + 1));
        }
        entries.push(entry("V-X-01", IdKind::Suite, entries.len() + 1));
        entries.push(entry("R-X", IdKind::Risk, entries.len() + 1));
        entries.push(entry("W-X", IdKind::Workstream, entries.len() + 1));
        entries.push(entry("A-X", IdKind::ArtifactClass, entries.len() + 1));
        Registry { entries }
    }

    #[test]
    fn clean_synthetic_world_produces_no_issues() {
        let reg = clean_registry();
        let forward = forward_governing(&["W-X"], &["V-X-01"], &["R-X"], &["A-X"]);
        let docs: Vec<(String, String)> = Vec::new();
        let v = validate(&reg, &forward, &docs, "");
        assert!(v.is_ok(), "expected clean world, got: {:#?}", v.issues);
    }

    // -- Negative golden inputs (task 0.1.4) ---------------------------------
    //
    // Each on-disk fixture under `tests/fixtures/memory-graph/` carries exactly
    // one planted defect; these tests assert it fails for its intended reason
    // (exact issue kind + id).

    /// Read a negative golden-input fixture fragment by file name.
    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/memory-graph")
            .join(name);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
    }

    /// Build a registry from a single fixture fragment routed to `role`.
    fn registry_from(role: &str, content: &str) -> Registry {
        Registry::from_documents(&[(role.to_string(), content.to_string())])
    }

    #[test]
    fn golden_duplicate_id_fixture_fails() {
        let reg = registry_from("decisions.md", &fixture("duplicate-id.decisions.md"));
        let issues = check_duplicates(&reg);
        assert!(
            issues
                .iter()
                .any(|i| i.kind == IntegrityIssueKind::DuplicateId && i.id == "MGD-005"),
            "expected duplicate_id MGD-005, got: {issues:#?}"
        );
    }

    #[test]
    fn golden_out_of_range_fixture_fails() {
        let reg = registry_from("requirements.md", &fixture("out-of-range.requirements.md"));
        let issues = check_ranges(&reg);
        let hit = issues
            .iter()
            .find(|i| i.kind == IntegrityIssueKind::InvalidRange && i.id == "MGR-049")
            .expect("expected invalid_range MGR-049");
        assert!(hit.reason.contains("out of range"), "{}", hit.reason);
    }

    #[test]
    fn golden_reverse_orphan_fixture_fails() {
        let reg = registry_from("validation.md", &fixture("reverse-orphan.validation.md"));
        let forward = ForwardValidation::default();
        let issues = check_reverse_orphans(&reg, &forward);
        assert!(
            issues
                .iter()
                .any(|i| i.kind == IntegrityIssueKind::ReverseOrphan && i.id == "V-ORPHAN-01"),
            "expected reverse_orphan V-ORPHAN-01, got: {issues:#?}"
        );
    }

    #[test]
    fn golden_bad_gate_order_fixture_fails() {
        let reg = registry_from(
            "implementation-roadmap.md",
            &fixture("bad-gate-order.roadmap.md"),
        );
        let issues = check_predecessor_gaps(&reg);
        assert!(
            issues
                .iter()
                .any(|i| i.kind == IntegrityIssueKind::PredecessorGap && i.id == "F3"),
            "expected predecessor_gap F3, got: {issues:#?}"
        );
    }

    #[test]
    fn golden_undefined_code_fixture_fails() {
        let content = fixture("undefined-code.requirements.md");
        let reg = registry_from("requirements.md", &content);
        let docs = vec![("requirements.md".to_string(), content.clone())];
        let issues = check_undefined_codes(&reg, &docs);
        assert!(
            issues
                .iter()
                .any(|i| i.kind == IntegrityIssueKind::UndefinedCode && i.id == "R-GHOST-01"),
            "expected undefined_code R-GHOST-01, got: {issues:#?}"
        );
        // The locally-defined MGR-001 must not itself be reported undefined.
        assert!(
            !issues.iter().any(|i| i.id == "MGR-001"),
            "MGR-001 is defined and must not be undefined"
        );
    }

    #[test]
    fn golden_checklist_only_pass_fixture_fails() {
        let issues = check_status_manifests(
            "traceability.md",
            &fixture("checklist-only-pass.traceability.md"),
        );
        assert!(
            issues
                .iter()
                .any(|i| i.kind == IntegrityIssueKind::StatusWithoutManifest && i.id == "MGR-001"),
            "expected status_without_manifest MGR-001, got: {issues:#?}"
        );
    }

    #[test]
    fn golden_checksum_invalid_fixture_fails() {
        let issues = check_status_manifests(
            "traceability.md",
            &fixture("checksum-invalid.traceability.md"),
        );
        assert!(
            issues
                .iter()
                .any(|i| i.kind == IntegrityIssueKind::StatusWithoutManifest && i.id == "MGD-018"),
            "expected status_without_manifest MGD-018, got: {issues:#?}"
        );
    }
}
