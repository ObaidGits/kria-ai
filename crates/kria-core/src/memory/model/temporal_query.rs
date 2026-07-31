//! Historical instant/range evaluation independent of Transaction Time.
//!
//! # Design invariants (Design §2 A7, MGR-010 AC 2/3/6)
//!
//! * **Valid Time ⊥ Transaction Time**: [`TemporalQuerySpec::snapshot_revision`]
//!   carries the Transaction-Time snapshot for reproducibility metadata only —
//!   it does NOT filter valid-time results.
//! * **UTC is authoritative**: `source_tz_offset_min` and `source_local_repr`
//!   on [`TemporalInstant`] are display/provenance metadata only; all
//!   comparisons use `.utc`.
//! * **Half-open intervals**: point-query lower bound is inclusive
//!   (`valid_from <= t`); upper bound is exclusive (`valid_until > t`).
//! * **Range intersection**: a record `[a, b)` intersects range `[from, until)`
//!   iff `a < until` AND `b > from` (NULL = ∞).
//! * **Recency**: no valid-time filter; ordering by transaction time is the
//!   caller's responsibility.
//! * **Result metadata** (MGR-010 AC 3): every response includes the requested
//!   instant/range, Graph_Revision, validity intervals, source times, and
//!   timezone metadata via [`TemporalResultMetadata`].

use crate::memory::model::active_predicate::{SqlFragment, SqlParam};
use crate::memory::model::{GraphRevision, UtcTimestamp};

// ── TemporalInstant ───────────────────────────────────────────────────────

/// A precise instant for historical evaluation.
///
/// The `.utc` field is the authoritative value for all comparisons.
/// `source_tz_offset_min` and `source_local_repr` are display/provenance
/// metadata ONLY and MUST NOT be used for evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporalInstant {
    /// The instant in UTC — authoritative for all comparisons.
    pub utc: UtcTimestamp,
    /// Timezone offset in minutes from UTC at the source (e.g. `330` for IST
    /// +05:30, `-300` for EST).  Used for display only; evaluation uses UTC.
    pub source_tz_offset_min: Option<i16>,
    /// Original local time string from the source (provenance display).
    /// MUST NOT be used for evaluation — UTC is authoritative.
    pub source_local_repr: Option<String>,
}

impl TemporalInstant {
    /// Construct from a UTC timestamp with no source timezone metadata.
    pub fn from_utc(utc: UtcTimestamp) -> Self {
        TemporalInstant {
            utc,
            source_tz_offset_min: None,
            source_local_repr: None,
        }
    }

    /// Construct with source timezone offset for display purposes.
    pub fn with_source_tz(utc: UtcTimestamp, source_tz_offset_min: i16) -> Self {
        TemporalInstant {
            utc,
            source_tz_offset_min: Some(source_tz_offset_min),
            source_local_repr: None,
        }
    }
}

// ── TemporalRange ─────────────────────────────────────────────────────────

/// A closed-open interval `[from, until)` or open/half-open interval for
/// range queries.
///
/// `None` lower bound = no lower bound (open from the past).
/// `None` upper bound = no upper bound (open-ended / ongoing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporalRange {
    /// Inclusive lower bound. `None` = no lower bound.
    pub from: Option<TemporalInstant>,
    /// Exclusive upper bound. `None` = no upper bound (open-ended / current).
    pub until: Option<TemporalInstant>,
}

impl TemporalRange {
    /// A fully open range (matches everything).
    pub fn open() -> Self {
        TemporalRange {
            from: None,
            until: None,
        }
    }

    /// A range with only a lower bound (from `from` to now/open).
    pub fn from_instant(from: TemporalInstant) -> Self {
        TemporalRange {
            from: Some(from),
            until: None,
        }
    }

    /// A range with only an upper bound (from the beginning until `until`).
    pub fn until_instant(until: TemporalInstant) -> Self {
        TemporalRange {
            from: None,
            until: Some(until),
        }
    }

    /// A bounded range `[from, until)`.
    pub fn bounded(from: TemporalInstant, until: TemporalInstant) -> Self {
        TemporalRange {
            from: Some(from),
            until: Some(until),
        }
    }
}

// ── HistoricalQueryKind ───────────────────────────────────────────────────

/// The kind of historical (or recency) query to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoricalQueryKind {
    /// Point-in-time: what was valid at exactly this instant?
    Instant(TemporalInstant),
    /// Range: what was valid during any part of this interval?
    Range(TemporalRange),
    /// Recency: what is recent, sorted by transaction time descending?
    /// No valid-time filter is applied; ordering is the caller's responsibility.
    Recency { limit: u32 },
}

// ── TemporalQuerySpec ─────────────────────────────────────────────────────

/// The complete temporal query specification, independent of transaction
/// revision (MGR-010 AC 2).
///
/// `snapshot_revision` (Transaction Time) is carried for reproducibility
/// metadata and is NOT used to filter valid-time results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporalQuerySpec {
    /// The kind of temporal query.
    pub kind: HistoricalQueryKind,
    /// The graph revision this query is evaluated against (Transaction Time
    /// snapshot).  Independent of Valid Time — it is metadata only.
    pub snapshot_revision: GraphRevision,
    /// Caller-declared timezone for display metadata.  `None` = UTC.
    pub display_tz_offset_min: Option<i16>,
}

impl TemporalQuerySpec {
    /// Construct an instant point-in-time query.
    pub fn instant(instant: TemporalInstant, snapshot_revision: GraphRevision) -> Self {
        TemporalQuerySpec {
            kind: HistoricalQueryKind::Instant(instant),
            snapshot_revision,
            display_tz_offset_min: None,
        }
    }

    /// Construct a range query.
    pub fn range(range: TemporalRange, snapshot_revision: GraphRevision) -> Self {
        TemporalQuerySpec {
            kind: HistoricalQueryKind::Range(range),
            snapshot_revision,
            display_tz_offset_min: None,
        }
    }

    /// Construct a recency query.
    pub fn recency(limit: u32, snapshot_revision: GraphRevision) -> Self {
        TemporalQuerySpec {
            kind: HistoricalQueryKind::Recency { limit },
            snapshot_revision,
            display_tz_offset_min: None,
        }
    }
}

// ── EvaluatedUtcRange ─────────────────────────────────────────────────────

/// The UTC bounds that were actually evaluated, derived from the query spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluatedUtcRange {
    /// Inclusive lower bound in UTC.  `None` = open.
    pub from: Option<UtcTimestamp>,
    /// Exclusive upper bound in UTC.  `None` = open.
    pub until: Option<UtcTimestamp>,
}

impl EvaluatedUtcRange {
    /// Derive the evaluated range from a query spec.
    pub fn from_spec(spec: &TemporalQuerySpec) -> Self {
        match &spec.kind {
            HistoricalQueryKind::Instant(t) => EvaluatedUtcRange {
                from: Some(t.utc),
                until: Some(t.utc),
            },
            HistoricalQueryKind::Range(r) => EvaluatedUtcRange {
                from: r.from.as_ref().map(|i| i.utc),
                until: r.until.as_ref().map(|i| i.utc),
            },
            HistoricalQueryKind::Recency { .. } => EvaluatedUtcRange {
                from: None,
                until: None,
            },
        }
    }
}

// ── TemporalResultMetadata ────────────────────────────────────────────────

/// Metadata included with every temporal query result (MGR-010 AC 3).
///
/// Returned alongside result items so callers can render the precise query
/// context and verify what was requested vs what was evaluated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporalResultMetadata {
    /// The query spec that was used.
    pub query: TemporalQuerySpec,
    /// The UTC instant or range that was actually evaluated.
    pub evaluated_utc: EvaluatedUtcRange,
    /// Display timezone offset used for rendering (from
    /// `display_tz_offset_min` in the query spec, defaulting to `0` = UTC).
    pub display_tz_offset_min: i16,
}

impl TemporalResultMetadata {
    /// Construct from a query spec.
    pub fn from_spec(spec: TemporalQuerySpec) -> Self {
        let evaluated_utc = EvaluatedUtcRange::from_spec(&spec);
        let display_tz = spec.display_tz_offset_min.unwrap_or(0);
        TemporalResultMetadata {
            query: spec,
            evaluated_utc,
            display_tz_offset_min: display_tz,
        }
    }
}

// ── TemporalEvaluator ─────────────────────────────────────────────────────

/// Stateless evaluator for temporal query intersection and SQL fragment
/// generation (MGR-010 AC 2/3/6).
///
/// All comparisons use UTC.  `source_tz_offset_min` in [`TemporalInstant`]
/// is for display only.
pub struct TemporalEvaluator;

impl TemporalEvaluator {
    /// Evaluate whether a record's valid interval intersects the query.
    ///
    /// ## Rules
    ///
    /// **Point query** `Instant(t)`: the record is active at `t` iff
    /// ```text
    /// (valid_from IS NULL OR valid_from <= t) AND (valid_until IS NULL OR valid_until > t)
    /// ```
    ///
    /// **Range query** `Range([from, until))`: the record intersects iff
    /// ```text
    /// (record_valid_from < range_until OR range_until IS NULL)
    ///   AND (record_valid_until > range_from OR record_valid_until IS NULL OR range_from IS NULL)
    /// ```
    ///
    /// **Recency**: always returns `true` (ranking is handled externally).
    ///
    /// All comparisons use UTC.
    pub fn record_intersects_query(
        record_valid_from: Option<UtcTimestamp>,
        record_valid_until: Option<UtcTimestamp>,
        query: &TemporalQuerySpec,
    ) -> bool {
        match &query.kind {
            // ── Point query ──────────────────────────────────────────
            HistoricalQueryKind::Instant(t) => {
                let after_start = record_valid_from.map(|from| t.utc >= from).unwrap_or(true);
                let before_end = record_valid_until
                    .map(|until| t.utc < until)
                    .unwrap_or(true);
                after_start && before_end
            }

            // ── Range query [from, until) ─────────────────────────────
            HistoricalQueryKind::Range(range) => {
                let range_from: Option<UtcTimestamp> = range.from.as_ref().map(|i| i.utc);
                let range_until: Option<UtcTimestamp> = range.until.as_ref().map(|i| i.utc);

                // record_valid_from < range_until  (OR range_until IS NULL)
                let starts_before_range_end = match (record_valid_from, range_until) {
                    (_, None) => true, // open upper range — always ok
                    (None, Some(ru)) => {
                        // record has no lower bound — it started at -∞ < range_until
                        let _ = ru;
                        true
                    }
                    (Some(rvf), Some(ru)) => rvf < ru,
                };

                // record_valid_until > range_from  (OR record_valid_until IS NULL OR range_from IS NULL)
                let ends_after_range_start = match (record_valid_until, range_from) {
                    (None, _) => true, // open-ended record — still going
                    (_, None) => true, // no lower range bound
                    (Some(rvu), Some(rf)) => rvu > rf,
                };

                starts_before_range_end && ends_after_range_start
            }

            // ── Recency ──────────────────────────────────────────────
            HistoricalQueryKind::Recency { .. } => true,
        }
    }

    /// Generate a SQL WHERE fragment for temporal range/instant/recency queries.
    ///
    /// **Instant(t)**:
    /// ```sql
    /// (valid_from IS NULL OR valid_from <= ?1) AND (valid_until IS NULL OR valid_until > ?1)
    /// ```
    /// where `?1 = t` (RFC 3339 UTC string).
    ///
    /// **Range([from, until))**:
    /// ```sql
    /// (valid_from < ?1 OR ?1 IS NULL) AND (valid_until > ?2 OR valid_until IS NULL OR ?2 IS NULL)
    /// ```
    /// where `?1 = range_until`, `?2 = range_from`.
    ///
    /// **Recency**: `"1=1"` (no time filter; ORDER BY handled by caller).
    pub fn sql_fragment(query: &TemporalQuerySpec) -> SqlFragment {
        match &query.kind {
            // ── Instant ──────────────────────────────────────────────
            HistoricalQueryKind::Instant(t) => {
                let t_str = t.utc.to_rfc3339();
                SqlFragment {
                    fragment: concat!(
                        "(valid_from IS NULL OR valid_from <= ?1)",
                        " AND (valid_until IS NULL OR valid_until > ?1)"
                    )
                    .to_string(),
                    params: vec![SqlParam::Text(t_str)],
                }
            }

            // ── Range ─────────────────────────────────────────────────
            HistoricalQueryKind::Range(range) => {
                let p1 = match &range.until {
                    Some(i) => SqlParam::Text(i.utc.to_rfc3339()),
                    None => SqlParam::Null,
                };
                let p2 = match &range.from {
                    Some(i) => SqlParam::Text(i.utc.to_rfc3339()),
                    None => SqlParam::Null,
                };
                SqlFragment {
                    fragment: concat!(
                        "(valid_from < ?1 OR ?1 IS NULL)",
                        " AND (valid_until > ?2 OR valid_until IS NULL OR ?2 IS NULL)"
                    )
                    .to_string(),
                    params: vec![p1, p2],
                }
            }

            // ── Recency ──────────────────────────────────────────────
            HistoricalQueryKind::Recency { .. } => SqlFragment {
                fragment: "1=1".to_string(),
                params: vec![],
            },
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Build a `UtcTimestamp` from Unix seconds.
    fn ts(secs: i64) -> UtcTimestamp {
        UtcTimestamp::from_datetime(
            chrono::Utc
                .timestamp_opt(secs, 0)
                .single()
                .expect("valid timestamp"),
        )
    }

    fn instant(secs: i64) -> TemporalInstant {
        TemporalInstant::from_utc(ts(secs))
    }

    fn rev(n: u64) -> GraphRevision {
        GraphRevision::new(n)
    }

    // ── record_intersects_query: point queries ─────────────────────────

    #[test]
    fn point_query_record_starting_at_t_is_included() {
        // valid_from = t → inclusive lower bound
        let t = ts(1_000_000);
        let spec = TemporalQuerySpec::instant(TemporalInstant::from_utc(t), rev(10));
        let result = TemporalEvaluator::record_intersects_query(
            Some(t), // record starts exactly at t
            None,    // open upper bound
            &spec,
        );
        assert!(
            result,
            "record with valid_from == t must be included (inclusive lower bound)"
        );
    }

    #[test]
    fn point_query_record_ending_at_t_is_excluded() {
        // valid_until = t → exclusive upper bound
        let t = ts(1_000_000);
        let spec = TemporalQuerySpec::instant(TemporalInstant::from_utc(t), rev(10));
        let result = TemporalEvaluator::record_intersects_query(
            None,    // open lower bound
            Some(t), // record ends exactly at t
            &spec,
        );
        assert!(
            !result,
            "record with valid_until == t must be excluded (exclusive upper bound)"
        );
    }

    #[test]
    fn point_query_open_interval_always_included() {
        let t = ts(1_000_000);
        let spec = TemporalQuerySpec::instant(TemporalInstant::from_utc(t), rev(10));
        let result = TemporalEvaluator::record_intersects_query(None, None, &spec);
        assert!(result, "open interval (None/None) must always be included");
    }

    #[test]
    fn point_query_record_starting_before_t_and_ending_after_t_included() {
        let t = ts(1_000_000);
        let spec = TemporalQuerySpec::instant(TemporalInstant::from_utc(t), rev(10));
        let result = TemporalEvaluator::record_intersects_query(
            Some(ts(999_999)),   // starts before t
            Some(ts(1_000_001)), // ends after t
            &spec,
        );
        assert!(result, "record that spans t must be included");
    }

    #[test]
    fn point_query_record_after_t_is_excluded() {
        let t = ts(1_000_000);
        let spec = TemporalQuerySpec::instant(TemporalInstant::from_utc(t), rev(10));
        let result = TemporalEvaluator::record_intersects_query(
            Some(ts(1_000_001)), // starts after t
            None,
            &spec,
        );
        assert!(!result, "record that starts after t must be excluded");
    }

    #[test]
    fn point_query_record_before_t_is_excluded() {
        let t = ts(1_000_000);
        let spec = TemporalQuerySpec::instant(TemporalInstant::from_utc(t), rev(10));
        let result = TemporalEvaluator::record_intersects_query(
            None,
            Some(ts(999_999)), // ended before t
            &spec,
        );
        assert!(!result, "record that ended before t must be excluded");
    }

    // ── record_intersects_query: range queries ─────────────────────────

    #[test]
    fn range_query_overlapping_intervals_included() {
        // Range [100, 200), record [150, 250) → overlap
        let spec =
            TemporalQuerySpec::range(TemporalRange::bounded(instant(100), instant(200)), rev(10));
        let result =
            TemporalEvaluator::record_intersects_query(Some(ts(150)), Some(ts(250)), &spec);
        assert!(result, "overlapping intervals must be included");
    }

    #[test]
    fn range_query_record_entirely_before_range_excluded() {
        // Range [100, 200), record [10, 50) → no overlap
        let spec =
            TemporalQuerySpec::range(TemporalRange::bounded(instant(100), instant(200)), rev(10));
        let result = TemporalEvaluator::record_intersects_query(Some(ts(10)), Some(ts(50)), &spec);
        assert!(!result, "record ending before range must be excluded");
    }

    #[test]
    fn range_query_record_entirely_after_range_excluded() {
        // Range [100, 200), record [300, 400) → no overlap
        let spec =
            TemporalQuerySpec::range(TemporalRange::bounded(instant(100), instant(200)), rev(10));
        let result =
            TemporalEvaluator::record_intersects_query(Some(ts(300)), Some(ts(400)), &spec);
        assert!(!result, "record starting after range must be excluded");
    }

    #[test]
    fn range_query_open_lower_bound_includes_all_before_until() {
        // Range [None, 200), record [10, 150) → overlap (record ends before range_until)
        let spec = TemporalQuerySpec::range(TemporalRange::until_instant(instant(200)), rev(10));
        let result = TemporalEvaluator::record_intersects_query(Some(ts(10)), Some(ts(150)), &spec);
        assert!(
            result,
            "open lower bound range must include records before until"
        );
    }

    #[test]
    fn range_query_open_upper_bound_includes_all_after_from() {
        // Range [100, None), record [200, 300) → overlap
        let spec = TemporalQuerySpec::range(TemporalRange::from_instant(instant(100)), rev(10));
        let result =
            TemporalEvaluator::record_intersects_query(Some(ts(200)), Some(ts(300)), &spec);
        assert!(
            result,
            "open upper bound range must include records after from"
        );
    }

    #[test]
    fn range_query_fully_open_includes_all() {
        let spec = TemporalQuerySpec::range(TemporalRange::open(), rev(10));
        let result =
            TemporalEvaluator::record_intersects_query(Some(ts(500)), Some(ts(1000)), &spec);
        assert!(result, "fully open range must include every record");
    }

    #[test]
    fn range_query_open_record_included_in_open_range() {
        let spec =
            TemporalQuerySpec::range(TemporalRange::bounded(instant(100), instant(200)), rev(10));
        // Record is open on both ends — spans everything
        let result = TemporalEvaluator::record_intersects_query(None, None, &spec);
        assert!(result, "fully open record intersects any range");
    }

    // ── record_intersects_query: recency ───────────────────────────────

    #[test]
    fn recency_always_returns_true_regardless_of_interval() {
        let spec = TemporalQuerySpec::recency(10, rev(5));

        // Bounded record
        let r1 = TemporalEvaluator::record_intersects_query(Some(ts(0)), Some(ts(1)), &spec);
        // Open record
        let r2 = TemporalEvaluator::record_intersects_query(None, None, &spec);
        // Future record
        let r3 = TemporalEvaluator::record_intersects_query(Some(ts(999_999_999)), None, &spec);

        assert!(r1 && r2 && r3, "recency query must always return true");
    }

    // ── sql_fragment ───────────────────────────────────────────────────

    #[test]
    fn sql_fragment_instant_has_correct_structure() {
        let t = ts(1_000_000);
        let spec = TemporalQuerySpec::instant(TemporalInstant::from_utc(t), rev(7));
        let frag = TemporalEvaluator::sql_fragment(&spec);

        assert!(
            frag.fragment
                .contains("valid_from IS NULL OR valid_from <= ?1"),
            "instant fragment must have lower-bound clause, got: {}",
            frag.fragment
        );
        assert!(
            frag.fragment
                .contains("valid_until IS NULL OR valid_until > ?1"),
            "instant fragment must have upper-bound clause, got: {}",
            frag.fragment
        );
        // Exactly one param: t
        assert_eq!(frag.params.len(), 1);
        assert!(matches!(frag.params[0], SqlParam::Text(_)));
    }

    #[test]
    fn sql_fragment_range_bounded_has_two_params() {
        let spec =
            TemporalQuerySpec::range(TemporalRange::bounded(instant(100), instant(200)), rev(5));
        let frag = TemporalEvaluator::sql_fragment(&spec);

        assert!(
            frag.fragment.contains("valid_from < ?1 OR ?1 IS NULL"),
            "range fragment must have upper-bound clause (record_from < range_until)"
        );
        assert!(
            frag.fragment
                .contains("valid_until > ?2 OR valid_until IS NULL OR ?2 IS NULL"),
            "range fragment must have lower-bound clause"
        );
        assert_eq!(frag.params.len(), 2);
        // ?1 = range_until, ?2 = range_from
        assert!(
            matches!(frag.params[0], SqlParam::Text(_)),
            "?1 is range_until text"
        );
        assert!(
            matches!(frag.params[1], SqlParam::Text(_)),
            "?2 is range_from text"
        );
    }

    #[test]
    fn sql_fragment_range_open_upper_has_null_p1() {
        // Range [100, None) → p1 = NULL
        let spec = TemporalQuerySpec::range(TemporalRange::from_instant(instant(100)), rev(5));
        let frag = TemporalEvaluator::sql_fragment(&spec);
        assert_eq!(frag.params.len(), 2);
        assert_eq!(
            frag.params[0],
            SqlParam::Null,
            "open upper → ?1 must be NULL"
        );
        assert!(
            matches!(frag.params[1], SqlParam::Text(_)),
            "?2 is range_from text"
        );
    }

    #[test]
    fn sql_fragment_range_open_lower_has_null_p2() {
        // Range [None, 200) → p2 = NULL
        let spec = TemporalQuerySpec::range(TemporalRange::until_instant(instant(200)), rev(5));
        let frag = TemporalEvaluator::sql_fragment(&spec);
        assert_eq!(frag.params.len(), 2);
        assert!(
            matches!(frag.params[0], SqlParam::Text(_)),
            "?1 is range_until text"
        );
        assert_eq!(
            frag.params[1],
            SqlParam::Null,
            "open lower → ?2 must be NULL"
        );
    }

    #[test]
    fn sql_fragment_recency_is_tautology() {
        let spec = TemporalQuerySpec::recency(20, rev(3));
        let frag = TemporalEvaluator::sql_fragment(&spec);
        assert_eq!(frag.fragment, "1=1", "recency fragment must be '1=1'");
        assert!(frag.params.is_empty(), "recency has no params");
    }

    // ── TemporalResultMetadata ─────────────────────────────────────────

    #[test]
    fn temporal_result_metadata_preserves_query_spec() {
        let t = ts(1_000_000);
        let spec = TemporalQuerySpec {
            kind: HistoricalQueryKind::Instant(TemporalInstant::from_utc(t)),
            snapshot_revision: rev(42),
            display_tz_offset_min: Some(330),
        };
        let meta = TemporalResultMetadata::from_spec(spec.clone());

        assert_eq!(
            meta.query, spec,
            "metadata must preserve the original query spec"
        );
        assert_eq!(
            meta.display_tz_offset_min, 330,
            "display tz must be taken from spec"
        );
        assert_eq!(meta.evaluated_utc.from, Some(t));
        assert_eq!(meta.evaluated_utc.until, Some(t));
    }

    #[test]
    fn temporal_result_metadata_defaults_tz_to_zero() {
        let spec = TemporalQuerySpec::recency(5, rev(1));
        let meta = TemporalResultMetadata::from_spec(spec);
        assert_eq!(
            meta.display_tz_offset_min, 0,
            "missing tz must default to 0 (UTC)"
        );
    }

    #[test]
    fn temporal_result_metadata_range_evaluated_utc() {
        let spec =
            TemporalQuerySpec::range(TemporalRange::bounded(instant(100), instant(200)), rev(5));
        let meta = TemporalResultMetadata::from_spec(spec);
        assert_eq!(meta.evaluated_utc.from, Some(ts(100)));
        assert_eq!(meta.evaluated_utc.until, Some(ts(200)));
    }

    #[test]
    fn temporal_result_metadata_recency_open_range() {
        let spec = TemporalQuerySpec::recency(10, rev(3));
        let meta = TemporalResultMetadata::from_spec(spec);
        assert_eq!(meta.evaluated_utc.from, None);
        assert_eq!(meta.evaluated_utc.until, None);
    }

    // ── snapshot_revision is independent (display/metadata only) ──────

    #[test]
    fn snapshot_revision_does_not_affect_record_intersection() {
        // Same query, different revision — intersection result must be the same
        let t = ts(1_000_000);
        let spec_r5 = TemporalQuerySpec::instant(TemporalInstant::from_utc(t), rev(5));
        let spec_r99 = TemporalQuerySpec::instant(TemporalInstant::from_utc(t), rev(99));

        let r1 = TemporalEvaluator::record_intersects_query(Some(t), None, &spec_r5);
        let r2 = TemporalEvaluator::record_intersects_query(Some(t), None, &spec_r99);
        assert_eq!(
            r1, r2,
            "snapshot_revision must not affect valid-time evaluation"
        );
    }

    // ── source_tz_offset_min is display only ───────────────────────────

    #[test]
    fn source_tz_offset_does_not_affect_evaluation() {
        // Two instants at the same UTC point but different source tz metadata
        let t = ts(1_000_000);
        let i_utc = TemporalInstant::from_utc(t);
        let i_ist = TemporalInstant::with_source_tz(t, 330);

        let spec_utc = TemporalQuerySpec::instant(i_utc, rev(1));
        let spec_ist = TemporalQuerySpec::instant(i_ist, rev(1));

        let r_utc = TemporalEvaluator::record_intersects_query(Some(t), None, &spec_utc);
        let r_ist = TemporalEvaluator::record_intersects_query(Some(t), None, &spec_ist);
        assert_eq!(r_utc, r_ist, "source_tz_offset must not affect evaluation");
    }
}
