//! Trace finalization after prompt construction (design §6.4 step 8, task F3.5.6).
//!
//! After actual prompt construction determines the exact set of injected items and
//! their order, this module updates the pending trace to finalized status. Only
//! items with a confirmed injected_order can produce a "Used" label.
//!
//! # Design invariants
//! * "Used" is proven ONLY by an item with a committed injected_order.
//! * Finalization is transactional: all item updates + status change in one write block.
//! * If finalization fails, the trace stays "pending" — it MUST NOT be labeled "Used".
//! * response_id and task_id identify the specific response where items were injected.

use crate::memory::error::{MemoryResult, StorageError};

// ── Types ─────────────────────────────────────────────────────────────────────

/// One injected item confirmed by actual prompt construction.
#[derive(Debug, Clone)]
pub struct InjectedItem {
    /// The trace_id this item belongs to.
    pub trace_id: String,
    /// The record_id that was injected (must match an existing trace item).
    pub record_id: String,
    /// The strategy this item came from.
    pub strategy: String,
    /// The exact 0-based position in the injected context (0 = first).
    pub injected_order: i64,
    /// The number of tokens actually allocated for this item.
    pub allocated_tokens: i64,
}

/// Input to finalize a pending trace.
#[derive(Debug, Clone)]
pub struct FinalizeTraceInput {
    /// Trace ID to finalize.
    pub trace_id: String,
    /// Response ID to link (may differ from the initial response_id if multiple retries).
    pub response_id: Option<String>,
    /// Task ID to link.
    pub task_id: Option<String>,
    /// The exact set of injected items from prompt construction.
    pub injected_items: Vec<InjectedItem>,
}

/// Result of trace finalization.
#[derive(Debug, Clone, PartialEq)]
pub enum FinalizeResult {
    /// Trace successfully finalized.
    Finalized {
        /// Number of items updated with injected_order.
        updated_items: usize,
    },
    /// Trace was not found (already deleted or never created).
    TraceNotFound,
    /// Trace was already finalized (idempotent — not an error).
    AlreadyFinalized,
}

// ── Core functions ────────────────────────────────────────────────────────────

/// Finalize a pending trace after actual prompt construction.
///
/// # Algorithm
/// 1. Check if trace exists and has status != "finalized". If already finalized → AlreadyFinalized.
/// 2. For each injected_item:
///    a. Update the matching trace item row (by trace_id + record_id + strategy) to set
///       `injected_order` and `allocated_tokens`.
/// 3. Update the trace header: set `status = "finalized"`, optionally update
///    `response_id` and `task_id`.
///
/// All operations use the `trace_store` module functions.
pub fn finalize_trace(
    conn: &rusqlite::Connection,
    input: &FinalizeTraceInput,
) -> MemoryResult<FinalizeResult> {
    // Step 1: check trace existence and current status.
    let status: Option<String> = {
        let result = conn.query_row(
            "SELECT status FROM retrieval_traces WHERE id = ?1",
            rusqlite::params![input.trace_id],
            |row| row.get(0),
        );
        match result {
            Ok(s) => Some(s),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(StorageError::Sqlite(e).into()),
        }
    };

    match status.as_deref() {
        None => return Ok(FinalizeResult::TraceNotFound),
        Some("finalized") => return Ok(FinalizeResult::AlreadyFinalized),
        Some(_) => {} // "pending", "partial", "failed" — proceed
    }

    // Step 2: update each injected item's injected_order and allocated_tokens.
    let mut updated_items: usize = 0;
    for item in &input.injected_items {
        let rows_changed = conn
            .execute(
                "UPDATE retrieval_trace_items
                 SET injected_order = ?1, allocated_tokens = ?2
                 WHERE trace_id = ?3 AND record_id = ?4 AND strategy = ?5",
                rusqlite::params![
                    item.injected_order,
                    item.allocated_tokens,
                    item.trace_id,
                    item.record_id,
                    item.strategy,
                ],
            )
            .map_err(StorageError::Sqlite)?;
        updated_items += rows_changed;
    }

    // Step 3: update the trace header to "finalized" and link response/task IDs.
    conn.execute(
        "UPDATE retrieval_traces
         SET status = 'finalized',
             response_id = COALESCE(?1, response_id),
             task_id = COALESCE(?2, task_id)
         WHERE id = ?3",
        rusqlite::params![input.response_id, input.task_id, input.trace_id],
    )
    .map_err(StorageError::Sqlite)?;

    Ok(FinalizeResult::Finalized { updated_items })
}

/// A record is "Used" if and only if it has a committed `injected_order` in a
/// "finalized" trace. This function checks whether a given record appears in any
/// finalized trace with an injected_order.
pub fn is_record_used(conn: &rusqlite::Connection, record_id: &str) -> MemoryResult<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM retrieval_traces t
             JOIN retrieval_trace_items i ON i.trace_id = t.id
             WHERE t.status = 'finalized'
               AND i.record_id = ?1
               AND i.injected_order IS NOT NULL",
            rusqlite::params![record_id],
            |row| row.get(0),
        )
        .map_err(StorageError::Sqlite)?;
    Ok(count > 0)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::Database;

    // ── helpers ───────────────────────────────────────────────────────────────

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
                ?1, 'resp-seed', 'task-seed', 'hash-seed', 'exploratory',
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

    fn seed_trace_item(
        conn: &rusqlite::Connection,
        trace_id: &str,
        record_id: &str,
        strategy: &str,
    ) {
        conn.execute(
            "INSERT INTO retrieval_trace_items (
                trace_id, record_id, strategy, strategy_rank, strategy_score,
                weight, rrf_contribution, gate_disposition, reason_code,
                token_cost, allocated_tokens, injected_order, goal_id,
                evidence_contribution, memory_worth_contribution
            ) VALUES (
                ?1, ?2, ?3, NULL, NULL,
                NULL, NULL, 'included', NULL,
                100, NULL, NULL, NULL,
                NULL, NULL
            )",
            rusqlite::params![trace_id, record_id, strategy],
        )
        .expect("seed_trace_item failed");
    }

    // ── tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn finalize_trace_sets_status_to_finalized() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        seed_trace(&conn, "tr-1", "pending");

        let input = FinalizeTraceInput {
            trace_id: "tr-1".to_string(),
            response_id: None,
            task_id: None,
            injected_items: vec![],
        };
        let result = finalize_trace(&conn, &input).unwrap();
        assert_eq!(result, FinalizeResult::Finalized { updated_items: 0 });

        // Verify status in DB.
        let status: String = conn
            .query_row(
                "SELECT status FROM retrieval_traces WHERE id = 'tr-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "finalized");
    }

    #[test]
    fn finalize_trace_updates_injected_order() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        seed_trace(&conn, "tr-2", "pending");
        seed_trace_item(&conn, "tr-2", "rec-a", "fts");
        seed_trace_item(&conn, "tr-2", "rec-b", "vector");

        let input = FinalizeTraceInput {
            trace_id: "tr-2".to_string(),
            response_id: None,
            task_id: None,
            injected_items: vec![
                InjectedItem {
                    trace_id: "tr-2".to_string(),
                    record_id: "rec-a".to_string(),
                    strategy: "fts".to_string(),
                    injected_order: 0,
                    allocated_tokens: 150,
                },
                InjectedItem {
                    trace_id: "tr-2".to_string(),
                    record_id: "rec-b".to_string(),
                    strategy: "vector".to_string(),
                    injected_order: 1,
                    allocated_tokens: 200,
                },
            ],
        };
        let result = finalize_trace(&conn, &input).unwrap();
        assert_eq!(result, FinalizeResult::Finalized { updated_items: 2 });

        // Verify injected_order and allocated_tokens for each item.
        let order_a: Option<i64> = conn
            .query_row(
                "SELECT injected_order FROM retrieval_trace_items
                 WHERE trace_id = 'tr-2' AND record_id = 'rec-a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(order_a, Some(0));

        let alloc_b: Option<i64> = conn
            .query_row(
                "SELECT allocated_tokens FROM retrieval_trace_items
                 WHERE trace_id = 'tr-2' AND record_id = 'rec-b'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(alloc_b, Some(200));
    }

    #[test]
    fn finalize_trace_already_finalized_is_idempotent() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        seed_trace(&conn, "tr-3", "finalized");

        let input = FinalizeTraceInput {
            trace_id: "tr-3".to_string(),
            response_id: None,
            task_id: None,
            injected_items: vec![],
        };
        let result = finalize_trace(&conn, &input).unwrap();
        assert_eq!(result, FinalizeResult::AlreadyFinalized);
    }

    #[test]
    fn finalize_trace_not_found() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        let input = FinalizeTraceInput {
            trace_id: "no-such-trace".to_string(),
            response_id: None,
            task_id: None,
            injected_items: vec![],
        };
        let result = finalize_trace(&conn, &input).unwrap();
        assert_eq!(result, FinalizeResult::TraceNotFound);
    }

    #[test]
    fn is_record_used_true_after_finalization() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        seed_trace(&conn, "tr-4", "pending");
        seed_trace_item(&conn, "tr-4", "rec-used", "fts");

        let input = FinalizeTraceInput {
            trace_id: "tr-4".to_string(),
            response_id: None,
            task_id: None,
            injected_items: vec![InjectedItem {
                trace_id: "tr-4".to_string(),
                record_id: "rec-used".to_string(),
                strategy: "fts".to_string(),
                injected_order: 0,
                allocated_tokens: 100,
            }],
        };
        finalize_trace(&conn, &input).unwrap();

        assert!(is_record_used(&conn, "rec-used").unwrap());
    }

    #[test]
    fn is_record_used_false_if_no_injected_order() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        // Finalize trace but don't include "rec-no-order" in injected_items,
        // so its injected_order stays NULL.
        seed_trace(&conn, "tr-5", "pending");
        seed_trace_item(&conn, "tr-5", "rec-no-order", "fts");

        let input = FinalizeTraceInput {
            trace_id: "tr-5".to_string(),
            response_id: None,
            task_id: None,
            injected_items: vec![], // deliberately empty
        };
        finalize_trace(&conn, &input).unwrap();

        // Trace is finalized but item has no injected_order → not "Used".
        assert!(!is_record_used(&conn, "rec-no-order").unwrap());
    }

    #[test]
    fn is_record_used_false_if_trace_still_pending() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        seed_trace(&conn, "tr-6", "pending");
        seed_trace_item(&conn, "tr-6", "rec-pending", "fts");

        // Manually set injected_order without finalizing the trace.
        conn.execute(
            "UPDATE retrieval_trace_items SET injected_order = 0 WHERE record_id = 'rec-pending'",
            [],
        )
        .unwrap();

        // Trace is still "pending" → is_record_used must be false.
        assert!(!is_record_used(&conn, "rec-pending").unwrap());
    }

    #[test]
    fn finalize_updates_response_id() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        seed_trace(&conn, "tr-7", "pending");

        let input = FinalizeTraceInput {
            trace_id: "tr-7".to_string(),
            response_id: Some("new-resp-42".to_string()),
            task_id: Some("new-task-7".to_string()),
            injected_items: vec![],
        };
        let result = finalize_trace(&conn, &input).unwrap();
        assert_eq!(result, FinalizeResult::Finalized { updated_items: 0 });

        let (resp_id, task_id): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT response_id, task_id FROM retrieval_traces WHERE id = 'tr-7'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(resp_id.as_deref(), Some("new-resp-42"));
        assert_eq!(task_id.as_deref(), Some("new-task-7"));
    }
}
