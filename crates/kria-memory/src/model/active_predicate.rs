//! Centralized active-validity predicate for record/link current reads.
//!
//! # Design invariants (Design §2 A7, MGR-010 AC 1)
//!
//! * **One predicate**: every current-relationship query goes through
//!   [`ActivePredicate::is_active`] (or the SQL counterpart
//!   [`ActivePredicate::sql_fragment`]).  No other module re-implements these
//!   checks.
//! * **Valid Time ⊥ Transaction Time**: the `valid_time_instant` field of
//!   [`ActiveQueryPoint`] is evaluated independently from the WAL
//!   `snapshot_revision`.  Neither can substitute for the other.
//! * **Excluded from default reads** (Design §5.4, MGR-037):
//!   - `Forgotten`  — reversibly excluded during restore window.
//!   - `Deleted`    — always excluded (permanent hard-delete).
//!   - `Superseded` — replaced by a newer version; excluded from current reads.
//! * **Policy filtering is NOT done here** — that is `GraphPolicyFilter`'s job.
//!   This predicate only evaluates truth/lifecycle, valid-time, supersession,
//!   and transaction-revision fields.
//!
//! # Usage
//!
//! ```rust,ignore
//! use crate::model::active_predicate::{
//!     ActivePredicate, ActivePredicateInput, ActiveQueryPoint,
//! };
//! use crate::model::{GraphRevision, UtcTimestamp};
//! use crate::model::truth::TruthState;
//!
//! let input = ActivePredicateInput {
//!     truth_state: TruthState::Current,
//!     valid_from: None,
//!     valid_until: None,
//!     is_superseded: false,
//!     record_revision: GraphRevision::new(3),
//! };
//! let at = ActiveQueryPoint {
//!     snapshot_revision: GraphRevision::new(10),
//!     valid_time_instant: None,          // "now" / current query
//! };
//! assert!(ActivePredicate::is_active(&input, &at));
//! ```

use crate::model::truth::TruthState;
use crate::model::{GraphRevision, UtcTimestamp};

// ── Input / query-point ───────────────────────────────────────────────────

/// Fields extracted from a record/link row that the active predicate needs.
///
/// Design §4.2: `truth_state`, `valid_from`, `valid_until`, and supersession
/// state are present on both `records` and `relationships_v2`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivePredicateInput {
    /// Truth/lifecycle state of the record (maps to the `truth_state` column).
    pub truth_state: TruthState,
    /// Valid-time lower bound (inclusive). `None` = open / all-time from past.
    pub valid_from: Option<UtcTimestamp>,
    /// Valid-time upper bound (exclusive). `None` = ongoing / open upper end.
    pub valid_until: Option<UtcTimestamp>,
    /// Whether `superseded_by IS NOT NULL` in the row.  A record that has been
    /// superseded by a newer one is excluded from default current reads
    /// (MGR-037).
    pub is_superseded: bool,
    /// The graph revision at which this record was created/last committed.
    /// Must be ≤ `snapshot_revision` for the record to be visible in the WAL
    /// snapshot we are reading at.
    pub record_revision: GraphRevision,
}

/// The query "now" point supplied by the caller.
///
/// Design §2 A7: Valid Time and Transaction Time are independent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveQueryPoint {
    /// The WAL-snapshot revision this query executes at.  Only records with
    /// `record_revision ≤ snapshot_revision` are visible.
    pub snapshot_revision: GraphRevision,
    /// The instant at which to evaluate Valid Time.
    ///
    /// * `None`    → current / "now" query: open-ended (`valid_until IS NULL`)
    ///              intervals are active; the predicate uses the wall clock only
    ///              for the `valid_from ≤ now` bound.
    /// * `Some(t)` → historical point query at `t`; evaluated exactly.
    pub valid_time_instant: Option<UtcTimestamp>,
}

// ── Errors ────────────────────────────────────────────────────────────────

/// Errors that arise when validating predicate inputs before evaluation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActivePredicateError {
    /// `snapshot_revision` is `0` (uninitialized/base), which means no
    /// committed revision exists yet; cannot read.
    #[error("snapshot revision is 0 (uninitialized)")]
    UninitializedRevision,
    /// `valid_from > valid_until` — the interval bounds are inverted.
    #[error("valid-time bounds are inverted (valid_from > valid_until)")]
    InvertedValidTime,
}

// ── SQL fragment ──────────────────────────────────────────────────────────

/// A SQL WHERE fragment with positional parameters (`?1`, `?2`, …) for direct
/// use in store-layer queries.
///
/// The store layer is responsible for actually binding and executing SQL; this
/// type only carries the fragment text and its ordered parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct SqlFragment {
    /// The predicate SQL without the `WHERE` keyword.
    pub fragment: String,
    /// Parameter values in the order of their `?N` positional placeholders.
    pub params: Vec<SqlParam>,
}

/// A single SQL parameter value.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlParam {
    Text(String),
    Integer(i64),
    Null,
}

// ── Predicate ─────────────────────────────────────────────────────────────

/// The **one** centralized predicate for "is this record/link active at the
/// given query point?"
///
/// ## Activation rules
///
/// ### Default current query (`valid_time_instant = None`)
///
/// Active when **all** of:
/// 1. `truth_state ∉ {Forgotten, Deleted, Superseded}`
/// 2. `is_superseded == false`
/// 3. `record_revision ≤ snapshot_revision`
/// 4. `valid_from IS NULL  OR  valid_from ≤ now`
/// 5. `valid_until IS NULL  OR  valid_until > now`
///    (open upper bound = ongoing = included)
///
/// ### Historical query (`valid_time_instant = Some(t)`)
///
/// Active when **all** of:
/// 1. `truth_state ≠ Deleted`  (`Deleted` is always excluded)
///    — `Forgotten` and `Superseded` are included for historical reads
///      (the state may not have applied at time `t`; caller verifies restore
///       window for `Forgotten` separately)
/// 2. `is_superseded == false`  (supersession is permanent in this row)
/// 3. `record_revision ≤ snapshot_revision`
/// 4. `valid_from IS NULL  OR  valid_from ≤ t`
/// 5. `valid_until IS NULL  OR  valid_until > t`
///
/// ## Policy
///
/// Policy filtering (namespace / scope / sensitivity / capability) is **not**
/// performed here — that is `GraphPolicyFilter`'s responsibility.
pub struct ActivePredicate;

impl ActivePredicate {
    /// Evaluate whether a record/link row is active at `at`.
    ///
    /// This is the pure-Rust evaluation path; use [`Self::sql_fragment`] to
    /// generate the equivalent SQL WHERE clause for store queries.
    ///
    /// Returns `true` iff the record is active according to the rules above.
    pub fn is_active(input: &ActivePredicateInput, at: &ActiveQueryPoint) -> bool {
        // ── 1. Truth-state check ─────────────────────────────────────────
        let truth_ok = match at.valid_time_instant {
            // Current query: exclude Forgotten, Deleted, Superseded.
            None => !matches!(
                input.truth_state,
                TruthState::Forgotten | TruthState::Deleted | TruthState::Superseded
            ),
            // Historical query: only Deleted is always excluded.
            // Forgotten/Superseded may not have applied at `t`; caller
            // checks the restore window for Forgotten separately.
            Some(_) => !matches!(input.truth_state, TruthState::Deleted),
        };
        if !truth_ok {
            return false;
        }

        // ── 2. Supersession flag ─────────────────────────────────────────
        // A row with `superseded_by IS NOT NULL` is excluded: it has been
        // replaced by a newer record (MGR-037).  This applies to both current
        // and historical queries because the flag reflects the permanent state
        // of the row.
        if input.is_superseded {
            return false;
        }

        // ── 3. Transaction-time / revision visibility ────────────────────
        // Design §2 A7: Transaction Time is the WAL-snapshot revision.
        // Records committed after the snapshot are not yet visible.
        if input.record_revision > at.snapshot_revision {
            return false;
        }

        // ── 4 & 5. Valid-time ────────────────────────────────────────────
        match at.valid_time_instant {
            // Current query: use wall clock "now" for the lower-bound check;
            // open upper bound (None) means ongoing → included.
            None => {
                let now = UtcTimestamp::now();
                let after_start = input.valid_from.map(|from| now >= from).unwrap_or(true);
                let before_end = input.valid_until.map(|until| now < until).unwrap_or(true); // open upper = ongoing = active
                after_start && before_end
            }
            // Historical query at `t`.
            Some(t) => {
                let after_start = input.valid_from.map(|from| t >= from).unwrap_or(true);
                let before_end = input.valid_until.map(|until| t < until).unwrap_or(true);
                after_start && before_end
            }
        }
    }

    /// Generate a SQL WHERE fragment (without `WHERE`) and its ordered
    /// positional parameters for use in store-layer queries.
    ///
    /// ## Current query (`valid_time_instant = None`)
    ///
    /// ```sql
    /// truth_state NOT IN (?1, ?2, ?3)
    ///   AND superseded_by IS NULL
    ///   AND record_revision <= ?4
    ///   AND (valid_from IS NULL OR valid_from <= ?5)
    ///   AND (valid_until IS NULL OR valid_until > ?5)
    /// ```
    /// where `?1='forgotten'`, `?2='deleted'`, `?3='superseded'`,
    /// `?4=snapshot_revision`, `?5=now_utc`.
    ///
    /// ## Historical query (`valid_time_instant = Some(t)`)
    ///
    /// ```sql
    /// truth_state != ?1
    ///   AND superseded_by IS NULL
    ///   AND record_revision <= ?2
    ///   AND (valid_from IS NULL OR valid_from <= ?3)
    ///   AND (valid_until IS NULL OR valid_until > ?3)
    /// ```
    /// where `?1='deleted'`, `?2=snapshot_revision`, `?3=t`.
    ///
    /// Note: this fragment assumes the `superseded_by` column name used in
    /// both `records` and `relationships_v2`.  The store layer should rename
    /// or alias columns as needed.
    pub fn sql_fragment(at: &ActiveQueryPoint) -> SqlFragment {
        let snapshot_rev = at.snapshot_revision.get() as i64;

        match at.valid_time_instant {
            // ── Current query ────────────────────────────────────────────
            None => {
                let now = UtcTimestamp::now().to_rfc3339();
                SqlFragment {
                    fragment: concat!(
                        "truth_state NOT IN (?1, ?2, ?3)",
                        " AND superseded_by IS NULL",
                        " AND record_revision <= ?4",
                        " AND (valid_from IS NULL OR valid_from <= ?5)",
                        " AND (valid_until IS NULL OR valid_until > ?5)"
                    )
                    .to_string(),
                    params: vec![
                        SqlParam::Text(TruthState::Forgotten.as_str().to_string()), // ?1
                        SqlParam::Text(TruthState::Deleted.as_str().to_string()),   // ?2
                        SqlParam::Text(TruthState::Superseded.as_str().to_string()), // ?3
                        SqlParam::Integer(snapshot_rev),                            // ?4
                        SqlParam::Text(now),                                        // ?5
                    ],
                }
            }
            // ── Historical query ─────────────────────────────────────────
            Some(t) => {
                let t_str = t.to_rfc3339();
                SqlFragment {
                    fragment: concat!(
                        "truth_state != ?1",
                        " AND superseded_by IS NULL",
                        " AND record_revision <= ?2",
                        " AND (valid_from IS NULL OR valid_from <= ?3)",
                        " AND (valid_until IS NULL OR valid_until > ?3)"
                    )
                    .to_string(),
                    params: vec![
                        SqlParam::Text(TruthState::Deleted.as_str().to_string()), // ?1
                        SqlParam::Integer(snapshot_rev),                          // ?2
                        SqlParam::Text(t_str),                                    // ?3
                    ],
                }
            }
        }
    }

    /// Validate predicate inputs before evaluation and return a structured
    /// error rather than silently producing a wrong answer.
    ///
    /// Returns `Ok(())` when the inputs are coherent.  Call this before
    /// [`Self::is_active`] when inputs come from untrusted sources (e.g. a
    /// deserialised API request).
    pub fn validate(
        input: &ActivePredicateInput,
        at: &ActiveQueryPoint,
    ) -> Result<(), ActivePredicateError> {
        // Revision 0 (base) means nothing has been committed yet; reading at
        // that snapshot is not meaningful.
        if at.snapshot_revision == GraphRevision::base() {
            return Err(ActivePredicateError::UninitializedRevision);
        }
        // Inverted valid-time bounds are a data-integrity error.
        if let (Some(from), Some(until)) = (input.valid_from, input.valid_until) {
            if from > until {
                return Err(ActivePredicateError::InvertedValidTime);
            }
        }
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Convenience: build a `UtcTimestamp` from a Unix seconds value.
    fn ts(secs: i64) -> UtcTimestamp {
        UtcTimestamp::from_datetime(
            chrono::Utc
                .timestamp_opt(secs, 0)
                .single()
                .expect("valid timestamp"),
        )
    }

    /// A canonical "all-clear" current input: Current, open interval, not
    /// superseded, revision 1 visible at snapshot 10.
    fn current_input() -> ActivePredicateInput {
        ActivePredicateInput {
            truth_state: TruthState::Current,
            valid_from: None,
            valid_until: None,
            is_superseded: false,
            record_revision: GraphRevision::new(1),
        }
    }

    fn current_at(snapshot: u64) -> ActiveQueryPoint {
        ActiveQueryPoint {
            snapshot_revision: GraphRevision::new(snapshot),
            valid_time_instant: None,
        }
    }

    fn historical_at(snapshot: u64, t: UtcTimestamp) -> ActiveQueryPoint {
        ActiveQueryPoint {
            snapshot_revision: GraphRevision::new(snapshot),
            valid_time_instant: Some(t),
        }
    }

    // ── 1. Current state, open valid time → active ────────────────────────

    #[test]
    fn current_state_open_interval_is_active() {
        let input = current_input();
        let at = current_at(10);
        assert!(ActivePredicate::is_active(&input, &at));
    }

    // ── 2. Deleted → never active (current or historical) ─────────────────

    #[test]
    fn deleted_is_never_active_current() {
        let input = ActivePredicateInput {
            truth_state: TruthState::Deleted,
            ..current_input()
        };
        assert!(!ActivePredicate::is_active(&input, &current_at(10)));
    }

    #[test]
    fn deleted_is_never_active_historical() {
        let input = ActivePredicateInput {
            truth_state: TruthState::Deleted,
            ..current_input()
        };
        let at = historical_at(10, ts(1_000_000));
        assert!(!ActivePredicate::is_active(&input, &at));
    }

    // ── 3. Forgotten → inactive in current query, included in historical ───

    #[test]
    fn forgotten_excluded_from_current_query() {
        let input = ActivePredicateInput {
            truth_state: TruthState::Forgotten,
            ..current_input()
        };
        assert!(!ActivePredicate::is_active(&input, &current_at(10)));
    }

    #[test]
    fn forgotten_included_in_historical_query() {
        let input = ActivePredicateInput {
            truth_state: TruthState::Forgotten,
            ..current_input()
        };
        let at = historical_at(10, ts(1_000_000));
        assert!(ActivePredicate::is_active(&input, &at));
    }

    // ── 4. Superseded → inactive in current query, included in historical ──

    #[test]
    fn superseded_state_excluded_from_current_query() {
        let input = ActivePredicateInput {
            truth_state: TruthState::Superseded,
            ..current_input()
        };
        assert!(!ActivePredicate::is_active(&input, &current_at(10)));
    }

    #[test]
    fn superseded_state_included_in_historical_query() {
        let input = ActivePredicateInput {
            truth_state: TruthState::Superseded,
            ..current_input()
        };
        let at = historical_at(10, ts(1_000_000));
        assert!(ActivePredicate::is_active(&input, &at));
    }

    // ── 5. is_superseded=true → inactive always ───────────────────────────

    #[test]
    fn is_superseded_flag_excludes_in_current_query() {
        let input = ActivePredicateInput {
            is_superseded: true,
            ..current_input()
        };
        assert!(!ActivePredicate::is_active(&input, &current_at(10)));
    }

    #[test]
    fn is_superseded_flag_excludes_in_historical_query() {
        let input = ActivePredicateInput {
            is_superseded: true,
            ..current_input()
        };
        let at = historical_at(10, ts(1_000_000));
        assert!(!ActivePredicate::is_active(&input, &at));
    }

    // ── 6. record_revision > snapshot → not yet visible ───────────────────

    #[test]
    fn future_revision_is_invisible() {
        let input = ActivePredicateInput {
            record_revision: GraphRevision::new(11),
            ..current_input()
        };
        assert!(!ActivePredicate::is_active(&input, &current_at(10)));
    }

    #[test]
    fn exact_revision_equal_to_snapshot_is_visible() {
        let input = ActivePredicateInput {
            record_revision: GraphRevision::new(10),
            ..current_input()
        };
        assert!(ActivePredicate::is_active(&input, &current_at(10)));
    }

    // ── 7. Historical: valid_from boundary (exact match at t) ─────────────

    #[test]
    fn historical_valid_from_equal_to_t_is_active() {
        // valid_from = t → inclusive lower bound
        let t = ts(1_000_000);
        let input = ActivePredicateInput {
            valid_from: Some(t),
            valid_until: None,
            ..current_input()
        };
        let at = historical_at(10, t);
        assert!(
            ActivePredicate::is_active(&input, &at),
            "valid_from == t should be included (half-open [from, ∞))"
        );
    }

    #[test]
    fn historical_valid_from_after_t_is_inactive() {
        // valid_from = t+1 → record not yet started at t
        let t = ts(1_000_000);
        let input = ActivePredicateInput {
            valid_from: Some(ts(1_000_001)),
            valid_until: None,
            ..current_input()
        };
        let at = historical_at(10, t);
        assert!(
            !ActivePredicate::is_active(&input, &at),
            "valid_from > t should be excluded"
        );
    }

    // ── 8. Historical: valid_until boundary (exclusive upper bound) ────────

    #[test]
    fn historical_valid_until_equal_to_t_is_inactive() {
        // half-open [from, until) → t == until is outside
        let t = ts(1_000_000);
        let input = ActivePredicateInput {
            valid_from: None,
            valid_until: Some(t),
            ..current_input()
        };
        let at = historical_at(10, t);
        assert!(
            !ActivePredicate::is_active(&input, &at),
            "valid_until == t should be excluded (exclusive upper bound)"
        );
    }

    #[test]
    fn historical_valid_until_after_t_is_active() {
        // valid_until = t+1 → record still valid at t
        let t = ts(1_000_000);
        let input = ActivePredicateInput {
            valid_from: None,
            valid_until: Some(ts(1_000_001)),
            ..current_input()
        };
        let at = historical_at(10, t);
        assert!(
            ActivePredicate::is_active(&input, &at),
            "valid_until > t should be active"
        );
    }

    // ── 9. sql_fragment current: correct excluded states + placeholders ────

    #[test]
    fn sql_fragment_current_contains_three_excluded_states() {
        let at = current_at(42);
        let frag = ActivePredicate::sql_fragment(&at);
        // Must exclude Forgotten, Deleted, Superseded via NOT IN.
        assert!(
            frag.fragment.contains("truth_state NOT IN (?1, ?2, ?3)"),
            "current fragment must use NOT IN for three states"
        );
        // Must filter superseded_by.
        assert!(
            frag.fragment.contains("superseded_by IS NULL"),
            "current fragment must filter superseded_by"
        );
        // Must include snapshot_revision and valid_time placeholders.
        assert!(frag.fragment.contains("?4"), "must have snapshot rev param");
        assert!(frag.fragment.contains("?5"), "must have valid_time param");
        // Params: ?1=forgotten, ?2=deleted, ?3=superseded, ?4=revision, ?5=now
        assert_eq!(frag.params.len(), 5);
        assert_eq!(frag.params[0], SqlParam::Text("forgotten".to_string()));
        assert_eq!(frag.params[1], SqlParam::Text("deleted".to_string()));
        assert_eq!(frag.params[2], SqlParam::Text("superseded".to_string()));
        assert_eq!(frag.params[3], SqlParam::Integer(42));
    }

    // ── 10. sql_fragment historical: only excludes Deleted ────────────────

    #[test]
    fn sql_fragment_historical_only_excludes_deleted() {
        let at = historical_at(7, ts(1_000_000));
        let frag = ActivePredicate::sql_fragment(&at);
        // Uses != not NOT IN (only one excluded truth state).
        assert!(
            frag.fragment.contains("truth_state != ?1"),
            "historical fragment should use != for deleted only, got: {}",
            frag.fragment
        );
        // Must NOT use NOT IN (which would exclude Forgotten/Superseded states).
        assert!(
            !frag.fragment.contains("NOT IN"),
            "historical fragment must not use NOT IN (only Deleted excluded)"
        );
        // Must NOT mention 'forgotten' as a SQL parameter value (it can appear
        // in comments/column names but not as excluded state text in params).
        for p in &frag.params {
            if let SqlParam::Text(s) = p {
                assert_ne!(
                    s.as_str(),
                    "forgotten",
                    "historical fragment must not have 'forgotten' as param"
                );
            }
        }
        // Params: ?1=deleted, ?2=revision, ?3=t (only 3 params, not 5)
        assert_eq!(
            frag.params.len(),
            3,
            "historical fragment should have exactly 3 params"
        );
        assert_eq!(frag.params[0], SqlParam::Text("deleted".to_string()));
        assert_eq!(frag.params[1], SqlParam::Integer(7));
        // ?3 is the RFC3339 timestamp string.
        assert!(matches!(frag.params[2], SqlParam::Text(_)));
    }

    // ── 11. validate: uninitialized revision ─────────────────────────────

    #[test]
    fn validate_rejects_snapshot_revision_zero() {
        let input = current_input();
        let at = current_at(0); // revision 0 = uninitialized
        let err = ActivePredicate::validate(&input, &at).unwrap_err();
        assert_eq!(err, ActivePredicateError::UninitializedRevision);
    }

    // ── 12. validate: inverted valid time ────────────────────────────────

    #[test]
    fn validate_rejects_inverted_valid_interval() {
        let input = ActivePredicateInput {
            valid_from: Some(ts(2_000_000)),
            valid_until: Some(ts(1_000_000)), // from > until
            ..current_input()
        };
        let at = current_at(10);
        let err = ActivePredicate::validate(&input, &at).unwrap_err();
        assert_eq!(err, ActivePredicateError::InvertedValidTime);
    }

    // ── 13. Additional truth states are visible in current query ──────────

    #[test]
    fn other_truth_states_are_active_in_current_query() {
        let active_states = [
            TruthState::Current,
            TruthState::Unverified,
            TruthState::Stale,
            TruthState::Contradicted,
            TruthState::Inferred,
            TruthState::Confirmed,
            TruthState::Unavailable,
        ];
        for state in active_states {
            let input = ActivePredicateInput {
                truth_state: state.clone(),
                ..current_input()
            };
            assert!(
                ActivePredicate::is_active(&input, &current_at(10)),
                "{state:?} should be visible in default current reads"
            );
        }
    }

    // ── 14. sql_fragment current: superseded_by and valid_time check ──────

    #[test]
    fn sql_fragment_current_has_valid_time_bounds() {
        let at = current_at(5);
        let frag = ActivePredicate::sql_fragment(&at);
        assert!(
            frag.fragment
                .contains("valid_from IS NULL OR valid_from <="),
            "must check lower bound"
        );
        assert!(
            frag.fragment
                .contains("valid_until IS NULL OR valid_until >"),
            "must check upper bound"
        );
    }

    // ── 15. sql_fragment historical: valid_time bounds at t ───────────────

    #[test]
    fn sql_fragment_historical_has_valid_time_bounds() {
        let at = historical_at(5, ts(1_000_000));
        let frag = ActivePredicate::sql_fragment(&at);
        assert!(
            frag.fragment
                .contains("valid_from IS NULL OR valid_from <="),
            "historical fragment must check lower bound"
        );
        assert!(
            frag.fragment
                .contains("valid_until IS NULL OR valid_until >"),
            "historical fragment must check upper bound"
        );
    }
}
