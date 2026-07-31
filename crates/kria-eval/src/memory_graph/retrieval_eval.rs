//! Retrieval quality evaluation for the Memory Graph Production Redesign
//! spec (task F3.9 / 3.9.7).
//!
//! This module implements the metric functions and batch evaluation harness
//! required by `V-RET-03` to prove retrieval quality thresholds before
//! declaring F3 complete.
//!
//! ## Thresholds
//!
//! | Metric                          | Threshold |
//! |---------------------------------|-----------|
//! | Recall@10                       | ≥ 0.85    |
//! | nDCG@10                         | ≥ 0.80    |
//! | Identifier/Phrase recall        | ≥ 0.95    |
//! | Forbidden/deleted exclusion     | 100%      |
//!
//! ## Evidence
//!
//! The full 200+ judged-query run requires the `CMD-MG-EVAL` binary
//! (NBW-F3-01). Metric functions are implemented and unit-tested here;
//! execution against live fixtures is deferred to the F5 CMD-MG-EVAL run.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Query classification
// ---------------------------------------------------------------------------

/// The class of a judged query. Per-class metrics are computed in
/// [`evaluate_batch`] and reflected in [`RetrievalMetrics`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryClass {
    /// Exact identifier lookup (e.g. variable name, function name).
    Identifier,
    /// Phrase / keyword query spanning multiple tokens.
    Phrase,
    /// Semantic entity search (concept-level, not keyword-exact).
    SemanticEntity,
    /// Temporal / time-scoped retrieval.
    Temporal,
    /// Goal-oriented retrieval.
    Goal,
    /// Open-ended exploratory search.
    Exploratory,
}

impl QueryClass {
    /// Stable machine code for this class, used in reports.
    pub fn code(self) -> &'static str {
        match self {
            QueryClass::Identifier => "identifier",
            QueryClass::Phrase => "phrase",
            QueryClass::SemanticEntity => "semantic_entity",
            QueryClass::Temporal => "temporal",
            QueryClass::Goal => "goal",
            QueryClass::Exploratory => "exploratory",
        }
    }

    /// Whether this class is an identifier or phrase query (higher threshold
    /// applies: ≥ 0.95).
    pub fn is_identifier_or_phrase(self) -> bool {
        matches!(self, QueryClass::Identifier | QueryClass::Phrase)
    }
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Aggregate retrieval quality metrics across a judged query batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalMetrics {
    /// Fraction of relevant IDs appearing in the top-10 retrieved results,
    /// averaged over all queries in the batch.
    pub recall_at_10: f64,
    /// Normalized Discounted Cumulative Gain at rank 10, averaged over all
    /// queries in the batch.
    pub ndcg_at_10: f64,
    /// Recall@10 restricted to `Identifier` and `Phrase` class queries.
    pub identifier_phrase_recall: f64,
    /// Fraction of queries for which no forbidden/deleted/forgotten/
    /// default-superseded ID appeared in the retrieved results. Must be 1.0.
    pub exclusion_rate: f64,
    /// Number of judged queries evaluated.
    pub sample_size: usize,
}

impl Default for RetrievalMetrics {
    fn default() -> Self {
        RetrievalMetrics {
            recall_at_10: 0.0,
            ndcg_at_10: 0.0,
            identifier_phrase_recall: 0.0,
            exclusion_rate: 1.0,
            sample_size: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Core metric functions
// ---------------------------------------------------------------------------

/// Compute Recall@k: fraction of relevant IDs that appear in the top-k
/// retrieved results.
///
/// Only the first `k` elements of `retrieved_ids` are considered. If
/// `relevant_ids` is empty, returns `1.0` (vacuously all relevant items
/// retrieved). Returns `0.0` when `retrieved_ids` is empty and `relevant_ids`
/// is non-empty.
///
/// # Arguments
///
/// * `retrieved_ids` — ordered list of retrieved document IDs (most relevant
///   first). Only the first `k` entries are examined.
/// * `relevant_ids` — the complete set of ground-truth relevant IDs for the
///   query (order does not matter).
/// * `k` — cut-off rank. Typically `10`.
pub fn compute_recall_at_k(retrieved_ids: &[String], relevant_ids: &[String], k: usize) -> f64 {
    if relevant_ids.is_empty() {
        return 1.0;
    }
    if retrieved_ids.is_empty() || k == 0 {
        return 0.0;
    }
    let top_k = retrieved_ids.iter().take(k).collect::<std::collections::HashSet<_>>();
    let hits = relevant_ids.iter().filter(|r| top_k.contains(r)).count();
    hits as f64 / relevant_ids.len() as f64
}

/// Compute nDCG@k: Normalized Discounted Cumulative Gain at rank `k`.
///
/// Uses binary relevance: a retrieved ID is relevant (gain = 1) if it appears
/// in `relevant_ids`, irrelevant otherwise (gain = 0). The ideal DCG is
/// computed by placing all relevant items at the top `k` ranks.
///
/// Returns `1.0` when `relevant_ids` is empty (vacuously perfect) and `0.0`
/// when `retrieved_ids` is empty and `relevant_ids` is non-empty.
///
/// # Arguments
///
/// * `retrieved_ids` — ordered retrieved list. Only the first `k` are used.
/// * `relevant_ids` — ground-truth relevant set.
/// * `k` — rank cut-off.
pub fn compute_ndcg_at_k(retrieved_ids: &[String], relevant_ids: &[String], k: usize) -> f64 {
    if relevant_ids.is_empty() {
        return 1.0;
    }
    if retrieved_ids.is_empty() || k == 0 {
        return 0.0;
    }

    let relevant_set: std::collections::HashSet<&String> = relevant_ids.iter().collect();

    // DCG@k: sum of gain/log2(rank+1) for top-k retrieved items (1-indexed).
    let dcg: f64 = retrieved_ids
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, id)| {
            let gain = if relevant_set.contains(id) { 1.0 } else { 0.0 };
            let rank = i + 2; // log2(rank + 1) with rank starting at 1 → denominator = log2(i+2)
            gain / (rank as f64).log2()
        })
        .sum();

    // IDCG@k: perfect ranking places all relevant items first.
    let ideal_hits = relevant_ids.len().min(k);
    let idcg: f64 = (0..ideal_hits)
        .map(|i| {
            let rank = i + 2;
            1.0 / (rank as f64).log2()
        })
        .sum();

    if idcg == 0.0 {
        return 0.0;
    }
    (dcg / idcg).min(1.0)
}

/// Check that no forbidden ID appears in the retrieved results.
///
/// Returns `true` when the retrieved set is clean (passes the exclusion
/// requirement). Returns `false` when any ID in `forbidden_ids` appears in
/// `retrieved_ids` (at any rank).
///
/// An empty `forbidden_ids` slice always returns `true`.
///
/// # Arguments
///
/// * `retrieved_ids` — the full retrieved list for a query (all ranks).
/// * `forbidden_ids` — IDs that must not appear (deleted, forgotten,
///   superseded-by-default, etc.).
pub fn check_exclusion(retrieved_ids: &[String], forbidden_ids: &[String]) -> bool {
    if forbidden_ids.is_empty() {
        return true;
    }
    let retrieved_set: std::collections::HashSet<&String> = retrieved_ids.iter().collect();
    !forbidden_ids.iter().any(|f| retrieved_set.contains(f))
}

// ---------------------------------------------------------------------------
// Batch evaluation
// ---------------------------------------------------------------------------

/// The pass/fail thresholds applied by [`evaluate_batch`].
pub struct EvalThresholds {
    pub recall_at_10: f64,
    pub ndcg_at_10: f64,
    pub identifier_phrase_recall: f64,
    /// Must be exactly `1.0`; any exclusion violation fails the gate.
    pub exclusion_rate: f64,
}

impl Default for EvalThresholds {
    fn default() -> Self {
        EvalThresholds {
            recall_at_10: 0.85,
            ndcg_at_10: 0.80,
            identifier_phrase_recall: 0.95,
            exclusion_rate: 1.0,
        }
    }
}

/// The complete result of a batch retrieval evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalEvalResult {
    /// Aggregate and per-class metrics computed over the batch.
    pub metrics: RetrievalMetrics,
    /// `true` when all thresholds pass.
    pub passed: bool,
    /// Human-readable description of each threshold that was not met.
    pub failure_reasons: Vec<String>,
}

/// Evaluate a batch of judged queries and check all quality thresholds.
///
/// Each element of `queries` is a tuple of:
/// * `retrieved_ids` — ordered retrieved list for this query,
/// * `relevant_ids` — ground-truth relevant set,
/// * `class` — the [`QueryClass`] for per-class breakdown.
///
/// `forbidden_ids` is the global set of IDs that must never appear in any
/// retrieved result (deleted, forgotten, superseded-by-default, etc.).
///
/// Returns a [`RetrievalEvalResult`] with the computed metrics, a pass/fail
/// flag, and a list of failure reasons for any threshold not met.
pub fn evaluate_batch(
    queries: &[(Vec<String>, Vec<String>, QueryClass)],
    forbidden_ids: &[String],
) -> RetrievalEvalResult {
    let thresholds = EvalThresholds::default();
    evaluate_batch_with_thresholds(queries, forbidden_ids, &thresholds)
}

/// Like [`evaluate_batch`] but with caller-supplied thresholds — useful for
/// testing and calibration.
pub fn evaluate_batch_with_thresholds(
    queries: &[(Vec<String>, Vec<String>, QueryClass)],
    forbidden_ids: &[String],
    thresholds: &EvalThresholds,
) -> RetrievalEvalResult {
    if queries.is_empty() {
        return RetrievalEvalResult {
            metrics: RetrievalMetrics::default(),
            passed: true,
            failure_reasons: Vec::new(),
        };
    }

    let mut recall_sum = 0.0f64;
    let mut ndcg_sum = 0.0f64;
    let mut id_phrase_recall_sum = 0.0f64;
    let mut id_phrase_count = 0usize;
    let mut exclusion_pass_count = 0usize;

    for (retrieved, relevant, class) in queries {
        recall_sum += compute_recall_at_k(retrieved, relevant, 10);
        ndcg_sum += compute_ndcg_at_k(retrieved, relevant, 10);

        if class.is_identifier_or_phrase() {
            id_phrase_recall_sum += compute_recall_at_k(retrieved, relevant, 10);
            id_phrase_count += 1;
        }

        if check_exclusion(retrieved, forbidden_ids) {
            exclusion_pass_count += 1;
        }
    }

    let n = queries.len() as f64;
    let recall_at_10 = recall_sum / n;
    let ndcg_at_10 = ndcg_sum / n;
    let identifier_phrase_recall = if id_phrase_count > 0 {
        id_phrase_recall_sum / id_phrase_count as f64
    } else {
        1.0 // vacuously satisfied when no identifier/phrase queries present
    };
    let exclusion_rate = exclusion_pass_count as f64 / n;

    let metrics = RetrievalMetrics {
        recall_at_10,
        ndcg_at_10,
        identifier_phrase_recall,
        exclusion_rate,
        sample_size: queries.len(),
    };

    let mut failure_reasons = Vec::new();

    if recall_at_10 < thresholds.recall_at_10 {
        failure_reasons.push(format!(
            "Recall@10 {:.4} < threshold {:.2}",
            recall_at_10, thresholds.recall_at_10
        ));
    }
    if ndcg_at_10 < thresholds.ndcg_at_10 {
        failure_reasons.push(format!(
            "nDCG@10 {:.4} < threshold {:.2}",
            ndcg_at_10, thresholds.ndcg_at_10
        ));
    }
    if id_phrase_count > 0 && identifier_phrase_recall < thresholds.identifier_phrase_recall {
        failure_reasons.push(format!(
            "Identifier/Phrase Recall@10 {:.4} < threshold {:.2}",
            identifier_phrase_recall, thresholds.identifier_phrase_recall
        ));
    }
    if exclusion_rate < thresholds.exclusion_rate {
        failure_reasons.push(format!(
            "Exclusion rate {:.4} < threshold {:.2} ({} of {} queries violated)",
            exclusion_rate,
            thresholds.exclusion_rate,
            queries.len() - exclusion_pass_count,
            queries.len()
        ));
    }

    let passed = failure_reasons.is_empty();
    RetrievalEvalResult {
        metrics,
        passed,
        failure_reasons,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // -- compute_recall_at_k -------------------------------------------------

    #[test]
    fn recall_perfect_retrieval_is_one() {
        let retrieved = s(&["a", "b", "c"]);
        let relevant = s(&["a", "b", "c"]);
        assert_eq!(compute_recall_at_k(&retrieved, &relevant, 10), 1.0);
    }

    #[test]
    fn recall_empty_retrieval_is_zero() {
        let retrieved: Vec<String> = vec![];
        let relevant = s(&["a", "b"]);
        assert_eq!(compute_recall_at_k(&retrieved, &relevant, 10), 0.0);
    }

    #[test]
    fn recall_partial_retrieval() {
        // 2 of 4 relevant in top-10 → 0.5
        let retrieved = s(&["a", "b", "x", "y"]);
        let relevant = s(&["a", "b", "c", "d"]);
        let r = compute_recall_at_k(&retrieved, &relevant, 10);
        assert!((r - 0.5).abs() < 1e-9, "expected 0.5, got {r}");
    }

    #[test]
    fn recall_respects_k_cutoff() {
        // relevant item "c" is at rank 11, beyond k=10 → should not count
        let mut retrieved: Vec<String> = (0..10).map(|i| format!("x{i}")).collect();
        retrieved.push("c".to_string()); // rank 11
        let relevant = s(&["c"]);
        assert_eq!(compute_recall_at_k(&retrieved, &relevant, 10), 0.0);
    }

    #[test]
    fn recall_empty_relevant_is_one() {
        let retrieved = s(&["a", "b"]);
        let relevant: Vec<String> = vec![];
        assert_eq!(compute_recall_at_k(&retrieved, &relevant, 10), 1.0);
    }

    // -- compute_ndcg_at_k ---------------------------------------------------

    #[test]
    fn ndcg_perfect_order_is_one() {
        // All relevant items are the first results → nDCG = 1.0
        let retrieved = s(&["a", "b", "c"]);
        let relevant = s(&["a", "b", "c"]);
        let score = compute_ndcg_at_k(&retrieved, &relevant, 10);
        assert!(
            (score - 1.0).abs() < 1e-9,
            "expected 1.0, got {score}"
        );
    }

    #[test]
    fn ndcg_empty_retrieval_is_zero() {
        let retrieved: Vec<String> = vec![];
        let relevant = s(&["a", "b"]);
        assert_eq!(compute_ndcg_at_k(&retrieved, &relevant, 10), 0.0);
    }

    #[test]
    fn ndcg_empty_relevant_is_one() {
        let retrieved = s(&["a"]);
        let relevant: Vec<String> = vec![];
        assert_eq!(compute_ndcg_at_k(&retrieved, &relevant, 10), 1.0);
    }

    #[test]
    fn ndcg_no_relevant_in_results_is_zero() {
        let retrieved = s(&["x", "y", "z"]);
        let relevant = s(&["a", "b"]);
        assert_eq!(compute_ndcg_at_k(&retrieved, &relevant, 10), 0.0);
    }

    #[test]
    fn ndcg_bounded_by_one() {
        // Verify that nDCG never exceeds 1.0 on any well-formed input.
        let retrieved = s(&["a", "b", "c", "d", "e"]);
        let relevant = s(&["a", "b"]);
        let score = compute_ndcg_at_k(&retrieved, &relevant, 10);
        assert!(score <= 1.0, "nDCG must not exceed 1.0; got {score}");
    }

    // -- check_exclusion -----------------------------------------------------

    #[test]
    fn exclusion_forbidden_id_present_returns_false() {
        let retrieved = s(&["a", "b", "forbidden-id", "c"]);
        let forbidden = s(&["forbidden-id"]);
        assert!(!check_exclusion(&retrieved, &forbidden));
    }

    #[test]
    fn exclusion_no_forbidden_ids_returns_true() {
        let retrieved = s(&["a", "b", "c"]);
        let forbidden: Vec<String> = vec![];
        assert!(check_exclusion(&retrieved, &forbidden));
    }

    #[test]
    fn exclusion_clean_retrieval_returns_true() {
        let retrieved = s(&["a", "b", "c"]);
        let forbidden = s(&["x", "y", "z"]);
        assert!(check_exclusion(&retrieved, &forbidden));
    }

    // -- evaluate_batch ------------------------------------------------------

    /// Build 5 perfect queries that all pass every threshold.
    fn perfect_queries() -> Vec<(Vec<String>, Vec<String>, QueryClass)> {
        vec![
            (s(&["a", "b", "c"]), s(&["a", "b", "c"]), QueryClass::Identifier),
            (s(&["d", "e"]),      s(&["d", "e"]),       QueryClass::Phrase),
            (s(&["f"]),           s(&["f"]),             QueryClass::SemanticEntity),
            (s(&["g", "h"]),      s(&["g", "h"]),       QueryClass::Temporal),
            (s(&["i"]),           s(&["i"]),             QueryClass::Goal),
        ]
    }

    #[test]
    fn batch_all_passing_queries_yields_passed_true() {
        let queries = perfect_queries();
        let result = evaluate_batch(&queries, &[]);
        assert!(result.passed, "expected passed=true; reasons: {:?}", result.failure_reasons);
        assert!(result.failure_reasons.is_empty());
        assert_eq!(result.metrics.sample_size, 5);
        assert!((result.metrics.recall_at_10 - 1.0).abs() < 1e-9);
        assert!((result.metrics.ndcg_at_10 - 1.0).abs() < 1e-9);
        assert!((result.metrics.exclusion_rate - 1.0).abs() < 1e-9);
    }

    #[test]
    fn batch_fails_when_recall_below_threshold() {
        // All queries return nothing relevant → recall = 0.0
        let queries = vec![
            (s(&["x"]), s(&["a"]), QueryClass::SemanticEntity),
            (s(&["y"]), s(&["b"]), QueryClass::Goal),
        ];
        let result = evaluate_batch(&queries, &[]);
        assert!(!result.passed);
        assert!(result.failure_reasons.iter().any(|r| r.contains("Recall@10")));
    }

    #[test]
    fn batch_fails_when_exclusion_violated() {
        let forbidden = s(&["bad-id"]);
        let queries = vec![
            // This query returns a forbidden ID
            (s(&["bad-id", "a"]), s(&["a"]), QueryClass::Exploratory),
        ];
        let result = evaluate_batch(&queries, &forbidden);
        assert!(!result.passed);
        assert!(result.failure_reasons.iter().any(|r| r.contains("Exclusion")));
        assert!((result.metrics.exclusion_rate - 0.0).abs() < 1e-9);
    }

    #[test]
    fn batch_empty_queries_passes_vacuously() {
        let result = evaluate_batch(&[], &[]);
        assert!(result.passed);
        assert_eq!(result.metrics.sample_size, 0);
    }

    #[test]
    fn identifier_phrase_recall_computed_only_for_relevant_classes() {
        // One identifier query with perfect recall, one exploratory with zero recall.
        let queries = vec![
            (s(&["a"]), s(&["a"]), QueryClass::Identifier),
            (s(&["x"]), s(&["b"]), QueryClass::Exploratory),
        ];
        let result = evaluate_batch(&queries, &[]);
        // identifier_phrase_recall covers only the first query → should be 1.0
        assert!(
            (result.metrics.identifier_phrase_recall - 1.0).abs() < 1e-9,
            "expected 1.0 id/phrase recall; got {}",
            result.metrics.identifier_phrase_recall
        );
    }
}
