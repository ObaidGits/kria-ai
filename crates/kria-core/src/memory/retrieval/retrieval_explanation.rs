//! Retrieval explanation model (design §4.3/§6.4, task F3.5.7).
//!
//! Defines the five explanation categories and enforces the invariant that
//! only injected trace membership can produce a "Used" label.
//!
//! # Five explanation categories
//! 1. **WhyStored** — record provenance (not retrieval-specific)
//! 2. **WhyRecalled** — why this record appeared as a retrieval candidate
//! 3. **HowUsed** — whether and how the record was used in context (if at all)
//! 4. **RetrievedFiltered** — why a candidate was filtered before injection
//! 5. **AvailableSafe** — record exists but was not retrieved for this query
//!
//! # Key invariant
//! `is_used = true` ONLY when a `retrieval_trace_items` row exists with
//! `gate_disposition = "included"`, `injected_order IS NOT NULL`, and its
//! parent trace has `status = "finalized"`. No other path may produce the
//! "Used" label (design §6.4, MGR-001 AC5).

use crate::memory::error::{MemoryResult, StorageError};

// ── Types ─────────────────────────────────────────────────────────────────────

/// The five retrieval explanation categories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplanationKind {
    /// Record provenance — how/why it was created in authority (not retrieval-specific).
    WhyStored,
    /// Why this record appeared as a candidate in this retrieval (strategy, score).
    WhyRecalled,
    /// What happened after retrieval: was it injected? what was the impact?
    ///
    /// This is the ONLY kind that sets `is_used = true`.
    HowUsed,
    /// Why a candidate was filtered out before context injection (gate reason).
    RetrievedFiltered,
    /// Record exists but was never retrieved for this query (no trace item).
    AvailableSafe,
}

/// The explanation for one record's relationship to a retrieval call.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordExplanation {
    pub record_id: String,
    pub kind: ExplanationKind,
    /// Optional detail string (policy-safe; no hidden IDs).
    pub detail: String,
    /// Whether this record received the "Used" label.
    ///
    /// INVARIANT: `is_used = true` ONLY when:
    /// * A trace item exists for this record
    /// * `gate_disposition = "included"`
    /// * `injected_order IS NOT NULL`
    /// * The parent trace has `status = "finalized"`
    pub is_used: bool,
}

/// Evidence that a record was used — links to the trace item that proves it.
#[derive(Debug, Clone, PartialEq)]
pub struct UsedEvidence {
    pub trace_id: String,
    pub record_id: String,
    pub injected_order: i64,
    pub response_id: Option<String>,
}

/// A view of a trace item sufficient for explanation classification.
#[derive(Debug, Clone)]
pub struct TraceItemView {
    pub trace_id: String,
    pub record_id: String,
    pub gate_disposition: Option<String>,
    pub injected_order: Option<i64>,
    pub strategy: String,
    pub strategy_rank: Option<i64>,
    pub strategy_score: Option<f64>,
    pub rrf_contribution: Option<f64>,
}

// ── Classification logic ──────────────────────────────────────────────────────

/// Derive the explanation kind for a record given its trace item (if any).
///
/// # Rules
/// - No trace item at all → `AvailableSafe`
/// - `gate_disposition = "unauthorized"` → `AvailableSafe` (opaque; don't reveal existence)
/// - `gate_disposition = "excluded"` or `"filtered"` → `RetrievedFiltered`
/// - `gate_disposition = "included"` AND `injected_order IS NOT NULL` AND `trace_finalized` → `HowUsed`
/// - `gate_disposition = "included"` AND (`injected_order IS NULL` OR `!trace_finalized`) → `WhyRecalled`
/// - Any other/None disposition → `WhyRecalled`
pub fn classify_explanation(
    trace_item: Option<&TraceItemView>,
    trace_finalized: bool,
) -> ExplanationKind {
    let item = match trace_item {
        None => return ExplanationKind::AvailableSafe,
        Some(i) => i,
    };

    match item.gate_disposition.as_deref() {
        Some("unauthorized") => ExplanationKind::AvailableSafe,
        Some("excluded") | Some("filtered") => ExplanationKind::RetrievedFiltered,
        Some("included") => {
            if item.injected_order.is_some() && trace_finalized {
                ExplanationKind::HowUsed
            } else {
                ExplanationKind::WhyRecalled
            }
        }
        // Any other or None disposition: record was retrieved but outcome unclear.
        _ => ExplanationKind::WhyRecalled,
    }
}

/// Build a `RecordExplanation` from a trace item view and finalization status.
///
/// - Calls `classify_explanation` to derive `kind`.
/// - `is_used` is `true` only when `kind == HowUsed`.
/// - `detail` is a human-readable policy-safe summary of the reason.
pub fn build_explanation(
    record_id: &str,
    trace_item: Option<&TraceItemView>,
    trace_finalized: bool,
) -> RecordExplanation {
    let kind = classify_explanation(trace_item, trace_finalized);
    let is_used = kind == ExplanationKind::HowUsed;

    let detail = match (&kind, trace_item) {
        (ExplanationKind::AvailableSafe, None) => {
            "Record exists in the memory authority but was not retrieved for this query."
                .to_string()
        }
        (ExplanationKind::AvailableSafe, Some(_)) => {
            // Unauthorized — opaque message, reveal no retrieval details.
            "Record is not available for retrieval in this context.".to_string()
        }
        (ExplanationKind::RetrievedFiltered, Some(item)) => {
            let disposition = item.gate_disposition.as_deref().unwrap_or("filtered");
            format!(
                "Record was retrieved via '{}' strategy (rank {:?}) but was {} before context injection.",
                item.strategy,
                item.strategy_rank,
                disposition,
            )
        }
        (ExplanationKind::HowUsed, Some(item)) => {
            format!(
                "Record was retrieved via '{}' strategy (rank {:?}, score {:.4}) and injected at position {}.",
                item.strategy,
                item.strategy_rank,
                item.strategy_score.unwrap_or(0.0),
                item.injected_order
                    .map(|o| o.to_string())
                    .unwrap_or_else(|| "?".to_string()),
            )
        }
        (ExplanationKind::WhyRecalled, Some(item)) => {
            format!(
                "Record was retrieved via '{}' strategy (rank {:?}, score {:.4}) but was not injected into context.",
                item.strategy,
                item.strategy_rank,
                item.strategy_score.unwrap_or(0.0),
            )
        }
        (ExplanationKind::WhyStored, _) => {
            "Record provenance: stored in the memory authority.".to_string()
        }
        // Fallback for mismatched arms (unreachable in practice).
        _ => "No retrieval detail available.".to_string(),
    };

    RecordExplanation {
        record_id: record_id.to_string(),
        kind,
        detail,
        is_used,
    }
}

// ── Database lookup ───────────────────────────────────────────────────────────

/// Retrieve the `UsedEvidence` for a record from the database.
///
/// Returns `None` if the record was not used (no finalized trace with
/// `gate_disposition = "included"` and `injected_order IS NOT NULL`).
///
/// This is the authoritative "Used" proof — no other path may produce this.
///
/// # SQL
/// ```sql
/// SELECT t.id, i.record_id, i.injected_order, t.response_id
/// FROM retrieval_traces t
/// JOIN retrieval_trace_items i ON i.trace_id = t.id
/// WHERE t.status = 'finalized'
///   AND i.record_id = ?1
///   AND i.gate_disposition = 'included'
///   AND i.injected_order IS NOT NULL
/// ORDER BY i.injected_order ASC
/// LIMIT 1
/// ```
pub fn get_used_evidence(
    conn: &rusqlite::Connection,
    record_id: &str,
) -> MemoryResult<Option<UsedEvidence>> {
    let result = conn.query_row(
        "SELECT t.id, i.record_id, i.injected_order, t.response_id
         FROM retrieval_traces t
         JOIN retrieval_trace_items i ON i.trace_id = t.id
         WHERE t.status = 'finalized'
           AND i.record_id = ?1
           AND i.gate_disposition = 'included'
           AND i.injected_order IS NOT NULL
         ORDER BY i.injected_order ASC
         LIMIT 1",
        rusqlite::params![record_id],
        |row| {
            Ok(UsedEvidence {
                trace_id: row.get(0)?,
                record_id: row.get(1)?,
                injected_order: row.get(2)?,
                response_id: row.get(3)?,
            })
        },
    );

    match result {
        Ok(evidence) => Ok(Some(evidence)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(StorageError::Sqlite(e).into()),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::Database;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn make_item(
        disposition: Option<&str>,
        injected_order: Option<i64>,
        strategy: &str,
    ) -> TraceItemView {
        TraceItemView {
            trace_id: "tr-test".to_string(),
            record_id: "rec-test".to_string(),
            gate_disposition: disposition.map(|s| s.to_string()),
            injected_order,
            strategy: strategy.to_string(),
            strategy_rank: Some(1),
            strategy_score: Some(0.85),
            rrf_contribution: Some(0.016),
        }
    }

    fn seed_trace(conn: &rusqlite::Connection, trace_id: &str, status: &str) {
        conn.execute(
            "INSERT INTO retrieval_traces (
                id, response_id, task_id, query_hash, query_class,
                classifier_version, profile_id, graph_revision, policy_hash,
                token_budget, status, degradation_json, embed_model_version,
                k_value, availability_json, weights_json,
                evidence_contribution, memory_worth_contribution, goal_contribution_total,
                created_at
            ) VALUES (
                ?1, 'resp-1', NULL, 'hash-1', 'exploratory',
                'classifier-v1', 'rrf-general-v1', NULL, NULL,
                4096, ?2, NULL, NULL,
                60.0, '{}', '{}',
                0.0, 0.0, 0.0,
                '2024-01-01T00:00:00Z'
            )",
            rusqlite::params![trace_id, status],
        )
        .expect("seed_trace failed");
    }

    fn seed_item(
        conn: &rusqlite::Connection,
        trace_id: &str,
        record_id: &str,
        disposition: &str,
        injected_order: Option<i64>,
    ) {
        conn.execute(
            "INSERT INTO retrieval_trace_items (
                trace_id, record_id, strategy, strategy_rank, strategy_score,
                weight, rrf_contribution, gate_disposition, reason_code,
                token_cost, allocated_tokens, injected_order, goal_id,
                evidence_contribution, memory_worth_contribution
            ) VALUES (
                ?1, ?2, 'fts', 1, 0.9,
                1.0, 0.016, ?3, NULL,
                100, ?4, ?5, NULL,
                NULL, NULL
            )",
            rusqlite::params![
                trace_id,
                record_id,
                disposition,
                injected_order,
                injected_order
            ],
        )
        .expect("seed_item failed");
    }

    // ── classify_explanation tests ────────────────────────────────────────────

    #[test]
    fn classify_no_trace_item_is_available_safe() {
        let kind = classify_explanation(None, true);
        assert_eq!(kind, ExplanationKind::AvailableSafe);
    }

    #[test]
    fn classify_no_trace_item_unfinalized_is_available_safe() {
        let kind = classify_explanation(None, false);
        assert_eq!(kind, ExplanationKind::AvailableSafe);
    }

    #[test]
    fn classify_unauthorized_is_available_safe() {
        let item = make_item(Some("unauthorized"), None, "fts");
        let kind = classify_explanation(Some(&item), true);
        assert_eq!(kind, ExplanationKind::AvailableSafe);
    }

    #[test]
    fn classify_excluded_is_retrieved_filtered() {
        let item = make_item(Some("excluded"), None, "fts");
        let kind = classify_explanation(Some(&item), true);
        assert_eq!(kind, ExplanationKind::RetrievedFiltered);
    }

    #[test]
    fn classify_filtered_is_retrieved_filtered() {
        let item = make_item(Some("filtered"), None, "vector");
        let kind = classify_explanation(Some(&item), true);
        assert_eq!(kind, ExplanationKind::RetrievedFiltered);
    }

    #[test]
    fn classify_included_with_order_finalized_is_how_used() {
        let item = make_item(Some("included"), Some(0), "fts");
        let kind = classify_explanation(Some(&item), true);
        assert_eq!(kind, ExplanationKind::HowUsed);
    }

    #[test]
    fn classify_included_with_order_not_finalized_is_why_recalled() {
        // Has injected_order but trace is NOT finalized → WhyRecalled (not "Used").
        let item = make_item(Some("included"), Some(0), "fts");
        let kind = classify_explanation(Some(&item), false);
        assert_eq!(kind, ExplanationKind::WhyRecalled);
    }

    #[test]
    fn classify_included_no_order_finalized_is_why_recalled() {
        // Finalized trace but injected_order IS NULL → WhyRecalled.
        let item = make_item(Some("included"), None, "vector");
        let kind = classify_explanation(Some(&item), true);
        assert_eq!(kind, ExplanationKind::WhyRecalled);
    }

    // ── build_explanation tests ───────────────────────────────────────────────

    #[test]
    fn build_explanation_is_used_only_for_how_used() {
        // Only the one combination that satisfies the invariant → is_used = true.
        let item = make_item(Some("included"), Some(0), "fts");
        let exp = build_explanation("rec-1", Some(&item), true);
        assert_eq!(exp.kind, ExplanationKind::HowUsed);
        assert!(exp.is_used, "HowUsed must produce is_used = true");
        assert_eq!(exp.record_id, "rec-1");
    }

    #[test]
    fn build_explanation_not_used_for_retrieved_filtered() {
        let item = make_item(Some("excluded"), None, "fts");
        let exp = build_explanation("rec-2", Some(&item), true);
        assert_eq!(exp.kind, ExplanationKind::RetrievedFiltered);
        assert!(
            !exp.is_used,
            "RetrievedFiltered must produce is_used = false"
        );
    }

    #[test]
    fn build_explanation_not_used_for_available_safe_no_trace() {
        let exp = build_explanation("rec-3", None, true);
        assert_eq!(exp.kind, ExplanationKind::AvailableSafe);
        assert!(!exp.is_used);
    }

    #[test]
    fn build_explanation_not_used_for_why_recalled() {
        // included but no injected_order → WhyRecalled, not Used.
        let item = make_item(Some("included"), None, "vector");
        let exp = build_explanation("rec-4", Some(&item), true);
        assert_eq!(exp.kind, ExplanationKind::WhyRecalled);
        assert!(!exp.is_used);
    }

    #[test]
    fn build_explanation_not_used_for_included_not_finalized() {
        let item = make_item(Some("included"), Some(1), "graph");
        let exp = build_explanation("rec-5", Some(&item), false);
        assert_eq!(exp.kind, ExplanationKind::WhyRecalled);
        assert!(!exp.is_used);
    }

    // ── get_used_evidence tests ───────────────────────────────────────────────

    #[test]
    fn get_used_evidence_returns_some_when_used() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        seed_trace(&conn, "tr-ev-1", "finalized");
        seed_item(&conn, "tr-ev-1", "rec-used", "included", Some(0));

        let evidence = get_used_evidence(&conn, "rec-used").unwrap();
        assert!(
            evidence.is_some(),
            "finalized + included + order → Some(evidence)"
        );
        let ev = evidence.unwrap();
        assert_eq!(ev.trace_id, "tr-ev-1");
        assert_eq!(ev.record_id, "rec-used");
        assert_eq!(ev.injected_order, 0);
    }

    #[test]
    fn get_used_evidence_returns_none_when_pending() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        // Trace is still "pending" — even with included + injected_order, not "Used".
        seed_trace(&conn, "tr-ev-2", "pending");
        seed_item(&conn, "tr-ev-2", "rec-pending", "included", Some(0));

        let evidence = get_used_evidence(&conn, "rec-pending").unwrap();
        assert!(
            evidence.is_none(),
            "pending trace must never produce UsedEvidence"
        );
    }

    #[test]
    fn get_used_evidence_returns_none_when_no_injected_order() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        // Finalized trace but injected_order is NULL.
        seed_trace(&conn, "tr-ev-3", "finalized");
        seed_item(&conn, "tr-ev-3", "rec-no-order", "included", None);

        let evidence = get_used_evidence(&conn, "rec-no-order").unwrap();
        assert!(
            evidence.is_none(),
            "NULL injected_order must never produce UsedEvidence"
        );
    }

    #[test]
    fn get_used_evidence_returns_none_when_excluded() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        seed_trace(&conn, "tr-ev-4", "finalized");
        seed_item(&conn, "tr-ev-4", "rec-excluded", "excluded", None);

        let evidence = get_used_evidence(&conn, "rec-excluded").unwrap();
        assert!(
            evidence.is_none(),
            "excluded disposition must never produce UsedEvidence"
        );
    }

    #[test]
    fn get_used_evidence_returns_none_for_unknown_record() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        let evidence = get_used_evidence(&conn, "no-such-record").unwrap();
        assert!(evidence.is_none());
    }

    // ── comprehensive invariant test ──────────────────────────────────────────

    /// Validates: only the exact combination of included + injected_order + finalized
    /// may produce is_used = true. All other combinations produce is_used = false.
    #[test]
    fn used_label_invariant_requires_finalized_trace_and_order() {
        let cases: &[(Option<&str>, Option<i64>, bool, bool)] = &[
            // (disposition, injected_order, trace_finalized, expected_is_used)
            (None, None, true, false),                 // no trace item
            (None, None, false, false),                // no trace item, not finalized
            (Some("unauthorized"), None, true, false), // opaque → AvailableSafe
            (Some("excluded"), None, true, false),     // filtered out
            (Some("filtered"), Some(0), true, false),  // filtered (order irrelevant)
            (Some("included"), None, true, false),     // included but no order
            (Some("included"), Some(0), false, false), // included + order but not finalized
            (Some("included"), Some(0), true, true),   // ← THE ONLY VALID "Used" case
            (Some("included"), Some(1), true, true),   // second-position also valid
            (Some("other"), Some(0), true, false),     // unknown disposition → WhyRecalled
        ];

        for (i, &(disposition, order, finalized, expected_used)) in cases.iter().enumerate() {
            let item_opt = disposition.map(|d| TraceItemView {
                trace_id: "tr-inv".to_string(),
                record_id: format!("rec-{i}"),
                gate_disposition: Some(d.to_string()),
                injected_order: order,
                strategy: "fts".to_string(),
                strategy_rank: Some(1),
                strategy_score: Some(0.9),
                rrf_contribution: Some(0.016),
            });

            let exp = build_explanation(&format!("rec-{i}"), item_opt.as_ref(), finalized);

            assert_eq!(
                exp.is_used, expected_used,
                "case {i}: disposition={disposition:?}, order={order:?}, finalized={finalized} → expected is_used={expected_used}, got {}",
                exp.is_used
            );
        }
    }
}
