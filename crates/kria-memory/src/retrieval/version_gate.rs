//! Gate 3 of the retrieval pipeline: model/record/content version and
//! Valid Time (design §6.4 step 3, task F3.5.2).
//!
//! Also provides deduplication by semantic ID / content version (gate step 4).
//!
//! # Design invariants
//! * Model partition ID must match the expected partition (or be absent/unknown).
//! * Record schema version must be within the expected range.
//! * Valid Time gate: valid_from <= query_time AND (valid_until IS NULL OR valid_until > query_time).
//! * Deduplication: keep only the lowest-rank occurrence of each (semantic_id, content_version) pair.

use chrono::{DateTime, Utc};
use std::collections::HashMap;

// ── Constraints ───────────────────────────────────────────────────────────────

/// Expected model and schema version constraints for a retrieval call.
#[derive(Debug, Clone)]
pub struct VersionConstraints {
    /// Expected embedding model partition ID.
    /// When `None`, model version check is skipped.
    pub expected_model_partition: Option<String>,
    /// Minimum acceptable record schema version (inclusive).
    /// When `None`, no minimum is enforced.
    pub min_schema_version: Option<i64>,
    /// Maximum acceptable record schema version (inclusive).
    /// When `None`, no maximum is enforced.
    pub max_schema_version: Option<i64>,
    /// The point-in-time instant to use for Valid Time filtering.
    /// When `None`, Valid Time check is skipped.
    pub query_time: Option<DateTime<Utc>>,
}

impl Default for VersionConstraints {
    fn default() -> Self {
        Self {
            expected_model_partition: None,
            min_schema_version: None,
            max_schema_version: None,
            query_time: None,
        }
    }
}

// ── Candidate version info ────────────────────────────────────────────────────

/// Version and time metadata for one candidate, provided by the strategy.
#[derive(Debug, Clone)]
pub struct CandidateVersionInfo {
    /// Candidate's semantic ID (for deduplication).
    pub semantic_id: String,
    /// Candidate's content version hash (for deduplication).
    pub content_version: String,
    /// The model partition this candidate's embedding belongs to.
    /// `None` if candidate is not vector-based.
    pub model_partition_id: Option<String>,
    /// Record schema version.
    pub schema_version: Option<i64>,
    /// Valid interval start (inclusive). `None` = no start constraint.
    pub valid_from: Option<DateTime<Utc>>,
    /// Valid interval end (exclusive). `None` = no end constraint / open-ended.
    pub valid_until: Option<DateTime<Utc>>,
}

// ── Gate disposition ──────────────────────────────────────────────────────────

/// Disposition from the version/time gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionGateDisposition {
    Pass,
    Excluded { reason: VersionExclusionReason },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionExclusionReason {
    /// Model partition ID doesn't match expected.
    ModelPartitionMismatch,
    /// Schema version is outside the acceptable range.
    SchemaVersionOutOfRange,
    /// Candidate's Valid Time does not intersect the query time.
    ValidTimeNotSatisfied,
}

// ── Gate function ─────────────────────────────────────────────────────────────

/// Evaluate model/record/content version and Valid Time for one candidate.
///
/// Gate order (fixed):
/// 1. Model partition check: if constraints.expected_model_partition is Some(p) AND
///    candidate.model_partition_id is Some(q) AND p != q → ModelPartitionMismatch.
///    (Skip if either is None.)
/// 2. Schema version check: if candidate.schema_version is Some(v):
///    - if constraints.min_schema_version is Some(min) AND v < min → SchemaVersionOutOfRange
///    - if constraints.max_schema_version is Some(max) AND v > max → SchemaVersionOutOfRange
/// 3. Valid Time: if constraints.query_time is Some(t):
///    - if candidate.valid_from is Some(vf) AND vf > t → ValidTimeNotSatisfied
///    - if candidate.valid_until is Some(vu) AND vu <= t → ValidTimeNotSatisfied
///    (valid_from is inclusive: vf <= t; valid_until is exclusive: vu > t)
pub fn evaluate_version_gate(
    constraints: &VersionConstraints,
    candidate: &CandidateVersionInfo,
) -> VersionGateDisposition {
    // Gate 1: Model partition check — skip if either side is None.
    if let (Some(expected), Some(actual)) = (
        &constraints.expected_model_partition,
        &candidate.model_partition_id,
    ) {
        if expected != actual {
            return VersionGateDisposition::Excluded {
                reason: VersionExclusionReason::ModelPartitionMismatch,
            };
        }
    }

    // Gate 2: Schema version range check — only when candidate has a version.
    if let Some(v) = candidate.schema_version {
        if let Some(min) = constraints.min_schema_version {
            if v < min {
                return VersionGateDisposition::Excluded {
                    reason: VersionExclusionReason::SchemaVersionOutOfRange,
                };
            }
        }
        if let Some(max) = constraints.max_schema_version {
            if v > max {
                return VersionGateDisposition::Excluded {
                    reason: VersionExclusionReason::SchemaVersionOutOfRange,
                };
            }
        }
    }

    // Gate 3: Valid Time — only when a query_time is set.
    if let Some(t) = constraints.query_time {
        // valid_from is inclusive: vf must be <= t
        if let Some(vf) = candidate.valid_from {
            if vf > t {
                return VersionGateDisposition::Excluded {
                    reason: VersionExclusionReason::ValidTimeNotSatisfied,
                };
            }
        }
        // valid_until is exclusive: vu must be > t
        if let Some(vu) = candidate.valid_until {
            if vu <= t {
                return VersionGateDisposition::Excluded {
                    reason: VersionExclusionReason::ValidTimeNotSatisfied,
                };
            }
        }
    }

    VersionGateDisposition::Pass
}

// ── Deduplication ─────────────────────────────────────────────────────────────

/// Deduplicate candidates by (semantic_id, content_version), keeping the lowest rank.
///
/// Input: slice of (semantic_id, content_version, rank) tuples.
/// Output: unique (semantic_id, content_version) pairs with their best (lowest) rank.
/// Stable output ordering: by best_rank ASC, then semantic_id ASC.
pub fn dedup_by_semantic_version(
    candidates: &[(String, String, u32)],
) -> Vec<(String, String, u32)> {
    // Map (semantic_id, content_version) → best (lowest) rank seen so far.
    let mut best: HashMap<(String, String), u32> = HashMap::new();

    for (semantic_id, content_version, rank) in candidates {
        let key = (semantic_id.clone(), content_version.clone());
        let entry = best.entry(key).or_insert(*rank);
        if *rank < *entry {
            *entry = *rank;
        }
    }

    // Collect into a vec and apply stable ordering: rank ASC, then semantic_id ASC.
    let mut result: Vec<(String, String, u32)> = best
        .into_iter()
        .map(|((sid, cv), rank)| (sid, cv, rank))
        .collect();

    result.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.0.cmp(&b.0)));

    result
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn make_t(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
    }

    fn simple_candidate() -> CandidateVersionInfo {
        CandidateVersionInfo {
            semantic_id: "id-1".to_string(),
            content_version: "v1".to_string(),
            model_partition_id: None,
            schema_version: None,
            valid_from: None,
            valid_until: None,
        }
    }

    // ── Version gate tests ────────────────────────────────────────────────────

    #[test]
    fn pass_no_constraints() {
        let constraints = VersionConstraints::default();
        let candidate = simple_candidate();
        assert_eq!(
            evaluate_version_gate(&constraints, &candidate),
            VersionGateDisposition::Pass
        );
    }

    #[test]
    fn pass_matching_model_partition() {
        let constraints = VersionConstraints {
            expected_model_partition: Some("partition-a".to_string()),
            ..Default::default()
        };
        let candidate = CandidateVersionInfo {
            model_partition_id: Some("partition-a".to_string()),
            ..simple_candidate()
        };
        assert_eq!(
            evaluate_version_gate(&constraints, &candidate),
            VersionGateDisposition::Pass
        );
    }

    #[test]
    fn fail_model_partition_mismatch() {
        let constraints = VersionConstraints {
            expected_model_partition: Some("partition-a".to_string()),
            ..Default::default()
        };
        let candidate = CandidateVersionInfo {
            model_partition_id: Some("partition-b".to_string()),
            ..simple_candidate()
        };
        assert_eq!(
            evaluate_version_gate(&constraints, &candidate),
            VersionGateDisposition::Excluded {
                reason: VersionExclusionReason::ModelPartitionMismatch
            }
        );
    }

    #[test]
    fn pass_model_partition_skip_when_candidate_has_none() {
        // constraints has expected partition, but candidate has no partition → skip check
        let constraints = VersionConstraints {
            expected_model_partition: Some("partition-a".to_string()),
            ..Default::default()
        };
        let candidate = CandidateVersionInfo {
            model_partition_id: None,
            ..simple_candidate()
        };
        assert_eq!(
            evaluate_version_gate(&constraints, &candidate),
            VersionGateDisposition::Pass
        );
    }

    #[test]
    fn fail_schema_version_too_low() {
        let constraints = VersionConstraints {
            min_schema_version: Some(5),
            ..Default::default()
        };
        let candidate = CandidateVersionInfo {
            schema_version: Some(4),
            ..simple_candidate()
        };
        assert_eq!(
            evaluate_version_gate(&constraints, &candidate),
            VersionGateDisposition::Excluded {
                reason: VersionExclusionReason::SchemaVersionOutOfRange
            }
        );
    }

    #[test]
    fn fail_schema_version_too_high() {
        let constraints = VersionConstraints {
            max_schema_version: Some(10),
            ..Default::default()
        };
        let candidate = CandidateVersionInfo {
            schema_version: Some(11),
            ..simple_candidate()
        };
        assert_eq!(
            evaluate_version_gate(&constraints, &candidate),
            VersionGateDisposition::Excluded {
                reason: VersionExclusionReason::SchemaVersionOutOfRange
            }
        );
    }

    #[test]
    fn pass_schema_version_at_min() {
        let constraints = VersionConstraints {
            min_schema_version: Some(5),
            ..Default::default()
        };
        let candidate = CandidateVersionInfo {
            schema_version: Some(5),
            ..simple_candidate()
        };
        assert_eq!(
            evaluate_version_gate(&constraints, &candidate),
            VersionGateDisposition::Pass
        );
    }

    #[test]
    fn pass_schema_version_at_max() {
        let constraints = VersionConstraints {
            max_schema_version: Some(10),
            ..Default::default()
        };
        let candidate = CandidateVersionInfo {
            schema_version: Some(10),
            ..simple_candidate()
        };
        assert_eq!(
            evaluate_version_gate(&constraints, &candidate),
            VersionGateDisposition::Pass
        );
    }

    #[test]
    fn pass_no_valid_time_constraint() {
        // query_time is None → Valid Time gate skipped entirely
        let constraints = VersionConstraints::default();
        let candidate = CandidateVersionInfo {
            valid_from: Some(make_t(2030, 1, 1)),
            valid_until: Some(make_t(2020, 1, 1)),
            ..simple_candidate()
        };
        // Even with nonsensical valid_from/valid_until, no query_time → Pass
        assert_eq!(
            evaluate_version_gate(&constraints, &candidate),
            VersionGateDisposition::Pass
        );
    }

    #[test]
    fn fail_valid_from_after_query_time() {
        let query = make_t(2024, 6, 1);
        let constraints = VersionConstraints {
            query_time: Some(query),
            ..Default::default()
        };
        let candidate = CandidateVersionInfo {
            valid_from: Some(make_t(2024, 7, 1)), // starts after query
            ..simple_candidate()
        };
        assert_eq!(
            evaluate_version_gate(&constraints, &candidate),
            VersionGateDisposition::Excluded {
                reason: VersionExclusionReason::ValidTimeNotSatisfied
            }
        );
    }

    #[test]
    fn fail_valid_until_at_or_before_query_time() {
        // valid_until is exclusive, so valid_until == query_time → excluded
        let query = make_t(2024, 6, 1);
        let constraints = VersionConstraints {
            query_time: Some(query),
            ..Default::default()
        };
        let candidate = CandidateVersionInfo {
            valid_until: Some(make_t(2024, 6, 1)), // equal → excluded (exclusive)
            ..simple_candidate()
        };
        assert_eq!(
            evaluate_version_gate(&constraints, &candidate),
            VersionGateDisposition::Excluded {
                reason: VersionExclusionReason::ValidTimeNotSatisfied
            }
        );
    }

    #[test]
    fn pass_valid_from_equal_query_time() {
        // valid_from is inclusive: vf == t → Pass
        let query = make_t(2024, 6, 1);
        let constraints = VersionConstraints {
            query_time: Some(query),
            ..Default::default()
        };
        let candidate = CandidateVersionInfo {
            valid_from: Some(make_t(2024, 6, 1)),
            ..simple_candidate()
        };
        assert_eq!(
            evaluate_version_gate(&constraints, &candidate),
            VersionGateDisposition::Pass
        );
    }

    #[test]
    fn pass_open_ended_valid_until() {
        // valid_until is None → no end constraint → Pass
        let query = make_t(2024, 6, 1);
        let constraints = VersionConstraints {
            query_time: Some(query),
            ..Default::default()
        };
        let candidate = CandidateVersionInfo {
            valid_from: Some(make_t(2024, 1, 1)),
            valid_until: None,
            ..simple_candidate()
        };
        assert_eq!(
            evaluate_version_gate(&constraints, &candidate),
            VersionGateDisposition::Pass
        );
    }

    // ── Deduplication tests ───────────────────────────────────────────────────

    #[test]
    fn dedup_empty_list() {
        let result = dedup_by_semantic_version(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn dedup_no_duplicates() {
        let candidates = vec![
            ("id-a".to_string(), "v1".to_string(), 1u32),
            ("id-b".to_string(), "v1".to_string(), 2u32),
            ("id-c".to_string(), "v1".to_string(), 3u32),
        ];
        let result = dedup_by_semantic_version(&candidates);
        assert_eq!(result.len(), 3);
        // All three should be present; ordered by rank then id
        assert_eq!(result[0], ("id-a".to_string(), "v1".to_string(), 1));
        assert_eq!(result[1], ("id-b".to_string(), "v1".to_string(), 2));
        assert_eq!(result[2], ("id-c".to_string(), "v1".to_string(), 3));
    }

    #[test]
    fn dedup_keeps_lowest_rank() {
        // Same (semantic_id, content_version) at rank 5 and rank 1 → rank 1 kept
        let candidates = vec![
            ("id-a".to_string(), "v1".to_string(), 5u32),
            ("id-a".to_string(), "v1".to_string(), 1u32),
        ];
        let result = dedup_by_semantic_version(&candidates);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], ("id-a".to_string(), "v1".to_string(), 1));
    }

    #[test]
    fn dedup_different_versions_kept_separate() {
        // Same semantic_id but different content_version → both kept
        let candidates = vec![
            ("id-a".to_string(), "v1".to_string(), 2u32),
            ("id-a".to_string(), "v2".to_string(), 3u32),
        ];
        let result = dedup_by_semantic_version(&candidates);
        assert_eq!(result.len(), 2);
        let ids: Vec<_> = result.iter().map(|(_, cv, _)| cv.as_str()).collect();
        assert!(ids.contains(&"v1"));
        assert!(ids.contains(&"v2"));
    }

    #[test]
    fn dedup_output_ordered_by_rank_then_id() {
        // Multiple entries; verify ordering is rank ASC, then semantic_id ASC on ties.
        let candidates = vec![
            ("id-z".to_string(), "v1".to_string(), 2u32),
            ("id-a".to_string(), "v1".to_string(), 2u32),
            ("id-m".to_string(), "v1".to_string(), 1u32),
        ];
        let result = dedup_by_semantic_version(&candidates);
        assert_eq!(result.len(), 3);
        // rank 1 first
        assert_eq!(result[0].0, "id-m");
        assert_eq!(result[0].2, 1);
        // then rank 2 entries, sorted by id ASC
        assert_eq!(result[1].0, "id-a");
        assert_eq!(result[1].2, 2);
        assert_eq!(result[2].0, "id-z");
        assert_eq!(result[2].2, 2);
    }
}
