//! Retrieval trace persistence (design §4.3/§6.4, task F3.4.4).
//!
//! Provides types and functions for storing and retrieving replay-ready
//! retrieval trace records.  Traces capture all RRF replay fields (k value,
//! strategy availability, weights) and separate Evidence/Memory-Worth
//! contributions so offline replay from stored one-based ranks is exact.
//!
//! # Design invariants
//! * Uses `rusqlite::Connection` directly — callers hold the write connection.
//! * `availability_json` / `weights_json` are JSON objects built with
//!   `format!` (no serde required).
//! * Memory-Worth / evidence terms are tracked separately and are inert below
//!   20 observations (design §6.4 step 5).

use crate::error::{MemoryResult, StorageError};
use crate::retrieval::rrf_fusion::{StrategyAvailability, StrategyInput, StrategyKind};
use crate::retrieval::rrf_profile::FusionProfile;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Complete replay-ready trace header for one retrieval call.
#[derive(Debug, Clone, PartialEq)]
pub struct RetrievalTraceRecord {
    pub trace_id: String,
    pub response_id: Option<String>,
    pub task_id: Option<String>,
    pub query_hash: String,
    pub query_class: String,
    pub classifier_version: String,
    pub profile_id: String,
    pub graph_revision: Option<i64>,
    pub policy_hash: Option<String>,
    pub token_budget: Option<i64>,
    /// "pending" | "finalized" | "partial" | "failed"
    pub status: String,
    /// JSON describing unavailable strategies.
    pub degradation_json: Option<String>,
    pub embed_model_version: Option<String>,
    /// RRF damping constant k used in this fusion run.
    pub k_value: f32,
    /// JSON: `{"fts": 1, "vector": 0, ...}` (1=available, 0=unavailable).
    pub availability_json: String,
    /// JSON: `{"fts": 1.0, "vector": 1.2, ...}` per-strategy weights used.
    pub weights_json: String,
    /// Separate evidence term contribution (inert below 20 observations).
    pub evidence_contribution: f32,
    /// Memory-Worth contribution (inert below 20 observations).
    pub memory_worth_contribution: f32,
    /// Aggregate goal contribution across all active goals.
    pub goal_contribution_total: f32,
    /// RFC3339 UTC creation timestamp.
    pub created_at: String,
}

/// One trace item (per candidate, per strategy) for replay.
#[derive(Debug, Clone, PartialEq)]
pub struct RetrievalTraceItem {
    pub trace_id: String,
    pub record_id: String,
    /// Strategy name: "fts" | "vector" | "graph" | "temporal" | "goal"
    pub strategy: String,
    pub strategy_rank: Option<i64>,
    pub strategy_score: Option<f64>,
    /// Profile weight for this strategy.
    pub weight: Option<f64>,
    pub rrf_contribution: Option<f64>,
    /// "included" | "excluded" | "filtered" | "unauthorized"
    pub gate_disposition: Option<String>,
    pub reason_code: Option<String>,
    pub token_cost: Option<i64>,
    pub allocated_tokens: Option<i64>,
    pub injected_order: Option<i64>,
    pub goal_id: Option<String>,
    /// Per-item evidence term (separate from rrf_contribution).
    pub evidence_contribution: Option<f64>,
    /// Per-item Memory-Worth term (inert below 20 observations).
    pub memory_worth_contribution: Option<f64>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build the JSON string for the `availability_json` column from strategy inputs.
///
/// Format: `{"fts": 1, "vector": 0, "graph": 1, "temporal": 1, "goal": 0}`
/// where 1 = available and 0 = unavailable.  Any strategy not present in
/// `strategies` is omitted (no default assumed).
pub fn availability_to_json(strategies: &[StrategyInput]) -> String {
    let pairs: Vec<String> = strategies
        .iter()
        .map(|s| {
            let name = strategy_kind_name(s.strategy);
            let flag = match s.availability {
                StrategyAvailability::Available => 1,
                StrategyAvailability::Unavailable => 0,
            };
            format!("\"{name}\": {flag}")
        })
        .collect();
    format!("{{{}}}", pairs.join(", "))
}

/// Build the JSON string for the `weights_json` column from a fusion profile.
///
/// Format: `{"fts": 1.0, "vector": 1.2, "graph": 0.8, "temporal": 0.6, "goal": 0.6}`
pub fn weights_to_json(profile: &FusionProfile) -> String {
    let w = &profile.weights;
    format!(
        "{{\"fts\": {fts}, \"vector\": {vec}, \"graph\": {gph}, \"temporal\": {tmp}, \"goal\": {goal}}}",
        fts  = w.fts,
        vec  = w.vector,
        gph  = w.graph,
        tmp  = w.temporal,
        goal = w.goal,
    )
}

/// Canonical lowercase name for a `StrategyKind`.
#[inline]
pub fn strategy_kind_name(kind: StrategyKind) -> &'static str {
    match kind {
        StrategyKind::Fts => "fts",
        StrategyKind::Vector => "vector",
        StrategyKind::Graph => "graph",
        StrategyKind::Temporal => "temporal",
        StrategyKind::Goal => "goal",
    }
}

// ── CRUD functions ────────────────────────────────────────────────────────────

/// Insert a new trace header into `retrieval_traces`.
pub fn insert_trace(conn: &rusqlite::Connection, trace: &RetrievalTraceRecord) -> MemoryResult<()> {
    conn.execute(
        "INSERT INTO retrieval_traces (
            id, response_id, task_id, query_hash, query_class,
            classifier_version, profile_id, graph_revision, policy_hash,
            token_budget, status, degradation_json, embed_model_version,
            k_value, availability_json, weights_json,
            evidence_contribution, memory_worth_contribution, goal_contribution_total,
            created_at
        ) VALUES (
            ?1,  ?2,  ?3,  ?4,  ?5,
            ?6,  ?7,  ?8,  ?9,
            ?10, ?11, ?12, ?13,
            ?14, ?15, ?16,
            ?17, ?18, ?19,
            ?20
        )",
        rusqlite::params![
            trace.trace_id,
            trace.response_id,
            trace.task_id,
            trace.query_hash,
            trace.query_class,
            trace.classifier_version,
            trace.profile_id,
            trace.graph_revision,
            trace.policy_hash,
            trace.token_budget,
            trace.status,
            trace.degradation_json,
            trace.embed_model_version,
            trace.k_value,
            trace.availability_json,
            trace.weights_json,
            trace.evidence_contribution,
            trace.memory_worth_contribution,
            trace.goal_contribution_total,
            trace.created_at,
        ],
    )
    .map_err(StorageError::Sqlite)?;
    Ok(())
}

/// Insert a trace item into `retrieval_trace_items`.
pub fn insert_trace_item(
    conn: &rusqlite::Connection,
    item: &RetrievalTraceItem,
) -> MemoryResult<()> {
    conn.execute(
        "INSERT INTO retrieval_trace_items (
            trace_id, record_id, strategy, strategy_rank, strategy_score,
            weight, rrf_contribution, gate_disposition, reason_code,
            token_cost, allocated_tokens, injected_order, goal_id,
            evidence_contribution, memory_worth_contribution
        ) VALUES (
            ?1,  ?2,  ?3,  ?4,  ?5,
            ?6,  ?7,  ?8,  ?9,
            ?10, ?11, ?12, ?13,
            ?14, ?15
        )",
        rusqlite::params![
            item.trace_id,
            item.record_id,
            item.strategy,
            item.strategy_rank,
            item.strategy_score,
            item.weight,
            item.rrf_contribution,
            item.gate_disposition,
            item.reason_code,
            item.token_cost,
            item.allocated_tokens,
            item.injected_order,
            item.goal_id,
            item.evidence_contribution,
            item.memory_worth_contribution,
        ],
    )
    .map_err(StorageError::Sqlite)?;
    Ok(())
}

/// Insert multiple trace items in one batch (one SQL per item).
pub fn insert_trace_items(
    conn: &rusqlite::Connection,
    items: &[RetrievalTraceItem],
) -> MemoryResult<()> {
    for item in items {
        insert_trace_item(conn, item)?;
    }
    Ok(())
}

/// Retrieve a trace header by ID.
pub fn get_trace(
    conn: &rusqlite::Connection,
    trace_id: &str,
) -> MemoryResult<Option<RetrievalTraceRecord>> {
    let result = conn.query_row(
        "SELECT
            id, response_id, task_id, query_hash, query_class,
            classifier_version, profile_id, graph_revision, policy_hash,
            token_budget, status, degradation_json, embed_model_version,
            k_value, availability_json, weights_json,
            evidence_contribution, memory_worth_contribution, goal_contribution_total,
            created_at
         FROM retrieval_traces
         WHERE id = ?1",
        rusqlite::params![trace_id],
        |row| {
            Ok(RetrievalTraceRecord {
                trace_id: row.get(0)?,
                response_id: row.get(1)?,
                task_id: row.get(2)?,
                query_hash: row.get(3)?,
                query_class: row.get(4)?,
                classifier_version: row.get(5)?,
                profile_id: row.get(6)?,
                graph_revision: row.get(7)?,
                policy_hash: row.get(8)?,
                token_budget: row.get(9)?,
                status: row.get(10)?,
                degradation_json: row.get(11)?,
                embed_model_version: row.get(12)?,
                k_value: row.get(13)?,
                availability_json: row.get(14)?,
                weights_json: row.get(15)?,
                evidence_contribution: row.get(16)?,
                memory_worth_contribution: row.get(17)?,
                goal_contribution_total: row.get(18)?,
                created_at: row.get(19)?,
            })
        },
    );

    match result {
        Ok(record) => Ok(Some(record)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(StorageError::Sqlite(e).into()),
    }
}

/// Retrieve all items for a trace.
pub fn get_trace_items(
    conn: &rusqlite::Connection,
    trace_id: &str,
) -> MemoryResult<Vec<RetrievalTraceItem>> {
    let mut stmt = conn
        .prepare(
            "SELECT
                trace_id, record_id, strategy, strategy_rank, strategy_score,
                weight, rrf_contribution, gate_disposition, reason_code,
                token_cost, allocated_tokens, injected_order, goal_id,
                evidence_contribution, memory_worth_contribution
             FROM retrieval_trace_items
             WHERE trace_id = ?1
             ORDER BY COALESCE(injected_order, 9999999), record_id",
        )
        .map_err(StorageError::Sqlite)?;

    let items = stmt
        .query_map(rusqlite::params![trace_id], |row| {
            Ok(RetrievalTraceItem {
                trace_id: row.get(0)?,
                record_id: row.get(1)?,
                strategy: row.get(2)?,
                strategy_rank: row.get(3)?,
                strategy_score: row.get(4)?,
                weight: row.get(5)?,
                rrf_contribution: row.get(6)?,
                gate_disposition: row.get(7)?,
                reason_code: row.get(8)?,
                token_cost: row.get(9)?,
                allocated_tokens: row.get(10)?,
                injected_order: row.get(11)?,
                goal_id: row.get(12)?,
                evidence_contribution: row.get(13)?,
                memory_worth_contribution: row.get(14)?,
            })
        })
        .map_err(StorageError::Sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::Sqlite)?;

    Ok(items)
}

/// Update the status of an existing trace (e.g., "pending" → "finalized").
pub fn update_trace_status(
    conn: &rusqlite::Connection,
    trace_id: &str,
    status: &str,
) -> MemoryResult<()> {
    conn.execute(
        "UPDATE retrieval_traces SET status = ?1 WHERE id = ?2",
        rusqlite::params![status, trace_id],
    )
    .map_err(StorageError::Sqlite)?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::retrieval::rrf_fusion::StrategyAvailability;
    use crate::retrieval::rrf_profile::PROFILE_EXPLORATORY;

    // ── builders ──────────────────────────────────────────────────────────────

    fn sample_trace(trace_id: &str) -> RetrievalTraceRecord {
        RetrievalTraceRecord {
            trace_id: trace_id.to_string(),
            response_id: Some("resp-1".to_string()),
            task_id: Some("task-1".to_string()),
            query_hash: "abc123def456".to_string(),
            query_class: "exploratory".to_string(),
            classifier_version: "classifier-v1".to_string(),
            profile_id: "rrf-general-v1".to_string(),
            graph_revision: Some(7),
            policy_hash: Some("polhash".to_string()),
            token_budget: Some(4096),
            status: "pending".to_string(),
            degradation_json: None,
            embed_model_version: Some("fastembed-v1".to_string()),
            k_value: 60.0,
            availability_json: r#"{"fts": 1, "vector": 1, "graph": 0, "temporal": 1, "goal": 0}"#
                .to_string(),
            weights_json:
                r#"{"fts": 1.0, "vector": 1.2, "graph": 0.8, "temporal": 0.6, "goal": 0.6}"#
                    .to_string(),
            evidence_contribution: 0.0,
            memory_worth_contribution: 0.0,
            goal_contribution_total: 0.15,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    fn sample_item(trace_id: &str, record_id: &str, strategy: &str) -> RetrievalTraceItem {
        RetrievalTraceItem {
            trace_id: trace_id.to_string(),
            record_id: record_id.to_string(),
            strategy: strategy.to_string(),
            strategy_rank: Some(1),
            strategy_score: Some(0.95),
            weight: Some(1.0),
            rrf_contribution: Some(0.016),
            gate_disposition: Some("included".to_string()),
            reason_code: None,
            token_cost: Some(120),
            allocated_tokens: Some(120),
            injected_order: Some(0),
            goal_id: None,
            evidence_contribution: Some(0.0),
            memory_worth_contribution: Some(0.0),
        }
    }

    // ── tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn insert_and_retrieve_trace_round_trips() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let trace = sample_trace("trace-rt-1");
        insert_trace(&conn, &trace).unwrap();

        let retrieved = get_trace(&conn, "trace-rt-1")
            .unwrap()
            .expect("must be Some");
        assert_eq!(retrieved.trace_id, trace.trace_id);
        assert_eq!(retrieved.response_id, trace.response_id);
        assert_eq!(retrieved.task_id, trace.task_id);
        assert_eq!(retrieved.query_hash, trace.query_hash);
        assert_eq!(retrieved.query_class, trace.query_class);
        assert_eq!(retrieved.classifier_version, trace.classifier_version);
        assert_eq!(retrieved.profile_id, trace.profile_id);
        assert_eq!(retrieved.graph_revision, trace.graph_revision);
        assert_eq!(retrieved.policy_hash, trace.policy_hash);
        assert_eq!(retrieved.token_budget, trace.token_budget);
        assert_eq!(retrieved.status, trace.status);
        assert_eq!(retrieved.degradation_json, trace.degradation_json);
        assert_eq!(retrieved.embed_model_version, trace.embed_model_version);
        assert!((retrieved.k_value - trace.k_value).abs() < 1e-5);
        assert_eq!(retrieved.availability_json, trace.availability_json);
        assert_eq!(retrieved.weights_json, trace.weights_json);
        assert!((retrieved.evidence_contribution - trace.evidence_contribution).abs() < 1e-5);
        assert!(
            (retrieved.memory_worth_contribution - trace.memory_worth_contribution).abs() < 1e-5
        );
        assert!((retrieved.goal_contribution_total - trace.goal_contribution_total).abs() < 1e-5);
        assert_eq!(retrieved.created_at, trace.created_at);
    }

    #[test]
    fn insert_and_retrieve_trace_items() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        let trace = sample_trace("trace-items-1");
        insert_trace(&conn, &trace).unwrap();

        let items = vec![
            sample_item("trace-items-1", "rec-a", "fts"),
            sample_item("trace-items-1", "rec-b", "vector"),
            sample_item("trace-items-1", "rec-c", "graph"),
        ];
        insert_trace_items(&conn, &items).unwrap();

        let retrieved = get_trace_items(&conn, "trace-items-1").unwrap();
        assert_eq!(retrieved.len(), 3);

        // Verify one item field-by-field.
        let fts_item = retrieved
            .iter()
            .find(|i| i.record_id == "rec-a")
            .expect("rec-a must be present");
        assert_eq!(fts_item.trace_id, "trace-items-1");
        assert_eq!(fts_item.strategy, "fts");
        assert_eq!(fts_item.strategy_rank, Some(1));
        assert!((fts_item.strategy_score.unwrap() - 0.95).abs() < 1e-9);
        assert_eq!(fts_item.gate_disposition.as_deref(), Some("included"));
        assert_eq!(fts_item.evidence_contribution, Some(0.0));
        assert_eq!(fts_item.memory_worth_contribution, Some(0.0));
    }

    #[test]
    fn update_trace_status_changes_status() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        let trace = sample_trace("trace-status-1");
        insert_trace(&conn, &trace).unwrap();

        // Initial status is "pending".
        let r1 = get_trace(&conn, "trace-status-1").unwrap().unwrap();
        assert_eq!(r1.status, "pending");

        update_trace_status(&conn, "trace-status-1", "finalized").unwrap();

        let r2 = get_trace(&conn, "trace-status-1").unwrap().unwrap();
        assert_eq!(r2.status, "finalized");
    }

    #[test]
    fn get_trace_items_empty_for_unknown_id() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let items = get_trace_items(&conn, "no-such-trace").unwrap();
        assert!(items.is_empty(), "unknown trace_id must return empty Vec");
    }

    #[test]
    fn get_trace_returns_none_for_unknown_id() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let result = get_trace(&conn, "no-such-trace").unwrap();
        assert!(result.is_none(), "unknown trace_id must return None");
    }

    #[test]
    fn availability_to_json_correct_format() {
        let strategies = vec![
            StrategyInput {
                strategy: StrategyKind::Fts,
                availability: StrategyAvailability::Available,
                candidates: vec![],
            },
            StrategyInput {
                strategy: StrategyKind::Vector,
                availability: StrategyAvailability::Available,
                candidates: vec![],
            },
            StrategyInput {
                strategy: StrategyKind::Graph,
                availability: StrategyAvailability::Unavailable,
                candidates: vec![],
            },
            StrategyInput {
                strategy: StrategyKind::Temporal,
                availability: StrategyAvailability::Available,
                candidates: vec![],
            },
            StrategyInput {
                strategy: StrategyKind::Goal,
                availability: StrategyAvailability::Unavailable,
                candidates: vec![],
            },
        ];
        let json = availability_to_json(&strategies);
        assert_eq!(
            json,
            r#"{"fts": 1, "vector": 1, "graph": 0, "temporal": 1, "goal": 0}"#
        );
    }

    #[test]
    fn weights_to_json_correct_format() {
        // Use PROFILE_EXPLORATORY: fts=1.0, vector=1.2, graph=0.8, temporal=0.6, goal=0.6
        let json = weights_to_json(&PROFILE_EXPLORATORY);
        assert_eq!(
            json,
            r#"{"fts": 1, "vector": 1.2, "graph": 0.8, "temporal": 0.6, "goal": 0.6}"#
        );
    }

    #[test]
    fn insert_trace_items_batch_inserts_all() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        let trace = sample_trace("trace-batch-1");
        insert_trace(&conn, &trace).unwrap();

        let items: Vec<RetrievalTraceItem> = (0..5)
            .map(|i| sample_item("trace-batch-1", &format!("rec-{i}"), "fts"))
            .collect();
        insert_trace_items(&conn, &items).unwrap();

        let retrieved = get_trace_items(&conn, "trace-batch-1").unwrap();
        assert_eq!(
            retrieved.len(),
            5,
            "all 5 items must be inserted and retrieved"
        );
    }
}
