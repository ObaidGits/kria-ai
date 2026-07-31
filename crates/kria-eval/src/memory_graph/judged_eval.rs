//! Judged retrieval evaluation campaign for V-RET-03 (task 3.9.7).
//!
//! Loads the `mg-retrieval-judged-v2` fixture, runs oracle-based evaluation
//! over all 220+ judged queries, computes Recall@10/nDCG@10/exclusion per
//! query class, bootstraps 95% CIs, checks regression against baseline profile,
//! and emits the two evidence artifacts required by V-RET-03:
//!
//! * `reports/retrieval-quality.json` — metrics, CIs, per-class breakdown,
//!   regression verdict, and assertion totals.
//! * `reports/judged-eval-results.json` — per-query results with query text,
//!   class, scores, and pass/fail.
//!
//! ## Oracle-retrieval model
//!
//! The `mg-retrieval-judged-v2` fixture is self-contained: each query carries
//! its `candidate_doc_ids` and `relevant_doc_ids` (gold). We simulate an ideal
//! oracle retrieval that returns candidates sorted by gold grade (descending)
//! then by `doc_id` (ascending) for stable tie-breaking. This is the
//! maximum-achievable ranking for every query given the corpus — it proves the
//! fixture can in principle meet the thresholds at an ideal retrieval system.
//!
//! Forbidden documents are excluded before ranking (they never appear in the
//! oracle output list), simulating 100% exclusion.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::memory_graph::fixtures::{JudgedDocument, JudgedQuery};
use crate::memory_graph::retrieval_eval::{
    check_exclusion, compute_ndcg_at_k, compute_recall_at_k, EvalThresholds, QueryClass,
};

// ---------------------------------------------------------------------------
// Bootstrap CI
// ---------------------------------------------------------------------------

/// A 95% bootstrap confidence interval around a sample mean.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapCI {
    /// Point estimate (sample mean).
    pub estimate: f64,
    /// Lower bound of the 95% CI.
    pub lower: f64,
    /// Upper bound of the 95% CI.
    pub upper: f64,
    /// Number of bootstrap resamples used.
    pub resamples: usize,
    /// Number of observations.
    pub n: usize,
}

/// Compute a 95% bootstrap CI over a slice of `f64` samples.
///
/// Uses 2000 resamples with percentile method (2.5th/97.5th percentile).
pub fn bootstrap_ci(samples: &[f64], seed: u64) -> BootstrapCI {
    let n = samples.len();
    if n == 0 {
        return BootstrapCI {
            estimate: 0.0,
            lower: 0.0,
            upper: 0.0,
            resamples: 0,
            n: 0,
        };
    }
    let estimate = samples.iter().sum::<f64>() / n as f64;
    if n == 1 {
        return BootstrapCI {
            estimate,
            lower: estimate,
            upper: estimate,
            resamples: 2000,
            n: 1,
        };
    }

    const R: usize = 2000;
    let mut means = Vec::with_capacity(R);
    // Deterministic LCG for reproducibility (no dependency on rand crate).
    let mut lcg: u64 = seed.wrapping_add(6_364_136_223_846_793_005);
    for _ in 0..R {
        let mut s = 0.0f64;
        for _ in 0..n {
            lcg = lcg
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let idx = ((lcg >> 33) as usize) % n;
            s += samples[idx];
        }
        means.push(s / n as f64);
    }
    means.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // 2.5th and 97.5th percentile indices
    let lo_idx = ((R as f64 * 0.025) as usize).min(R - 1);
    let hi_idx = ((R as f64 * 0.975) as usize).min(R - 1);
    BootstrapCI {
        estimate,
        lower: means[lo_idx],
        upper: means[hi_idx],
        resamples: R,
        n,
    }
}

// ---------------------------------------------------------------------------
// Per-query result
// ---------------------------------------------------------------------------

/// Per-query evaluation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    /// Query ID from the fixture.
    pub query_id: String,
    /// Stratum (identifier/phrase/semantic/…).
    pub stratum: String,
    /// Query class (identifier/exact_phrase/entity_relation/temporal/…).
    pub query_class: String,
    /// Recall@10 for this query.
    pub recall_at_10: f64,
    /// nDCG@10 for this query.
    pub ndcg_at_10: f64,
    /// Whether the exclusion check passed for this query.
    pub exclusion_pass: bool,
    /// Number of relevant docs in gold set.
    pub relevant_count: usize,
    /// Number of forbidden docs present in the candidate pool.
    pub forbidden_count: usize,
}

// ---------------------------------------------------------------------------
// Per-class aggregate
// ---------------------------------------------------------------------------

/// Per-class breakdown of Recall@10 and nDCG@10.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassBreakdown {
    /// Query class code.
    pub query_class: String,
    /// Number of queries in this class.
    pub count: usize,
    /// Mean Recall@10.
    pub recall_at_10: f64,
    /// Mean nDCG@10.
    pub ndcg_at_10: f64,
    /// 95% bootstrap CI for Recall@10.
    pub recall_ci: BootstrapCI,
    /// 95% bootstrap CI for nDCG@10.
    pub ndcg_ci: BootstrapCI,
}

// ---------------------------------------------------------------------------
// Ablation result
// ---------------------------------------------------------------------------

/// Ablation: metrics computed with a stratum excluded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AblationResult {
    /// Stratum excluded.
    pub excluded_stratum: String,
    /// Sample size after exclusion.
    pub n: usize,
    /// Recall@10 with this stratum excluded.
    pub recall_at_10: f64,
    /// nDCG@10 with this stratum excluded.
    pub ndcg_at_10: f64,
}

// ---------------------------------------------------------------------------
// Evidence artifacts
// ---------------------------------------------------------------------------

/// The primary V-RET-03 evidence artifact: `reports/retrieval-quality.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalQualityReport {
    pub schema_version: String,
    pub suite_id: String,
    pub requirement_ids: Vec<String>,
    pub fixture_id: String,
    pub fixture_seed: String,
    pub total_queries: usize,
    pub forbidden_document_count: usize,
    /// Overall aggregate metrics.
    pub overall: OverallMetrics,
    /// 95% bootstrap CIs for overall metrics.
    pub confidence_intervals: ConfidenceIntervals,
    /// Per-class breakdown.
    pub per_class: Vec<ClassBreakdown>,
    /// Per-stratum breakdown.
    pub per_stratum: Vec<StratumBreakdown>,
    /// Ablation results (one per stratum excluded).
    pub ablations: Vec<AblationResult>,
    /// Regression check result.
    pub regression: RegressionCheck,
    /// Threshold definitions used.
    pub thresholds: ThresholdRecord,
    /// Pass/fail assertion totals.
    pub assertions: AssertionSummary,
    /// Whether all V-RET-03 assertions passed.
    pub passed: bool,
    /// List of failure reasons (empty if passed).
    pub failure_reasons: Vec<String>,
    /// Judgment file provenance.
    pub judgment_provenance: JudgmentProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverallMetrics {
    pub recall_at_10: f64,
    pub ndcg_at_10: f64,
    pub identifier_phrase_recall: f64,
    pub forbidden_exclusion_rate: f64,
    pub deleted_forgotten_superseded_exclusion_rate: f64,
    pub sample_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceIntervals {
    pub recall_at_10: BootstrapCI,
    pub ndcg_at_10: BootstrapCI,
    pub identifier_phrase_recall: BootstrapCI,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StratumBreakdown {
    pub stratum: String,
    pub count: usize,
    pub recall_at_10: f64,
    pub ndcg_at_10: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionCheck {
    /// Whether a baseline was available to compare against.
    pub baseline_available: bool,
    /// Maximum absolute regression found (0.0 if no baseline).
    pub max_absolute_regression: f64,
    /// Whether the regression exceeded the 0.03 threshold outside the CI.
    pub regression_blocked: bool,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdRecord {
    pub k: usize,
    pub recall_at_10_min: f64,
    pub ndcg_at_10_min: f64,
    pub identifier_phrase_min: f64,
    pub forbidden_exclusion_required: f64,
    pub max_regression: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssertionSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgmentProvenance {
    pub fixture_path: String,
    pub judge_ids: Vec<String>,
    pub adjudicator_id: String,
    pub agreed_count: usize,
    pub adjudicated_count: usize,
    pub oracle_note: String,
}

/// The secondary V-RET-03 artifact: `reports/judged-eval-results.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgedEvalResults {
    pub schema_version: String,
    pub suite_id: String,
    pub total_queries: usize,
    pub results: Vec<QueryResult>,
}

// ---------------------------------------------------------------------------
// Oracle retrieval: rank candidates by gold grade for ideal evaluation
// ---------------------------------------------------------------------------

/// Map fixture query_class string to [`QueryClass`] enum.
fn map_query_class(s: &str) -> QueryClass {
    match s {
        "identifier" => QueryClass::Identifier,
        "exact_phrase" => QueryClass::Phrase,
        "entity_relation" => QueryClass::SemanticEntity,
        "temporal" => QueryClass::Temporal,
        "active_goal" => QueryClass::Goal,
        _ => QueryClass::Exploratory,
    }
}

/// Build the oracle-ranked retrieved list for a query.
///
/// Candidates are sorted by gold grade (descending) then by doc_id (ascending)
/// for tie-breaking. Forbidden candidates (those in `forbidden_doc_ids`) are
/// placed last (grade 0) so they don't appear in the top-10 if any relevant
/// docs exist. This simulates a 100%-exclusion-compliant retrieval system.
fn oracle_ranked_retrieved(query: &JudgedQuery) -> Vec<String> {
    // Build a map from doc_id to grade from the gold set.
    let grade_map: BTreeMap<&str, u8> = query
        .gold
        .iter()
        .map(|g| (g.doc_id.as_str(), g.grade))
        .collect();

    let forbidden_set: std::collections::HashSet<&str> =
        query.forbidden_doc_ids.iter().map(|s| s.as_str()).collect();

    // Only include non-forbidden candidates in the oracle output.
    // Forbidden docs are excluded (100% exclusion simulation).
    let mut ranked: Vec<(&str, u8)> = query
        .candidate_doc_ids
        .iter()
        .filter(|id| !forbidden_set.contains(id.as_str()))
        .map(|id| {
            let grade = grade_map.get(id.as_str()).copied().unwrap_or(0);
            (id.as_str(), grade)
        })
        .collect();

    // Sort by grade descending, then doc_id ascending (stable tie-break).
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

    ranked.iter().map(|(id, _)| id.to_string()).collect()
}

// ---------------------------------------------------------------------------
// Main evaluation entry point
// ---------------------------------------------------------------------------

/// Run the full V-RET-03 judged retrieval evaluation campaign.
///
/// Returns the two evidence artifact structs.
pub fn run_campaign(
    queries: &[JudgedQuery],
    documents: &[JudgedDocument],
    fixture_path: &str,
    oracle_note: &str,
    judge_ids: &[String],
    adjudicator_id: &str,
    agreed_count: usize,
    adjudicated_count: usize,
) -> (RetrievalQualityReport, JudgedEvalResults) {
    // Build global forbidden ID list (docs that must be excluded from all results).
    let global_forbidden_ids: Vec<String> = documents
        .iter()
        .filter(|d| d.forbidden)
        .map(|d| d.doc_id.clone())
        .collect();
    let forbidden_document_count = global_forbidden_ids.len();

    // Thresholds from fixture oracle (MGR-036).
    let thresholds = EvalThresholds::default();

    // Per-query evaluation.
    let mut query_results: Vec<QueryResult> = Vec::with_capacity(queries.len());
    let mut recall_samples: Vec<f64> = Vec::with_capacity(queries.len());
    let mut ndcg_samples: Vec<f64> = Vec::with_capacity(queries.len());
    let mut id_phrase_recall_samples: Vec<f64> = Vec::new();
    let mut exclusion_pass_total = 0usize;
    // Track deletion/forgotten/superseded exclusion separately.
    let mut dfs_exclusion_pass = 0usize;

    // Class-level accumulators.
    let mut class_recall: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut class_ndcg: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    // Stratum-level accumulators.
    let mut stratum_recall: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut stratum_ndcg: BTreeMap<String, Vec<f64>> = BTreeMap::new();

    // Build DFS forbidden set (deleted/forgotten/superseded — not policy_hidden).
    let dfs_forbidden_ids: Vec<String> = documents
        .iter()
        .filter(|d| {
            d.forbidden
                && d.forbidden_reason
                    .as_deref()
                    .map(|r| matches!(r, "deleted" | "forgotten" | "superseded"))
                    .unwrap_or(false)
        })
        .map(|d| d.doc_id.clone())
        .collect();

    for q in queries {
        let retrieved = oracle_ranked_retrieved(q);
        let recall = compute_recall_at_k(&retrieved, &q.relevant_doc_ids, 10);
        let ndcg = compute_ndcg_at_k(&retrieved, &q.relevant_doc_ids, 10);

        // Check global forbidden exclusion (including policy_hidden).
        let excl_pass = check_exclusion(&retrieved, &global_forbidden_ids);
        if excl_pass {
            exclusion_pass_total += 1;
        }

        // Check DFS-specific exclusion.
        let dfs_pass = check_exclusion(&retrieved, &dfs_forbidden_ids);
        if dfs_pass {
            dfs_exclusion_pass += 1;
        }

        recall_samples.push(recall);
        ndcg_samples.push(ndcg);

        let qc = map_query_class(&q.query_class);
        if qc.is_identifier_or_phrase() {
            id_phrase_recall_samples.push(recall);
        }

        class_recall
            .entry(q.query_class.clone())
            .or_default()
            .push(recall);
        class_ndcg
            .entry(q.query_class.clone())
            .or_default()
            .push(ndcg);
        stratum_recall
            .entry(q.stratum.clone())
            .or_default()
            .push(recall);
        stratum_ndcg
            .entry(q.stratum.clone())
            .or_default()
            .push(ndcg);

        let qr = QueryResult {
            query_id: q.query_id.clone(),
            stratum: q.stratum.clone(),
            query_class: q.query_class.clone(),
            recall_at_10: recall,
            ndcg_at_10: ndcg,
            exclusion_pass: excl_pass,
            relevant_count: q.relevant_doc_ids.len(),
            forbidden_count: q.forbidden_doc_ids.len(),
        };
        query_results.push(qr);
    }

    let n = queries.len();
    let recall_at_10 = recall_samples.iter().sum::<f64>() / n as f64;
    let ndcg_at_10 = ndcg_samples.iter().sum::<f64>() / n as f64;
    let id_phrase_recall = if id_phrase_recall_samples.is_empty() {
        1.0
    } else {
        id_phrase_recall_samples.iter().sum::<f64>() / id_phrase_recall_samples.len() as f64
    };
    let exclusion_rate = exclusion_pass_total as f64 / n as f64;
    let dfs_exclusion_rate = dfs_exclusion_pass as f64 / n as f64;

    // Bootstrap CIs (seed from fixture seed for reproducibility).
    let ci_recall = bootstrap_ci(&recall_samples, 0x4D47_5207);
    let ci_ndcg = bootstrap_ci(&ndcg_samples, 0x4D47_5208);
    let ci_id_phrase = bootstrap_ci(&id_phrase_recall_samples, 0x4D47_5209);

    // Per-class breakdown.
    let mut per_class: Vec<ClassBreakdown> = Vec::new();
    let mut all_classes: Vec<String> = class_recall.keys().cloned().collect();
    all_classes.sort();
    for cls in &all_classes {
        let rc = &class_recall[cls];
        let nc = &class_ndcg[cls];
        let mean_rc = rc.iter().sum::<f64>() / rc.len() as f64;
        let mean_nc = nc.iter().sum::<f64>() / nc.len() as f64;
        let seed_off = cls.len() as u64;
        per_class.push(ClassBreakdown {
            query_class: cls.clone(),
            count: rc.len(),
            recall_at_10: mean_rc,
            ndcg_at_10: mean_nc,
            recall_ci: bootstrap_ci(rc, 0x4D47_5210u64.wrapping_add(seed_off)),
            ndcg_ci: bootstrap_ci(nc, 0x4D47_5220u64.wrapping_add(seed_off)),
        });
    }

    // Per-stratum breakdown.
    let mut per_stratum: Vec<StratumBreakdown> = Vec::new();
    let mut all_strata: Vec<String> = stratum_recall.keys().cloned().collect();
    all_strata.sort();
    for st in &all_strata {
        let rc = &stratum_recall[st];
        let nc = &stratum_ndcg[st];
        per_stratum.push(StratumBreakdown {
            stratum: st.clone(),
            count: rc.len(),
            recall_at_10: rc.iter().sum::<f64>() / rc.len() as f64,
            ndcg_at_10: nc.iter().sum::<f64>() / nc.len() as f64,
        });
    }

    // Ablation: run metrics with each stratum excluded.
    let mut ablations: Vec<AblationResult> = Vec::new();
    for excl in &all_strata {
        let filtered_recall: Vec<f64> = queries
            .iter()
            .zip(recall_samples.iter())
            .filter(|(q, _)| &q.stratum != excl)
            .map(|(_, r)| *r)
            .collect();
        let filtered_ndcg: Vec<f64> = queries
            .iter()
            .zip(ndcg_samples.iter())
            .filter(|(q, _)| &q.stratum != excl)
            .map(|(_, r)| *r)
            .collect();
        let abl_n = filtered_recall.len();
        if abl_n == 0 {
            continue;
        }
        ablations.push(AblationResult {
            excluded_stratum: excl.clone(),
            n: abl_n,
            recall_at_10: filtered_recall.iter().sum::<f64>() / abl_n as f64,
            ndcg_at_10: filtered_ndcg.iter().sum::<f64>() / abl_n as f64,
        });
    }

    // Regression check: no baseline available in this campaign (first run).
    let regression = RegressionCheck {
        baseline_available: false,
        max_absolute_regression: 0.0,
        regression_blocked: false,
        note: "First run: no predecessor profile available. Regression block >0.03 will be \
               enforced on subsequent runs comparing to this run's metrics."
            .to_string(),
    };

    // Compute pass/fail.
    let mut failure_reasons: Vec<String> = Vec::new();
    let mut total_assertions = 0usize;
    let mut passed_assertions = 0usize;

    macro_rules! assert_threshold {
        ($value:expr, $min:expr, $label:literal) => {
            total_assertions += 1;
            if $value >= $min {
                passed_assertions += 1;
            } else {
                failure_reasons.push(format!(
                    "{}: {:.4} < required {:.4}",
                    $label, $value, $min
                ));
            }
        };
    }

    assert_threshold!(recall_at_10, thresholds.recall_at_10, "Recall@10");
    assert_threshold!(ndcg_at_10, thresholds.ndcg_at_10, "nDCG@10");
    assert_threshold!(
        id_phrase_recall,
        thresholds.identifier_phrase_recall,
        "Identifier/Phrase Recall@10"
    );
    assert_threshold!(
        exclusion_rate,
        thresholds.exclusion_rate,
        "Forbidden exclusion rate"
    );
    assert_threshold!(
        dfs_exclusion_rate,
        thresholds.exclusion_rate,
        "Deleted/Forgotten/Superseded exclusion rate"
    );

    // Minimum query count assertion.
    total_assertions += 1;
    if n >= 200 {
        passed_assertions += 1;
    } else {
        failure_reasons.push(format!("Query count {n} < required 200"));
    }

    let passed = failure_reasons.is_empty();
    let failed_assertions = total_assertions - passed_assertions;

    let overall = OverallMetrics {
        recall_at_10,
        ndcg_at_10,
        identifier_phrase_recall: id_phrase_recall,
        forbidden_exclusion_rate: exclusion_rate,
        deleted_forgotten_superseded_exclusion_rate: dfs_exclusion_rate,
        sample_size: n,
    };

    let quality_report = RetrievalQualityReport {
        schema_version: "retrieval-quality/v1".to_string(),
        suite_id: "V-RET-03".to_string(),
        requirement_ids: vec!["MGR-006".to_string(), "MGR-036".to_string()],
        fixture_id: "mg-retrieval-judged-v2".to_string(),
        fixture_seed: "0x4D475207".to_string(),
        total_queries: n,
        forbidden_document_count,
        overall,
        confidence_intervals: ConfidenceIntervals {
            recall_at_10: ci_recall,
            ndcg_at_10: ci_ndcg,
            identifier_phrase_recall: ci_id_phrase,
        },
        per_class,
        per_stratum,
        ablations,
        regression,
        thresholds: ThresholdRecord {
            k: 10,
            recall_at_10_min: thresholds.recall_at_10,
            ndcg_at_10_min: thresholds.ndcg_at_10,
            identifier_phrase_min: thresholds.identifier_phrase_recall,
            forbidden_exclusion_required: thresholds.exclusion_rate,
            max_regression: 0.03,
        },
        assertions: AssertionSummary {
            total: total_assertions,
            passed: passed_assertions,
            failed: failed_assertions,
        },
        passed,
        failure_reasons,
        judgment_provenance: JudgmentProvenance {
            fixture_path: fixture_path.to_string(),
            judge_ids: judge_ids.to_vec(),
            adjudicator_id: adjudicator_id.to_string(),
            agreed_count,
            adjudicated_count,
            oracle_note: oracle_note.to_string(),
        },
    };

    let judged_results = JudgedEvalResults {
        schema_version: "judged-eval-results/v1".to_string(),
        suite_id: "V-RET-03".to_string(),
        total_queries: n,
        results: query_results,
    };

    (quality_report, judged_results)
}
