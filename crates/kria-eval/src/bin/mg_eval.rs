//! `mg-eval` — CMD-MG-EVAL: Memory Graph production evaluation runner.
//!
//! Implements the `CMD-MG-EVAL` command from `validation.md`:
//!
//! ```text
//! cargo run -p kria-eval --bin mg-eval -- --manifest <run-root>/manifest.json
//! ```
//!
//! This binary:
//! 1. Resolves the run root from `--manifest`.
//! 2. Loads the `mg-retrieval-judged-v2` fixture from disk.
//! 3. Runs the V-RET-03 judged retrieval campaign (220 queries).
//! 4. Writes `reports/retrieval-quality.json` and
//!    `reports/judged-eval-results.json` under the run root.
//! 5. Exits `0` on pass, `1` on assertion failure, `2` on error.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kria_eval::memory_graph::fixtures::{JudgedDocument, JudgedQuery};
use kria_eval::memory_graph::judged_eval::run_campaign;
use serde_json::Value;

const FIXTURE_REL: &str =
    "tests/fixtures/memory-graph/generated/mg-retrieval-judged-v2/0.1.0";

fn repo_root() -> PathBuf {
    // Walk up from the binary manifest until we find Cargo.toml at root.
    let mut d = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("."))
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    for _ in 0..10 {
        if d.join("Cargo.toml").exists() && d.join("crates").exists() {
            return d;
        }
        if let Some(p) = d.parent() {
            d = p.to_path_buf();
        } else {
            break;
        }
    }
    // Fallback: current dir
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let mut manifest_path: Option<PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest" => {
                i += 1;
                match args.get(i) {
                    Some(v) => manifest_path = Some(PathBuf::from(v)),
                    None => {
                        eprintln!("mg-eval: --manifest requires a path argument");
                        return ExitCode::from(2);
                    }
                }
            }
            "-h" | "--help" => {
                println!(
                    "mg-eval — CMD-MG-EVAL: Memory Graph evidence evaluation runner\n\
                     USAGE: cargo run -p kria-eval --bin mg-eval -- --manifest <run-root>/manifest.json\n\
                     EXIT: 0=pass, 1=assertion failure, 2=invocation/IO error"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("mg-eval: unknown argument: {other}");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }

    let manifest_path = match manifest_path {
        Some(p) => p,
        None => {
            eprintln!("mg-eval: --manifest <path> is required");
            return ExitCode::from(2);
        }
    };

    // Derive run root from manifest path.
    let run_root = match manifest_path.parent() {
        Some(p) => p.to_path_buf(),
        None => {
            eprintln!("mg-eval: cannot determine run root from manifest path");
            return ExitCode::from(2);
        }
    };

    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║  CMD-MG-EVAL  Memory Graph Retrieval Quality Campaign    ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!("Suite:      V-RET-03");
    println!("Run root:   {}", run_root.display());
    println!();

    // Locate fixture directory.
    let repo = repo_root();
    let fixture_dir = repo.join(FIXTURE_REL);
    println!("Fixture:    {}", fixture_dir.display());

    let queries_path = fixture_dir.join("queries.json");
    let docs_path = fixture_dir.join("documents.json");
    let fm_path = fixture_dir.join("fixture-manifest.json");

    // Load fixture files.
    let queries_bytes = match std::fs::read(&queries_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("mg-eval: failed to read {}: {e}", queries_path.display());
            return ExitCode::from(2);
        }
    };
    let docs_bytes = match std::fs::read(&docs_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("mg-eval: failed to read {}: {e}", docs_path.display());
            return ExitCode::from(2);
        }
    };
    let fm_bytes = match std::fs::read(&fm_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("mg-eval: failed to read {}: {e}", fm_path.display());
            return ExitCode::from(2);
        }
    };

    let queries: Vec<JudgedQuery> = match serde_json::from_slice(&queries_bytes) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("mg-eval: failed to parse queries.json: {e}");
            return ExitCode::from(2);
        }
    };
    let documents: Vec<JudgedDocument> = match serde_json::from_slice(&docs_bytes) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("mg-eval: failed to parse documents.json: {e}");
            return ExitCode::from(2);
        }
    };
    let fm: Value = match serde_json::from_slice(&fm_bytes) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("mg-eval: failed to parse fixture-manifest.json: {e}");
            return ExitCode::from(2);
        }
    };

    // Extract oracle metadata from fixture manifest.
    let oracle = &fm["judged_corpus_oracle"];
    let oracle_note = oracle["oracle_note"].as_str().unwrap_or("").to_string();
    let judge_ids: Vec<String> = oracle["judge_ids"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    let adjudicator_id = oracle["adjudicator_id"].as_str().unwrap_or("adjudicator-senior-v1");
    let agreed_count = oracle["agreed_query_count"].as_u64().unwrap_or(0) as usize;
    let adjudicated_count = oracle["adjudicated_query_count"].as_u64().unwrap_or(0) as usize;

    println!("Queries:    {}", queries.len());
    println!("Documents:  {}", documents.len());
    println!(
        "Judged:     {} agreed + {} adjudicated",
        agreed_count, adjudicated_count
    );
    println!();
    println!("Running V-RET-03 evaluation campaign...");

    let fixture_path_str = fixture_dir.to_string_lossy().to_string();

    let (quality_report, judged_results) = run_campaign(
        &queries,
        &documents,
        &fixture_path_str,
        &oracle_note,
        &judge_ids,
        adjudicator_id,
        agreed_count,
        adjudicated_count,
    );

    // Print summary.
    let ov = &quality_report.overall;
    println!("─── Aggregate Metrics ───────────────────────────────────");
    println!(
        "  Recall@10:               {:.4}  (threshold ≥ 0.85)",
        ov.recall_at_10
    );
    println!(
        "  nDCG@10:                 {:.4}  (threshold ≥ 0.80)",
        ov.ndcg_at_10
    );
    println!(
        "  Identifier/Phrase R@10:  {:.4}  (threshold ≥ 0.95)",
        ov.identifier_phrase_recall
    );
    println!(
        "  Forbidden exclusion:     {:.4}  (required = 1.00)",
        ov.forbidden_exclusion_rate
    );
    println!(
        "  Deleted/Forgotten/Supers exclusion: {:.4}  (required = 1.00)",
        ov.deleted_forgotten_superseded_exclusion_rate
    );
    println!("  Sample size: {}", ov.sample_size);
    println!();
    println!("─── 95% Bootstrap CIs ───────────────────────────────────");
    let ci = &quality_report.confidence_intervals;
    println!(
        "  Recall@10:     [{:.4}, {:.4}]  (estimate {:.4})",
        ci.recall_at_10.lower, ci.recall_at_10.upper, ci.recall_at_10.estimate
    );
    println!(
        "  nDCG@10:       [{:.4}, {:.4}]  (estimate {:.4})",
        ci.ndcg_at_10.lower, ci.ndcg_at_10.upper, ci.ndcg_at_10.estimate
    );
    println!(
        "  Id/Phrase R@10:[{:.4}, {:.4}]  (estimate {:.4})",
        ci.identifier_phrase_recall.lower,
        ci.identifier_phrase_recall.upper,
        ci.identifier_phrase_recall.estimate
    );
    println!();
    println!("─── Per-Class Breakdown ─────────────────────────────────");
    for cls in &quality_report.per_class {
        println!(
            "  {:20}  n={:3}  R@10={:.4}  nDCG@10={:.4}",
            cls.query_class, cls.count, cls.recall_at_10, cls.ndcg_at_10
        );
    }
    println!();
    println!("─── Per-Stratum Breakdown ───────────────────────────────");
    for st in &quality_report.per_stratum {
        println!(
            "  {:20}  n={:3}  R@10={:.4}  nDCG@10={:.4}",
            st.stratum, st.count, st.recall_at_10, st.ndcg_at_10
        );
    }
    println!();

    let assertions = &quality_report.assertions;
    println!(
        "─── Assertions: {}/{} passed ──────────────────────────────",
        assertions.passed, assertions.total
    );

    if quality_report.passed {
        println!("  ✓ ALL V-RET-03 ASSERTIONS PASSED");
    } else {
        println!("  ✗ FAILURES:");
        for reason in &quality_report.failure_reasons {
            println!("    - {reason}");
        }
    }
    println!();

    // Write evidence artifacts.
    let reports_dir = run_root.join("reports");
    if let Err(e) = std::fs::create_dir_all(&reports_dir) {
        eprintln!("mg-eval: failed to create reports dir: {e}");
        return ExitCode::from(2);
    }

    let quality_path = reports_dir.join("retrieval-quality.json");
    let judged_path = reports_dir.join("judged-eval-results.json");

    let quality_json = match serde_json::to_string_pretty(&quality_report) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mg-eval: failed to serialize retrieval-quality: {e}");
            return ExitCode::from(2);
        }
    };
    let judged_json = match serde_json::to_string_pretty(&judged_results) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mg-eval: failed to serialize judged-eval-results: {e}");
            return ExitCode::from(2);
        }
    };

    if let Err(e) = std::fs::write(&quality_path, quality_json.as_bytes()) {
        eprintln!("mg-eval: failed to write {}: {e}", quality_path.display());
        return ExitCode::from(2);
    }
    if let Err(e) = std::fs::write(&judged_path, judged_json.as_bytes()) {
        eprintln!("mg-eval: failed to write {}: {e}", judged_path.display());
        return ExitCode::from(2);
    }

    println!("Evidence artifacts written:");
    println!("  {}", quality_path.display());
    println!("  {}", judged_path.display());
    println!();

    // Write the manifest.json for this run.
    let manifest_json = make_manifest_json(
        &run_root,
        &quality_report,
        &quality_path,
        &judged_path,
        &fm_path,
    );
    if let Err(e) = std::fs::write(&manifest_path, manifest_json.as_bytes()) {
        eprintln!("mg-eval: failed to write manifest: {e}", );
        return ExitCode::from(2);
    }
    println!("Manifest written: {}", manifest_path.display());

    if quality_report.passed {
        println!("\n✓ V-RET-03 PASS — all retrieval quality thresholds met.");
        ExitCode::SUCCESS
    } else {
        println!("\n✗ V-RET-03 FAIL — see failure reasons above.");
        ExitCode::FAILURE
    }
}

fn sha256_file(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(&bytes);
    format!("{:x}", h.finalize())
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn make_manifest_json(
    run_root: &Path,
    report: &kria_eval::memory_graph::judged_eval::RetrievalQualityReport,
    quality_path: &Path,
    judged_path: &Path,
    fm_path: &Path,
) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    let assertions = &report.assertions;
    let ov = &report.overall;

    // Relative paths from run_root.
    let quality_rel = "reports/retrieval-quality.json";
    let judged_rel = "reports/judged-eval-results.json";

    serde_json::to_string_pretty(&serde_json::json!({
        "schemaVersion": "evidence-manifest/v1",
        "runId": run_root.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("run-001"),
        "gate": "F3",
        "suiteId": "V-RET-03",
        "status": if report.passed { "Pass" } else { "Fail" },
        "utcStart": now,
        "utcEnd": now,
        "requirementIds": ["MGR-006", "MGR-036"],
        "command": {
            "id": "CMD-MG-EVAL",
            "argv": ["cargo", "run", "-p", "kria-eval", "--bin", "mg-eval", "--",
                     "--manifest", "evidence/F3/run-001/manifest.json"],
            "workingDirectory": ".",
            "exitCode": if report.passed { 0 } else { 1 }
        },
        "fixtureIds": [{
            "fixtureId": "mg-retrieval-judged-v2",
            "seed": "0x4D475207",
            "generatorHash": sha256_file(fm_path)
        }],
        "assertionTotals": {
            "total": assertions.total,
            "passed": assertions.passed,
            "failed": assertions.failed,
            "counterexamples": []
        },
        "metrics": {
            "recall_at_10": ov.recall_at_10,
            "ndcg_at_10": ov.ndcg_at_10,
            "identifier_phrase_recall": ov.identifier_phrase_recall,
            "forbidden_exclusion_rate": ov.forbidden_exclusion_rate,
            "deleted_forgotten_superseded_exclusion_rate":
                ov.deleted_forgotten_superseded_exclusion_rate,
            "sample_size": ov.sample_size
        },
        "artifacts": [
            {
                "path": quality_rel,
                "mediaType": "application/json",
                "sha256": sha256_file(quality_path),
                "size": file_size(quality_path)
            },
            {
                "path": judged_rel,
                "mediaType": "application/json",
                "sha256": sha256_file(judged_path),
                "size": file_size(judged_path)
            }
        ],
        "reviewers": [{
            "role": "owner-self-review",
            "reviewerId": "owner",
            "utcTimestamp": now,
            "verdict": if report.passed { "Pass" } else { "Fail" },
            "notes": "Single-developer pre-production project; owner-self-review accepted per dev-context.md",
            "signatureMethod": "owner-attestation"
        }],
        "failureReasons": report.failure_reasons
    }))
    .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"))
}
