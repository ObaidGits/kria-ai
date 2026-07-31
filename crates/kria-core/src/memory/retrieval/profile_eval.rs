//! Offline profile comparison and activation gate (design §6.3, task F3.4.5).
//!
//! Computes Recall@k, nDCG@k, per-class metrics, 95% bootstrap CIs, forbidden/
//! deletion invariants, and enforces ≤0.03 regression gate before any candidate
//! profile can be activated.  Runtime feedback MUST NOT modify weights per user
//! request.

use crate::memory::retrieval::classifier::classify_query_v2;

// ── Types ─────────────────────────────────────────────────────────────────────

/// One judged query result for evaluation.
#[derive(Debug, Clone)]
pub struct JudgedQuery {
    /// Query text (for class detection).
    pub query: String,
    /// Ordered list of returned record IDs (by rank, best first).
    pub returned_ids: Vec<String>,
    /// Ground-truth relevant record IDs for this query.
    pub relevant_ids: Vec<String>,
    /// Record IDs that must NEVER appear in results (forbidden: deleted/forgotten/superseded).
    pub forbidden_ids: Vec<String>,
}

/// Per-class metric aggregates.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassMetrics {
    pub query_class: String,
    pub query_count: usize,
    pub recall_at_k: f64,
    pub ndcg_at_k: f64,
}

/// Bootstrap confidence interval (95% two-sided).
#[derive(Debug, Clone, PartialEq)]
pub struct BootstrapCI {
    /// "recall@10" | "ndcg@10" | etc.
    pub metric: String,
    pub mean: f64,
    /// 2.5th percentile
    pub lower: f64,
    /// 97.5th percentile
    pub upper: f64,
    /// number of bootstrap resamples
    pub sample_count: usize,
}

/// Result of evaluating a profile against judged queries.
#[derive(Debug, Clone)]
pub struct ProfileEvalResult {
    pub profile_id: String,
    pub query_count: usize,
    /// Overall Recall@10.
    pub recall_at_k: f64,
    /// Overall nDCG@10.
    pub ndcg_at_k: f64,
    /// Recall@10 for identifier + exact_phrase classes combined.
    pub identifier_phrase_recall: f64,
    /// Whether forbidden/deleted items appear in any result (must be 0.0).
    pub forbidden_leak_rate: f64,
    /// Per-class breakdown.
    pub class_metrics: Vec<ClassMetrics>,
    /// Bootstrap CIs for overall recall and nDCG.
    pub bootstrap_cis: Vec<BootstrapCI>,
}

/// Decision on whether to activate a candidate profile.
#[derive(Debug, Clone, PartialEq)]
pub enum ActivationDecision {
    /// All thresholds met; profile may be activated.
    Approved,
    /// One or more threshold failures — includes list of failing checks.
    Rejected(Vec<String>),
}

/// Parameters for profile activation gate.
#[derive(Debug, Clone)]
pub struct ActivationGate {
    /// Minimum Recall@10 (design: ≥0.85).
    pub min_recall: f64,
    /// Minimum nDCG@10 (design: ≥0.80).
    pub min_ndcg: f64,
    /// Minimum identifier/phrase Recall@10 (design: ≥0.95).
    pub min_identifier_phrase_recall: f64,
    /// Maximum allowed forbidden leak rate (design: 0.0 = 100% exclusion).
    pub max_forbidden_leak_rate: f64,
    /// Maximum accepted metric regression vs. baseline (design: ≤0.03 absolute).
    pub max_regression: f64,
    /// Minimum query count required (design: ≥200).
    pub min_query_count: usize,
    /// Number of bootstrap resamples (100 for fast offline eval).
    pub bootstrap_resamples: usize,
}

impl Default for ActivationGate {
    fn default() -> Self {
        Self {
            min_recall: 0.85,
            min_ndcg: 0.80,
            min_identifier_phrase_recall: 0.95,
            max_forbidden_leak_rate: 0.0,
            max_regression: 0.03,
            min_query_count: 200,
            bootstrap_resamples: 100,
        }
    }
}

// ── Simple LCG PRNG (deterministic, no external dependency) ──────────────────

struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_usize(&mut self, n: usize) -> usize {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.state >> 33) as usize) % n
    }
}

// ── Core metric functions ─────────────────────────────────────────────────────

/// Compute Recall@k for a single query: |retrieved ∩ relevant| / |relevant|.
/// Returns 1.0 if `relevant_ids` is empty (undefined; treat as perfect recall).
pub fn recall_at_k(returned: &[String], relevant: &[String], k: usize) -> f64 {
    if relevant.is_empty() {
        return 1.0;
    }
    let top_k = &returned[..k.min(returned.len())];
    let hits = top_k.iter().filter(|id| relevant.contains(id)).count();
    hits as f64 / relevant.len() as f64
}

/// Compute nDCG@k for a single query using binary relevance.
///
/// DCG@k = Σ_{i=1}^{k} rel_i / log2(i + 1)  where rel_i ∈ {0, 1}
/// IDCG@k = Σ_{i=1}^{min(|relevant|, k)} 1.0 / log2(i + 1)
/// nDCG = DCG / IDCG  (= 1.0 when relevant is empty)
pub fn ndcg_at_k(returned: &[String], relevant: &[String], k: usize) -> f64 {
    if relevant.is_empty() {
        return 1.0;
    }
    let top_k = &returned[..k.min(returned.len())];
    let dcg: f64 = top_k
        .iter()
        .enumerate()
        .map(|(i, id)| {
            if relevant.contains(id) {
                1.0 / (i as f64 + 2.0).log2() // i+2 because i is 0-indexed, formula uses i+1 with log2
            } else {
                0.0
            }
        })
        .sum();

    let ideal_len = relevant.len().min(k);
    let idcg: f64 = (1..=ideal_len).map(|i| 1.0 / (i as f64 + 1.0).log2()).sum();

    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}

// ── Bootstrap CI helper ───────────────────────────────────────────────────────

fn bootstrap_ci(scores: &[f64], n_resamples: usize, metric: &str) -> BootstrapCI {
    let n = scores.len();
    let mean = if n == 0 {
        0.0
    } else {
        scores.iter().sum::<f64>() / n as f64
    };

    if n == 0 {
        return BootstrapCI {
            metric: metric.to_string(),
            mean: 0.0,
            lower: 0.0,
            upper: 0.0,
            sample_count: n_resamples,
        };
    }

    let mut rng = Lcg::new(42);
    let mut resample_means: Vec<f64> = (0..n_resamples)
        .map(|_| {
            let sum: f64 = (0..n).map(|_| scores[rng.next_usize(n)]).sum();
            sum / n as f64
        })
        .collect();

    resample_means.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let lower_idx = (0.025 * n_resamples as f64).floor() as usize;
    let upper_idx = (0.975 * n_resamples as f64).floor() as usize;

    let lower = resample_means[lower_idx.min(n_resamples - 1)];
    let upper = resample_means[upper_idx.min(n_resamples - 1)];

    BootstrapCI {
        metric: metric.to_string(),
        mean,
        lower,
        upper,
        sample_count: n_resamples,
    }
}

// ── evaluate_profile ──────────────────────────────────────────────────────────

const K: usize = 10;

/// Evaluate a profile against a corpus of judged queries.
///
/// Uses k=10 for all metric computation. Uses the classifier to assign query classes.
pub fn evaluate_profile(
    queries: &[JudgedQuery],
    profile_id: &str,
    gate: &ActivationGate,
) -> ProfileEvalResult {
    use std::collections::HashMap;

    let mut recall_scores: Vec<f64> = Vec::with_capacity(queries.len());
    let mut ndcg_scores: Vec<f64> = Vec::with_capacity(queries.len());
    let mut forbidden_leak_count: usize = 0;

    // class -> (recall_sum, ndcg_sum, count)
    let mut class_map: HashMap<String, (f64, f64, usize)> = HashMap::new();
    // identifier + exact_phrase combined
    let mut ip_recall_sum = 0.0_f64;
    let mut ip_count: usize = 0;

    for q in queries {
        let r = recall_at_k(&q.returned_ids, &q.relevant_ids, K);
        let n = ndcg_at_k(&q.returned_ids, &q.relevant_ids, K);
        recall_scores.push(r);
        ndcg_scores.push(n);

        // Forbidden check: any forbidden id in the full returned list
        let has_leak = q.returned_ids.iter().any(|id| q.forbidden_ids.contains(id));
        if has_leak {
            forbidden_leak_count += 1;
        }

        // Classify query for per-class metrics
        let class = classify_query_v2(&q.query).class;
        let class_str = class.as_str().to_string();
        let entry = class_map.entry(class_str.clone()).or_insert((0.0, 0.0, 0));
        entry.0 += r;
        entry.1 += n;
        entry.2 += 1;

        // Identifier + ExactPhrase combined recall
        if class_str == "identifier" || class_str == "exact_phrase" {
            ip_recall_sum += r;
            ip_count += 1;
        }
    }

    let query_count = queries.len();

    let overall_recall = if query_count == 0 {
        0.0
    } else {
        recall_scores.iter().sum::<f64>() / query_count as f64
    };
    let overall_ndcg = if query_count == 0 {
        0.0
    } else {
        ndcg_scores.iter().sum::<f64>() / query_count as f64
    };

    let identifier_phrase_recall = if ip_count == 0 {
        1.0 // no queries of these classes — treat as perfect (vacuously true)
    } else {
        ip_recall_sum / ip_count as f64
    };

    let forbidden_leak_rate = if query_count == 0 {
        0.0
    } else {
        forbidden_leak_count as f64 / query_count as f64
    };

    let class_metrics: Vec<ClassMetrics> = class_map
        .into_iter()
        .map(|(cls, (r_sum, n_sum, cnt))| ClassMetrics {
            query_class: cls,
            query_count: cnt,
            recall_at_k: r_sum / cnt as f64,
            ndcg_at_k: n_sum / cnt as f64,
        })
        .collect();

    let bootstrap_cis = vec![
        bootstrap_ci(&recall_scores, gate.bootstrap_resamples, "recall@10"),
        bootstrap_ci(&ndcg_scores, gate.bootstrap_resamples, "ndcg@10"),
    ];

    ProfileEvalResult {
        profile_id: profile_id.to_string(),
        query_count,
        recall_at_k: overall_recall,
        ndcg_at_k: overall_ndcg,
        identifier_phrase_recall,
        forbidden_leak_rate,
        class_metrics,
        bootstrap_cis,
    }
}

// ── check_activation ──────────────────────────────────────────────────────────

/// Compare a candidate profile evaluation against a baseline evaluation.
///
/// Returns `ActivationDecision::Approved` if ALL of:
/// 1. recall >= gate.min_recall
/// 2. ndcg >= gate.min_ndcg
/// 3. identifier_phrase_recall >= gate.min_identifier_phrase_recall
/// 4. forbidden_leak_rate <= gate.max_forbidden_leak_rate (must be 0.0)
/// 5. recall regression vs. baseline <= gate.max_regression (if baseline provided)
/// 6. ndcg regression vs. baseline <= gate.max_regression (if baseline provided)
/// 7. query_count >= gate.min_query_count
///
/// `baseline` is `None` for first-ever activation (no regression check).
pub fn check_activation(
    candidate: &ProfileEvalResult,
    baseline: Option<&ProfileEvalResult>,
    gate: &ActivationGate,
) -> ActivationDecision {
    let mut failures: Vec<String> = Vec::new();

    // 1. Minimum query count
    if candidate.query_count < gate.min_query_count {
        failures.push(format!(
            "insufficient query count: {} < {} required",
            candidate.query_count, gate.min_query_count
        ));
    }

    // 2. Recall@10 threshold
    if candidate.recall_at_k < gate.min_recall {
        failures.push(format!(
            "recall@10 {:.4} < minimum {:.4}",
            candidate.recall_at_k, gate.min_recall
        ));
    }

    // 3. nDCG@10 threshold
    if candidate.ndcg_at_k < gate.min_ndcg {
        failures.push(format!(
            "ndcg@10 {:.4} < minimum {:.4}",
            candidate.ndcg_at_k, gate.min_ndcg
        ));
    }

    // 4. Identifier + phrase recall threshold
    if candidate.identifier_phrase_recall < gate.min_identifier_phrase_recall {
        failures.push(format!(
            "identifier/phrase recall@10 {:.4} < minimum {:.4}",
            candidate.identifier_phrase_recall, gate.min_identifier_phrase_recall
        ));
    }

    // 5. Forbidden leak rate (must be 0.0)
    if candidate.forbidden_leak_rate > gate.max_forbidden_leak_rate {
        failures.push(format!(
            "forbidden leak rate {:.4} > maximum {:.4} (100% exclusion required)",
            candidate.forbidden_leak_rate, gate.max_forbidden_leak_rate
        ));
    }

    // 6. Regression checks against baseline (skipped if no baseline)
    if let Some(base) = baseline {
        let recall_regression = base.recall_at_k - candidate.recall_at_k;
        if recall_regression > gate.max_regression {
            failures.push(format!(
                "recall@10 regression {:.4} > maximum allowed {:.4} (baseline={:.4}, candidate={:.4})",
                recall_regression, gate.max_regression, base.recall_at_k, candidate.recall_at_k
            ));
        }

        let ndcg_regression = base.ndcg_at_k - candidate.ndcg_at_k;
        if ndcg_regression > gate.max_regression {
            failures.push(format!(
                "ndcg@10 regression {:.4} > maximum allowed {:.4} (baseline={:.4}, candidate={:.4})",
                ndcg_regression, gate.max_regression, base.ndcg_at_k, candidate.ndcg_at_k
            ));
        }
    }

    if failures.is_empty() {
        ActivationDecision::Approved
    } else {
        ActivationDecision::Rejected(failures)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // ── recall_at_k ──────────────────────────────────────────────────────────

    #[test]
    fn recall_at_k_perfect_recall() {
        let returned = ids(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]);
        let relevant = ids(&["a", "b", "c"]);
        assert_eq!(recall_at_k(&returned, &relevant, 10), 1.0);
    }

    #[test]
    fn recall_at_k_zero_recall() {
        let returned = ids(&["x", "y", "z", "w", "v", "u", "t", "s", "r", "q"]);
        let relevant = ids(&["a", "b", "c"]);
        assert_eq!(recall_at_k(&returned, &relevant, 10), 0.0);
    }

    #[test]
    fn recall_at_k_partial_recall() {
        // 2 out of 4 relevant in top 10 → 0.5
        let returned = ids(&["a", "x", "b", "y", "z1", "z2", "z3", "z4", "z5", "z6"]);
        let relevant = ids(&["a", "b", "c", "d"]);
        assert_eq!(recall_at_k(&returned, &relevant, 10), 0.5);
    }

    #[test]
    fn recall_at_k_empty_relevant_is_1() {
        let returned = ids(&["a", "b", "c"]);
        let relevant: Vec<String> = vec![];
        assert_eq!(recall_at_k(&returned, &relevant, 10), 1.0);
    }

    // ── ndcg_at_k ────────────────────────────────────────────────────────────

    #[test]
    fn ndcg_at_k_perfect() {
        // returned exactly matches relevant in ideal order → nDCG = 1.0
        let returned = ids(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]);
        let relevant = ids(&["a", "b", "c"]);
        let score = ndcg_at_k(&returned, &relevant, 10);
        assert!((score - 1.0).abs() < 1e-9, "expected 1.0, got {score}");
    }

    #[test]
    fn ndcg_at_k_wrong_order() {
        // relevant items present but in non-ideal positions → 0 < nDCG < 1
        let returned = ids(&["x", "y", "a", "b", "c", "z1", "z2", "z3", "z4", "z5"]);
        let relevant = ids(&["a", "b", "c"]);
        let score = ndcg_at_k(&returned, &relevant, 10);
        assert!(
            score > 0.0 && score < 1.0,
            "expected 0 < nDCG < 1, got {score}"
        );
    }

    #[test]
    fn ndcg_at_k_no_relevant_in_top_k() {
        let returned = ids(&["x1", "x2", "x3", "x4", "x5", "x6", "x7", "x8", "x9", "x10"]);
        let relevant = ids(&["a", "b", "c"]);
        let score = ndcg_at_k(&returned, &relevant, 10);
        assert_eq!(score, 0.0);
    }

    // ── Helper: build a PassingEvalResult above all thresholds ───────────────

    fn passing_eval(profile_id: &str, query_count: usize) -> ProfileEvalResult {
        ProfileEvalResult {
            profile_id: profile_id.to_string(),
            query_count,
            recall_at_k: 0.90,
            ndcg_at_k: 0.85,
            identifier_phrase_recall: 0.97,
            forbidden_leak_rate: 0.0,
            class_metrics: vec![],
            bootstrap_cis: vec![],
        }
    }

    // ── evaluate_profile ─────────────────────────────────────────────────────

    #[test]
    fn evaluate_profile_forbidden_leak_detected() {
        let gate = ActivationGate {
            min_query_count: 1,
            ..ActivationGate::default()
        };
        let queries = vec![JudgedQuery {
            query: "find uuid-1234".to_string(),
            returned_ids: ids(&["r1", "forbidden-doc", "r3"]),
            relevant_ids: ids(&["r1", "r3"]),
            forbidden_ids: ids(&["forbidden-doc"]),
        }];
        let result = evaluate_profile(&queries, "test-profile", &gate);
        assert!(
            result.forbidden_leak_rate > 0.0,
            "expected forbidden_leak_rate > 0, got {}",
            result.forbidden_leak_rate
        );
    }

    #[test]
    fn evaluate_profile_no_forbidden_leak() {
        let gate = ActivationGate {
            min_query_count: 1,
            ..ActivationGate::default()
        };
        let queries = vec![JudgedQuery {
            query: "search something".to_string(),
            returned_ids: ids(&["r1", "r2", "r3"]),
            relevant_ids: ids(&["r1", "r2"]),
            forbidden_ids: ids(&["forbidden-doc"]),
        }];
        let result = evaluate_profile(&queries, "test-profile", &gate);
        assert_eq!(result.forbidden_leak_rate, 0.0);
    }

    // ── check_activation ─────────────────────────────────────────────────────

    #[test]
    fn check_activation_approved_when_all_pass() {
        let gate = ActivationGate::default();
        let candidate = passing_eval("candidate", 250);
        let decision = check_activation(&candidate, None, &gate);
        assert_eq!(decision, ActivationDecision::Approved);
    }

    #[test]
    fn check_activation_rejected_low_recall() {
        let gate = ActivationGate::default();
        let candidate = ProfileEvalResult {
            recall_at_k: 0.80, // below min 0.85
            ..passing_eval("candidate", 250)
        };
        match check_activation(&candidate, None, &gate) {
            ActivationDecision::Rejected(reasons) => {
                assert!(
                    reasons.iter().any(|r| r.contains("recall@10")),
                    "expected recall reason, got: {reasons:?}"
                );
            }
            ActivationDecision::Approved => panic!("expected Rejected"),
        }
    }

    #[test]
    fn check_activation_rejected_forbidden_leak() {
        let gate = ActivationGate::default();
        let candidate = ProfileEvalResult {
            forbidden_leak_rate: 0.05,
            ..passing_eval("candidate", 250)
        };
        match check_activation(&candidate, None, &gate) {
            ActivationDecision::Rejected(reasons) => {
                assert!(
                    reasons.iter().any(|r| r.contains("forbidden")),
                    "expected forbidden reason, got: {reasons:?}"
                );
            }
            ActivationDecision::Approved => panic!("expected Rejected"),
        }
    }

    #[test]
    fn check_activation_regression_above_0_03() {
        // baseline recall=0.90, candidate recall=0.86 → regression=0.04 > 0.03
        let gate = ActivationGate::default();
        let baseline = passing_eval("baseline", 250); // recall=0.90
        let candidate = ProfileEvalResult {
            recall_at_k: 0.86,
            ..passing_eval("candidate", 250)
        };
        match check_activation(&candidate, Some(&baseline), &gate) {
            ActivationDecision::Rejected(reasons) => {
                assert!(
                    reasons.iter().any(|r| r.contains("recall@10 regression")),
                    "expected recall regression reason, got: {reasons:?}"
                );
            }
            ActivationDecision::Approved => panic!("expected Rejected due to regression"),
        }
    }

    #[test]
    fn check_activation_regression_within_0_03() {
        // baseline recall=0.90, candidate recall=0.88 → regression=0.02 ≤ 0.03 → not rejected
        let gate = ActivationGate::default();
        let baseline = passing_eval("baseline", 250); // recall=0.90
        let candidate = ProfileEvalResult {
            recall_at_k: 0.88,
            ..passing_eval("candidate", 250)
        };
        // Should not be rejected solely on recall regression
        let decision = check_activation(&candidate, Some(&baseline), &gate);
        if let ActivationDecision::Rejected(ref reasons) = decision {
            assert!(
                !reasons.iter().any(|r| r.contains("recall@10 regression")),
                "recall regression should not trigger at 0.02, but got: {reasons:?}"
            );
        }
        // May still approve
        assert_eq!(decision, ActivationDecision::Approved);
    }

    #[test]
    fn check_activation_no_baseline_skips_regression() {
        // No baseline → regression check is skipped entirely
        let gate = ActivationGate::default();
        let candidate = passing_eval("candidate", 250);
        let decision = check_activation(&candidate, None, &gate);
        assert_eq!(decision, ActivationDecision::Approved);
    }

    #[test]
    fn check_activation_rejected_insufficient_query_count() {
        let gate = ActivationGate::default(); // min_query_count = 200
        let candidate = ProfileEvalResult {
            query_count: 150,
            ..passing_eval("candidate", 150)
        };
        match check_activation(&candidate, None, &gate) {
            ActivationDecision::Rejected(reasons) => {
                assert!(
                    reasons
                        .iter()
                        .any(|r| r.contains("insufficient query count")),
                    "expected query count reason, got: {reasons:?}"
                );
            }
            ActivationDecision::Approved => panic!("expected Rejected"),
        }
    }

    #[test]
    fn activation_gate_default_values() {
        let gate = ActivationGate::default();
        assert_eq!(gate.min_recall, 0.85);
        assert_eq!(gate.min_ndcg, 0.80);
        assert_eq!(gate.min_identifier_phrase_recall, 0.95);
        assert_eq!(gate.max_forbidden_leak_rate, 0.0);
        assert_eq!(gate.max_regression, 0.03);
        assert_eq!(gate.min_query_count, 200);
        assert_eq!(gate.bootstrap_resamples, 100);
    }
}
