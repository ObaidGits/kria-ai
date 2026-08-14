//! Task 0.1 — Freeze the canonical capability and tool contract inventory.
//!
//! Pure fixture/schema freeze tests for the `linux-os-control-production` spec.
//! These tests parse the frozen contract manifest (`operation-contracts.json`,
//! mirrored as a deterministic fixture) and the normative design tables, then
//! assert a bidirectional design <-> manifest parity oracle plus closed-ID,
//! reverse-orphan, and closed-schema-graph invariants.
//!
//! Invariants proven here (Task 0.1 code-level validation):
//!   * exact 149 tool-name set agreement, both directions;
//!   * per-operation phase, resolved-risk class, output-type resolution, and
//!     §13.1 `rollbackClaim` agreement;
//!   * §13.1 buckets are mutually exclusive and total over every mutation row;
//!   * closed IDs (`os.<tool>`, `oracle.<tool>`, one `OSC-nnn`, one `N.N` task),
//!     one trace edge each, no BLACK operation, `HostLocalOnly` target;
//!   * the closed schema graph is fully reachable with zero dangling/orphan defs;
//!   * requirement/task trace links resolve in requirements.md / tasks.md.
//!
//! This test performs NO production registry mutation and NO provider invocation.
//! It only reads data files. It is gated behind `os-control-test` so it runs under
//! the spec-mandated `--no-default-features --features os-control-test` command.
#![cfg(feature = "os-control-test")]

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const EXPECTED_OPERATION_COUNT: usize = 149;

fn spec_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.kiro/specs/linux-os-control-production")
}

fn fixture_manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/os_control/contract_manifest.json")
}

fn read(path: &PathBuf) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn load_manifest() -> Value {
    serde_json::from_str(&read(&fixture_manifest_path())).expect("fixture manifest is valid JSON")
}

fn operations(manifest: &Value) -> &Vec<Value> {
    manifest["operations"]
        .as_array()
        .expect("operations array present")
}

// ---------------------------------------------------------------------------
// Design-table parsing (no regex; markdown table scan).
// ---------------------------------------------------------------------------

/// A parsed design row from §§10.1–10.3.
#[derive(Debug, Clone)]
struct DesignRow {
    tool: String,
    risk_part: String,
    phase: String,
    output_after_arrow: String,
}

fn table_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim().trim_matches('|');
    trimmed.split('|').map(|c| c.trim().to_string()).collect()
}

fn is_table_data_row(line: &str) -> bool {
    let l = line.trim_start();
    l.starts_with('|') && !l.contains("Canonical tool") && !l.contains("---")
}

/// First backtick-delimited identifier `[a-z0-9_]+` in a cell (stops at `(`).
fn first_backtick_ident(cell: &str) -> Option<String> {
    let bytes = cell.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let mut j = i + 1;
            let mut ident = String::new();
            while j < bytes.len() {
                let c = bytes[j] as char;
                if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
                    ident.push(c);
                    j += 1;
                } else {
                    break;
                }
            }
            if !ident.is_empty() {
                return Some(ident);
            }
        }
        i += 1;
    }
    None
}

fn section_bounds(lines: &[&str], start_prefix: &str, end_prefix: &str) -> (usize, usize) {
    let mut start = None;
    let mut end = None;
    for (i, l) in lines.iter().enumerate() {
        if start.is_none() && l.starts_with(start_prefix) {
            start = Some(i);
        } else if start.is_some() && l.starts_with(end_prefix) {
            end = Some(i);
            break;
        }
    }
    (
        start.unwrap_or_else(|| panic!("section {start_prefix} not found")),
        end.unwrap_or_else(|| panic!("section end {end_prefix} not found")),
    )
}

fn parse_design_rows(design: &str) -> Vec<DesignRow> {
    let lines: Vec<&str> = design.lines().collect();
    let (start, end) = section_bounds(&lines, "### 10.1", "### 10.4");
    let mut rows = Vec::new();
    for line in &lines[start..end] {
        if !is_table_data_row(line) {
            continue;
        }
        let cells = table_cells(line);
        if cells.len() < 3 {
            continue;
        }
        let (col1, col2, col3) = (&cells[0], &cells[1], &cells[2]);
        let tool = match first_backtick_ident(col1) {
            Some(t) => t,
            None => continue,
        };
        // phase: token after the LAST '/'; risk part is everything before it.
        let idx = col3
            .rfind('/')
            .unwrap_or_else(|| panic!("row {tool}: no '/phase' in risk cell {col3:?}"));
        let phase = col3[idx + 1..].trim().to_string();
        let risk_part = col3[..idx].trim().to_string();
        let output_after_arrow = match col2.rsplit('→').next() {
            Some(s) => s.trim().to_string(),
            None => col2.clone(),
        };
        rows.push(DesignRow {
            tool,
            risk_part,
            phase,
            output_after_arrow,
        });
    }
    rows
}

/// Parse §13.1 rollback buckets: claim -> member tool names (backtick tokens).
fn parse_rollback_buckets(design: &str) -> BTreeMap<String, Vec<String>> {
    let lines: Vec<&str> = design.lines().collect();
    let (start, end) = section_bounds(&lines, "### 13.1", "## 14");
    let claims = ["Automatic", "UserRequestable", "CompensationOnly", "None"];
    let mut buckets: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in &lines[start..end] {
        if !line.trim_start().starts_with('|') || !line.contains('`') {
            continue;
        }
        let cells = table_cells(line);
        if cells.len() != 2 {
            continue;
        }
        let claim = match first_backtick_ident_any(&cells[0], &claims) {
            Some(c) => c,
            None => continue,
        };
        let members = all_backtick_idents(&cells[1]);
        buckets.entry(claim).or_default().extend(members);
    }
    buckets
}

/// First backticked token in a cell that matches one of `allowed` (case-sensitive,
/// allows leading capital claim names).
fn first_backtick_ident_any(cell: &str, allowed: &[&str]) -> Option<String> {
    for tok in all_backtick_tokens(cell) {
        if allowed.contains(&tok.as_str()) {
            return Some(tok);
        }
    }
    None
}

/// All backtick-delimited tokens (any characters between backticks).
fn all_backtick_tokens(cell: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = cell.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            if let Some(close) = cell[i + 1..].find('`') {
                out.push(cell[i + 1..i + 1 + close].to_string());
                i = i + 1 + close + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// All backtick tokens that are lower_snake identifiers (tool names).
fn all_backtick_idents(cell: &str) -> Vec<String> {
    all_backtick_tokens(cell)
        .into_iter()
        .filter(|t| {
            !t.is_empty()
                && t.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Risk classification.
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum RiskClass {
    Fixed(String),
    Conditional,
}

fn design_risk_class(risk_part: &str) -> RiskClass {
    match risk_part {
        "GREEN" => RiskClass::Fixed("GREEN".into()),
        "YELLOW" => RiskClass::Fixed("YELLOW".into()),
        "RED" => RiskClass::Fixed("RED".into()),
        s if s.starts_with("RED because") => RiskClass::Fixed("RED".into()),
        _ => RiskClass::Conditional,
    }
}

fn json_risk_class(op: &Value) -> RiskClass {
    let id = op["riskFunctionId"].as_str().unwrap_or("");
    if let Some(level) = id.strip_prefix("risk.fixed.") {
        RiskClass::Fixed(level.to_ascii_uppercase())
    } else {
        RiskClass::Conditional
    }
}

fn json_risk_outcomes(op: &Value) -> BTreeSet<String> {
    op["riskRules"]
        .as_array()
        .map(|rules| {
            rules
                .iter()
                .filter_map(|r| {
                    r.get("risk")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn design_risk_tokens(risk_part: &str) -> BTreeSet<String> {
    ["GREEN", "YELLOW", "RED"]
        .into_iter()
        .filter(|w| risk_part.contains(*w))
        .map(|w| w.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Output-type classification.
// ---------------------------------------------------------------------------

fn design_is_mutation(output_after_arrow: &str) -> bool {
    output_after_arrow.to_ascii_lowercase().contains("receipt")
}

fn json_output_ref(op: &Value) -> String {
    op["outputSchema"]["$ref"]
        .as_str()
        .unwrap_or("")
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// Schema-graph helpers.
// ---------------------------------------------------------------------------

fn collect_refs(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if k == "$ref" {
                    if let Some(s) = v.as_str() {
                        out.insert(s.rsplit('/').next().unwrap_or(s).to_string());
                    }
                } else {
                    collect_refs(v, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_refs(item, out);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

/// Drift guard: the fixture under tests/fixtures MUST equal the spec manifest.
#[test]
fn fixture_matches_spec_manifest_no_drift() {
    let fixture: Value = load_manifest();
    let spec: Value = serde_json::from_str(&read(&spec_dir().join("operation-contracts.json")))
        .expect("spec manifest is valid JSON");
    assert_eq!(
        fixture, spec,
        "contract-manifest fixture has drifted from the spec operation-contracts.json; \
         regenerate the fixture (see tests/fixtures/os_control/README.md)"
    );
}

/// Manifest self-consistency: count, closed IDs, single trace edges, no BLACK,
/// HostLocalOnly, no duplicates, no placeholders.
#[test]
fn manifest_closed_ids_and_invariants() {
    let manifest = load_manifest();
    let ops = operations(&manifest);
    let mut failures: Vec<String> = Vec::new();

    assert_eq!(
        manifest["operationCount"].as_u64(),
        Some(EXPECTED_OPERATION_COUNT as u64),
        "declared operationCount must be {EXPECTED_OPERATION_COUNT}"
    );
    assert_eq!(
        ops.len(),
        EXPECTED_OPERATION_COUNT,
        "operations array must contain exactly {EXPECTED_OPERATION_COUNT} rows"
    );

    let placeholders = ["tbd", "todo", "fixme", "placeholder", "xxx", "tbc"];
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();

    for op in ops {
        let tool = op["toolName"].as_str().unwrap_or("<none>").to_string();
        let id = op["id"].as_str().unwrap_or("");
        if id != format!("os.{tool}") {
            failures.push(format!("{tool}: id {id:?} != os.{tool}"));
        }
        let oracle = op["oracleId"].as_str().unwrap_or("");
        if oracle != format!("oracle.{tool}") {
            failures.push(format!("{tool}: oracleId {oracle:?} != oracle.{tool}"));
        }
        let req = op["requirementId"].as_str().unwrap_or("");
        if !(req.len() == 7
            && req.starts_with("OSC-")
            && req[4..].chars().all(|c| c.is_ascii_digit()))
        {
            failures.push(format!("{tool}: requirementId {req:?} not OSC-nnn"));
        }
        let task = op["taskId"].as_str().unwrap_or("");
        let task_ok = {
            let parts: Vec<&str> = task.split('.').collect();
            parts.len() == 2
                && !parts[0].is_empty()
                && !parts[1].is_empty()
                && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()))
        };
        if !task_ok {
            failures.push(format!(
                "{tool}: taskId {task:?} not a single N.N (range/phase forbidden)"
            ));
        }
        if op["target"].as_str() != Some("HostLocalOnly") {
            failures.push(format!("{tool}: target != HostLocalOnly"));
        }
        for rule in op["riskRules"].as_array().into_iter().flatten() {
            if rule.get("risk").and_then(|v| v.as_str()) == Some("BLACK") {
                failures.push(format!("{tool}: BLACK risk present"));
            }
        }
        let blob = serde_json::to_string(op)
            .unwrap_or_default()
            .to_ascii_lowercase();
        for ph in placeholders {
            if blob.contains(ph) {
                failures.push(format!("{tool}: placeholder token {ph:?} present"));
            }
        }
        if !ids.insert(id.to_string()) {
            failures.push(format!("duplicate operation id {id:?}"));
        }
        if !names.insert(tool.clone()) {
            failures.push(format!("duplicate tool name {tool:?}"));
        }
    }

    assert!(
        failures.is_empty(),
        "manifest invariant failures:\n{}",
        failures.join("\n")
    );
}

/// Bidirectional design <-> manifest parity: tool set, phase, resolved risk, and
/// output-type resolution.
#[test]
fn design_manifest_parity_oracle() {
    let manifest = load_manifest();
    let ops = operations(&manifest);
    let design = read(&spec_dir().join("design.md"));
    let rows = parse_design_rows(&design);

    let mut failures: Vec<String> = Vec::new();

    // Tool-name set parity (both directions), exact count.
    assert_eq!(
        rows.len(),
        EXPECTED_OPERATION_COUNT,
        "design §§10.1–10.3 must contain {EXPECTED_OPERATION_COUNT} rows, found {}",
        rows.len()
    );
    let design_names: BTreeSet<String> = rows.iter().map(|r| r.tool.clone()).collect();
    let json_names: BTreeSet<String> = ops
        .iter()
        .map(|o| o["toolName"].as_str().unwrap_or("").to_string())
        .collect();
    let in_design_not_json: Vec<_> = design_names.difference(&json_names).cloned().collect();
    let in_json_not_design: Vec<_> = json_names.difference(&design_names).cloned().collect();
    if !in_design_not_json.is_empty() {
        failures.push(format!("in design not manifest: {in_design_not_json:?}"));
    }
    if !in_json_not_design.is_empty() {
        failures.push(format!("in manifest not design: {in_json_not_design:?}"));
    }

    let ops_by_tool: BTreeMap<String, &Value> = ops
        .iter()
        .map(|o| (o["toolName"].as_str().unwrap_or("").to_string(), o))
        .collect();
    let schema_defs = &manifest["schemaDefinitions"];

    for row in &rows {
        let op = match ops_by_tool.get(&row.tool) {
            Some(o) => *o,
            None => continue, // set-diff already reported
        };

        // Phase parity.
        let jphase = op["phase"].as_str().unwrap_or("");
        if jphase != row.phase {
            failures.push(format!(
                "{}: phase design={} manifest={}",
                row.tool, row.phase, jphase
            ));
        }

        // Resolved-risk parity.
        let dclass = design_risk_class(&row.risk_part);
        let jclass = json_risk_class(op);
        if dclass != jclass {
            failures.push(format!(
                "{}: risk class design={:?} manifest={:?} (fn {})",
                row.tool,
                dclass,
                jclass,
                op["riskFunctionId"].as_str().unwrap_or("")
            ));
        } else if let RiskClass::Conditional = dclass {
            let outcomes = json_risk_outcomes(op);
            if outcomes.contains("BLACK") {
                failures.push(format!("{}: conditional risk resolves to BLACK", row.tool));
            }
            if !outcomes
                .iter()
                .all(|o| ["GREEN", "YELLOW", "RED"].contains(&o.as_str()))
            {
                failures.push(format!(
                    "{}: conditional risk outcome outside G/Y/R: {outcomes:?}",
                    row.tool
                ));
            }
            let dtokens = design_risk_tokens(&row.risk_part);
            for t in &dtokens {
                if !outcomes.contains(t) {
                    failures.push(format!(
                        "{}: design names risk {t} but manifest outcomes {outcomes:?} omit it",
                        row.tool
                    ));
                }
            }
        }

        // Output-type resolution parity.
        let dmut = design_is_mutation(&row.output_after_arrow);
        let out_ref = json_output_ref(op);
        let jmut = out_ref.starts_with("MutationReceipt_Result_");
        if dmut != jmut {
            failures.push(format!(
                "{}: mutation/read mismatch design_mut={} manifest_ref={}",
                row.tool, dmut, out_ref
            ));
            continue;
        }
        if dmut {
            // If the design names MutationReceipt<X>, the manifest ReceiptState must reference X.
            if let Some(inner) = extract_receipt_inner(&row.output_after_arrow) {
                let rs_name = format!("ReceiptState_{}", row.tool);
                let rs_text = schema_defs
                    .get(&rs_name)
                    .map(|v| serde_json::to_string(v).unwrap_or_default())
                    .unwrap_or_default();
                if !rs_text.contains(&inner) {
                    failures.push(format!(
                        "{}: design output MutationReceipt<{}> not reflected in {}",
                        row.tool, inner, rs_name
                    ));
                }
            }
        } else {
            let after_low = row.output_after_arrow.to_ascii_lowercase();
            let named = first_backtick_ident_typelike(&row.output_after_arrow);
            if let Some(dtype) = named {
                if dtype != out_ref {
                    failures.push(format!(
                        "{}: read output design={} manifest={}",
                        row.tool, dtype, out_ref
                    ));
                }
            } else if after_low.contains("metadata only") || after_low.contains("page") {
                if !(out_ref.ends_with("Page") || out_ref.contains("Metadata")) {
                    failures.push(format!(
                        "{}: page/metadata read expected Page/Metadata type, got {}",
                        row.tool, out_ref
                    ));
                }
            } else {
                failures.push(format!(
                    "{}: unparseable read output {:?} -> manifest {}",
                    row.tool, row.output_after_arrow, out_ref
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "design<->manifest parity failures:\n{}",
        failures.join("\n")
    );
}

/// Extract `X` from a `MutationReceipt<X>` phrase, if present.
fn extract_receipt_inner(after: &str) -> Option<String> {
    let start = after.find("MutationReceipt<")? + "MutationReceipt<".len();
    let rest = &after[start..];
    let end = rest.find('>')?;
    Some(rest[..end].to_string())
}

/// First backticked token in an output cell that looks like a Type identifier
/// (starts uppercase, alnum/underscore).
fn first_backtick_ident_typelike(cell: &str) -> Option<String> {
    for tok in all_backtick_tokens(cell) {
        let mut chars = tok.chars();
        if let Some(first) = chars.next() {
            if first.is_ascii_uppercase()
                && tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Some(tok);
            }
        }
    }
    None
}

/// §13.1 rollback-claim parity + bucket mutual-exclusivity and totality over
/// every mutation row.
#[test]
fn rollback_buckets_exclusive_total_and_parity() {
    let manifest = load_manifest();
    let ops = operations(&manifest);
    let design = read(&spec_dir().join("design.md"));
    let rows = parse_design_rows(&design);
    let buckets = parse_rollback_buckets(&design);

    let mut failures: Vec<String> = Vec::new();

    // Mutation/read partition from design output classification.
    let mutations: BTreeSet<String> = rows
        .iter()
        .filter(|r| design_is_mutation(&r.output_after_arrow))
        .map(|r| r.tool.clone())
        .collect();
    let reads: BTreeSet<String> = rows
        .iter()
        .filter(|r| !design_is_mutation(&r.output_after_arrow))
        .map(|r| r.tool.clone())
        .collect();

    // tool -> claims (to detect double-bucketing).
    let mut tool_claims: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (claim, members) in &buckets {
        for m in members {
            tool_claims
                .entry(m.clone())
                .or_default()
                .push(claim.clone());
        }
    }

    // Mutual exclusivity: no tool in more than one bucket.
    for (tool, claims) in &tool_claims {
        if claims.len() > 1 {
            failures.push(format!("{tool}: double-bucketed in §13.1: {claims:?}"));
        }
    }

    // Totality over mutation rows: every mutation appears in exactly one bucket.
    for m in &mutations {
        match tool_claims.get(m) {
            None => failures.push(format!("{m}: mutation row not present in any §13.1 bucket")),
            Some(c) if c.len() != 1 => failures.push(format!(
                "{m}: mutation row not in exactly one bucket: {c:?}"
            )),
            _ => {}
        }
    }
    // No read row may occupy a bucket.
    for (tool, _) in &tool_claims {
        if reads.contains(tool) {
            failures.push(format!(
                "{tool}: read row must not appear in a §13.1 rollback bucket"
            ));
        }
    }

    // Per-tool parity: manifest rollbackClaim == design bucket for mutations;
    // reads must be `None`.
    let ops_by_tool: BTreeMap<String, &Value> = ops
        .iter()
        .map(|o| (o["toolName"].as_str().unwrap_or("").to_string(), o))
        .collect();
    for (tool, op) in &ops_by_tool {
        let jclaim = op["rollbackClaim"].as_str().unwrap_or("");
        if mutations.contains(tool) {
            let dclaim = tool_claims
                .get(tool)
                .and_then(|c| c.first())
                .cloned()
                .unwrap_or_default();
            if dclaim != jclaim {
                failures.push(format!(
                    "{tool}: rollbackClaim design={dclaim:?} manifest={jclaim:?}"
                ));
            }
        } else if jclaim != "None" {
            failures.push(format!(
                "{tool}: read row rollbackClaim must be None, got {jclaim:?}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "§13.1 rollback bucket failures:\n{}",
        failures.join("\n")
    );
}

/// Closed schema graph: zero dangling refs, zero orphan definitions.
#[test]
fn schema_graph_is_closed_and_reachable() {
    let manifest = load_manifest();
    let schema_defs = manifest["schemaDefinitions"]
        .as_object()
        .expect("schemaDefinitions object present");
    let defined: BTreeSet<String> = schema_defs.keys().cloned().collect();

    // No dangling refs anywhere in the document.
    let mut all_refs = BTreeSet::new();
    collect_refs(&manifest, &mut all_refs);
    let dangling: Vec<_> = all_refs.difference(&defined).cloned().collect();
    assert!(
        dangling.is_empty(),
        "dangling $ref (referenced but undefined): {dangling:?}"
    );

    // Reachability from operation input/output schemas.
    let mut seen = BTreeSet::new();
    let mut stack: Vec<String> = Vec::new();
    for op in operations(&manifest) {
        let mut roots = BTreeSet::new();
        collect_refs(&op["inputSchema"], &mut roots);
        collect_refs(&op["outputSchema"], &mut roots);
        stack.extend(roots);
    }
    while let Some(name) = stack.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(def) = schema_defs.get(&name) {
            let mut child = BTreeSet::new();
            collect_refs(def, &mut child);
            for c in child {
                if !seen.contains(&c) {
                    stack.push(c);
                }
            }
        }
    }
    let orphans: Vec<_> = defined.difference(&seen).cloned().collect();
    assert!(
        orphans.is_empty(),
        "orphan schema definitions (unreachable from any operation): {orphans:?}"
    );
}

/// Reverse-orphan across documents: every requirement/task trace link resolves.
#[test]
fn trace_links_resolve_in_requirements_and_tasks() {
    let manifest = load_manifest();
    let ops = operations(&manifest);
    let requirements = read(&spec_dir().join("requirements.md"));
    let tasks = read(&spec_dir().join("tasks.md"));

    // Defined OSC-nnn ids in requirements.md (byte scan; safe across multibyte chars).
    let mut defined_reqs = BTreeSet::new();
    let rbytes = requirements.as_bytes();
    let mut i = 0;
    while i + 7 <= rbytes.len() {
        if &rbytes[i..i + 4] == b"OSC-" && rbytes[i + 4..i + 7].iter().all(|b| b.is_ascii_digit()) {
            // All seven bytes are ASCII, so this slice is a valid str boundary.
            defined_reqs.insert(String::from_utf8_lossy(&rbytes[i..i + 7]).into_owned());
        }
        i += 1;
    }

    // Defined task ids from headings like "- [ ] 3.5 ..." / "- [x] 1.2 ...".
    let mut defined_tasks = BTreeSet::new();
    for line in tasks.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("- [") {
            // rest = "x] 3.5 ..." — skip the status char and "] ".
            if let Some(after) = rest.get(1..).and_then(|s| s.strip_prefix("] ")) {
                let token: String = after.chars().take_while(|c| !c.is_whitespace()).collect();
                let parts: Vec<&str> = token.split('.').collect();
                if parts.len() == 2
                    && !parts[0].is_empty()
                    && !parts[1].is_empty()
                    && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()))
                {
                    defined_tasks.insert(token);
                }
            }
        }
    }

    let mut failures: Vec<String> = Vec::new();
    for op in ops {
        let tool = op["toolName"].as_str().unwrap_or("");
        let req = op["requirementId"].as_str().unwrap_or("");
        if !defined_reqs.contains(req) {
            failures.push(format!(
                "{tool}: requirementId {req} not defined in requirements.md"
            ));
        }
        let task = op["taskId"].as_str().unwrap_or("");
        if !defined_tasks.contains(task) {
            failures.push(format!("{tool}: taskId {task} not defined in tasks.md"));
        }
    }
    assert!(
        failures.is_empty(),
        "trace-link reverse-orphan failures:\n{}",
        failures.join("\n")
    );
}
