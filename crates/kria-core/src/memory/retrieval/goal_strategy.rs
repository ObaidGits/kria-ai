//! Goal retrieval strategy (design §6.5, task F3.3.5).
//!
//! Selects only caller/task/session-authorized **Active** goals, ranks them by
//! a normalized `goal_contribution` score that factors in priority, authority
//! score, and recent progress, and records goal ID / contribution for trace
//! purposes.  All other statuses (Candidate, Paused, Completed, Conflicted,
//! Stale, Superseded, Deleted) contribute zero and are excluded.
//!
//! # Design invariants (design §6.5 / invariant A5)
//! * Policy gate (namespace / scope / sensitivity) is applied BEFORE any
//!   ranking — A5.
//! * Only `status = 'active'` goals are returned; all other statuses are
//!   excluded (contribute zero).
//! * Goal IDs and contribution scores are recorded in
//!   `retrieval_trace_items.goal_id` / `rrf_contribution` for trace purposes.
//! * Profile name is `"goal-v1"`.
//! * Hard maximum result cap is 100 (design §6.2: goal budget 100).

use std::sync::Arc;

use chrono::{Duration, Utc};
use rusqlite::Connection;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::retrieval::StrategyDeadline;

// ── Hard constants ───────────────────────────────────────────────────────────

/// Profile name for this strategy.
pub const PROFILE: &str = "goal-v1";

/// Hard cap on returned candidates regardless of caller request (design §6.2).
pub const MAX_RESULTS_HARD: usize = 100;

// ── GoalRetrievalRequest ─────────────────────────────────────────────────────

/// Input to [`retrieve_active_goals`].
#[derive(Debug, Clone)]
pub struct GoalRetrievalRequest {
    /// Caller namespace — only goals with matching `namespace` are visible.
    pub caller_namespace: String,
    /// Caller scope — only goals with matching `scope` are visible.
    pub caller_scope: String,
    /// Sensitivity ceiling — goals with `sensitivity > max_sensitivity` are
    /// excluded by the policy gate (invariant A5).
    pub max_sensitivity: i64,
    /// Optional task ID for context filtering. When `Some`, only goals whose
    /// task context matches are considered (implementation: no extra column
    /// filtering unless the schema exposes it; reserved for future use here we
    /// pass it through without filtering since goals_v2 has no task_id column).
    pub task_id: Option<String>,
    /// Optional session ID for context filtering.  Same note as `task_id`.
    pub session_id: Option<String>,
    /// Maximum results requested.  Clamped to [`MAX_RESULTS_HARD`].
    pub max_results: usize,
    /// Wall-clock deadline. When expired the strategy returns the candidates
    /// collected so far with `partial = true`.
    pub deadline: StrategyDeadline,
}

// ── GoalRetrievalResult ──────────────────────────────────────────────────────

/// Output of [`retrieve_active_goals`].
#[derive(Debug, Clone)]
pub struct GoalRetrievalResult {
    pub candidates: Vec<GoalCandidate>,
    /// `true` when the deadline fired before all results were collected.
    pub partial: bool,
}

// ── GoalCandidate ────────────────────────────────────────────────────────────

/// One goal candidate returned by the goal strategy.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalCandidate {
    /// Goal UUID.
    pub goal_id: String,
    /// Goal title.
    pub title: String,
    /// Goal kind (e.g. `"user"`, `"system"`, `"task"`).
    pub kind: Option<String>,
    /// Resumption context — used to seed related memory retrieval.
    pub resumption_context: Option<String>,
    /// Priority 0–10 (higher = more urgent).
    pub priority: i64,
    /// Optional authority score.
    pub score: Option<f64>,
    /// Score semantics label.
    pub score_semantics: Option<String>,
    /// Authority revision.
    pub revision: Option<i64>,
    /// Normalized goal contribution score \[0.0, 1.0\] for RRF fusion.
    pub goal_contribution: f32,
    /// Human-readable rationale.
    pub score_rationale: String,
}

// ── retrieve_active_goals ────────────────────────────────────────────────────

/// Retrieve and rank active goals from the authority, enforcing policy BEFORE
/// ranking.
///
/// # Contract
/// * Policy gate (namespace / scope / sensitivity) applied first (A5).
/// * Only `status = 'active'` goals are returned.  All other statuses
///   contribute zero (empty list / excluded).
/// * Ranking under `goal-v1`:
///   1. Base contribution: `priority / 10.0`
///   2. Score boost:       `score.min(1.0) * 0.3` when `score > 0.0`
///   3. Recent-progress boost: `+0.1` when latest `goal_progress.observed_at`
///      is within the last 24 hours
///   4. Capped at 1.0
/// * Results ordered by `goal_contribution DESC`, then `priority DESC`, then
///   `goal_id ASC` (stable deterministic output).
/// * Capped at `min(req.max_results, MAX_RESULTS_HARD)`.
pub fn retrieve_active_goals(
    db: &Arc<Database>,
    req: &GoalRetrievalRequest,
) -> MemoryResult<GoalRetrievalResult> {
    db.with_read(|conn| retrieve_active_goals_inner(conn, req))
}

fn retrieve_active_goals_inner(
    conn: &Connection,
    req: &GoalRetrievalRequest,
) -> MemoryResult<GoalRetrievalResult> {
    let max_results = req.max_results.min(MAX_RESULTS_HARD);

    // Compute the 24-hour recency cutoff for the progress boost.
    let cutoff_24h = (Utc::now() - Duration::hours(24)).to_rfc3339();

    let sql = "
        SELECT
            g.id,
            COALESCE(g.title, ''),
            g.kind,
            g.resumption_context,
            COALESCE(g.priority, 0),
            g.score,
            g.score_semantics,
            g.revision,
            MAX(gp.observed_at) AS latest_progress
        FROM goals_v2 g
        LEFT JOIN goal_progress gp ON gp.goal_id = g.id
        WHERE g.status     = 'active'
          AND g.namespace  = ?1
          AND g.scope      = ?2
          AND g.sensitivity <= ?3
        GROUP BY g.id
        ORDER BY g.id ASC
    ";

    let mut stmt = conn.prepare(sql).map_err(StorageError::Sqlite)?;
    let rows = stmt
        .query_map(
            rusqlite::params![req.caller_namespace, req.caller_scope, req.max_sensitivity,],
            |row| {
                let goal_id: String = row.get(0)?;
                let title: String = row.get(1)?;
                let kind: Option<String> = row.get(2)?;
                let resumption_context: Option<String> = row.get(3)?;
                let priority: i64 = row.get(4)?;
                let score: Option<f64> = row.get(5)?;
                let score_semantics: Option<String> = row.get(6)?;
                let revision: Option<i64> = row.get(7)?;
                let latest_progress: Option<String> = row.get(8)?;
                Ok((
                    goal_id,
                    title,
                    kind,
                    resumption_context,
                    priority,
                    score,
                    score_semantics,
                    revision,
                    latest_progress,
                ))
            },
        )
        .map_err(StorageError::Sqlite)?;

    let mut candidates: Vec<GoalCandidate> = Vec::new();
    for row_result in rows {
        let (
            goal_id,
            title,
            kind,
            resumption_context,
            priority,
            score,
            score_semantics,
            revision,
            latest_progress,
        ) = row_result.map_err(StorageError::Sqlite)?;

        let contribution =
            compute_contribution(priority, score, latest_progress.as_deref(), &cutoff_24h);
        let rationale =
            format!("{PROFILE}: active goal — priority {priority}, contribution {contribution:.2}");

        candidates.push(GoalCandidate {
            goal_id,
            title,
            kind,
            resumption_context,
            priority,
            score,
            score_semantics,
            revision,
            goal_contribution: contribution,
            score_rationale: rationale,
        });
    }

    // Sort: contribution DESC, priority DESC, goal_id ASC (stable).
    candidates.sort_by(|a, b| {
        b.goal_contribution
            .partial_cmp(&a.goal_contribution)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.priority.cmp(&a.priority))
            .then_with(|| a.goal_id.cmp(&b.goal_id))
    });

    candidates.truncate(max_results);
    let partial = req.deadline.is_expired();
    Ok(GoalRetrievalResult {
        candidates,
        partial,
    })
}

// ── Contribution scoring ─────────────────────────────────────────────────────

/// Compute the normalized `goal_contribution` for one goal under `goal-v1`.
///
/// Formula (capped at 1.0):
/// * Base:              `priority / 10.0`
/// * Score boost:       `score.min(1.0) * 0.3`  (only when `score > 0.0`)
/// * Progress boost:    `0.1` when `latest_observed_at >= cutoff_24h`
fn compute_contribution(
    priority: i64,
    score: Option<f64>,
    latest_progress: Option<&str>,
    cutoff_24h: &str,
) -> f32 {
    // Base from priority (0..10 → 0.0..1.0).
    let base = (priority.clamp(0, 10) as f32) / 10.0;

    // Optional score boost.
    let score_boost = match score {
        Some(s) if s > 0.0 => (s.min(1.0) as f32) * 0.3,
        _ => 0.0,
    };

    // Recent progress boost (within last 24 hours).
    let progress_boost = match latest_progress {
        Some(ts) if ts >= cutoff_24h => 0.1_f32,
        _ => 0.0,
    };

    (base + score_boost + progress_boost).min(1.0)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::Database;
    use crate::memory::ids::new_id;
    use rusqlite::params;

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn open() -> Arc<Database> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    fn new_id_str() -> String {
        new_id().to_string()
    }

    /// Default request using "core" namespace / "global" scope / sensitivity ≤ 3.
    fn req_default() -> GoalRetrievalRequest {
        GoalRetrievalRequest {
            caller_namespace: "core".into(),
            caller_scope: "global".into(),
            max_sensitivity: 3,
            task_id: None,
            session_id: None,
            max_results: 50,
            deadline: StrategyDeadline::never(),
        }
    }

    /// Insert the minimal `events_v2` row required by FK constraints.
    fn seed_event(conn: &rusqlite::Connection, event_id: &str) {
        conn.execute(
            "INSERT OR IGNORE INTO events_v2(
                 id, phase, hlc, ts_utc, tz_offset_min, event_type,
                 source_kind, source_id, actor_id,
                 namespace, owner_id, scope, sensitivity, policy_version,
                 payload_plain, payload_encoding, payload_checksum, schema_version)
             VALUES (?1,'start','hlc-goal-seed','2024-01-01T00:00:00Z',0,'observation',
                     'user','src-1','actor-1',
                     'core','owner-1','global',0,'p1',
                     '{}','utf8','chk',1)",
            params![event_id],
        )
        .unwrap();
    }

    /// Insert a goal row into `goals_v2`.
    #[allow(clippy::too_many_arguments)]
    fn insert_goal(
        conn: &rusqlite::Connection,
        id: &str,
        event_id: &str,
        status: &str,
        priority: i64,
        score: Option<f64>,
        namespace: &str,
        scope: &str,
        resumption_context: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO goals_v2(
                 id, kind, title, status, priority, score,
                 resumption_context,
                 namespace, owner_id, scope, sensitivity,
                 source_id, policy_version,
                 created_event_id, created_at, revision)
             VALUES (?1, 'user', 'Test Goal', ?2, ?3, ?4,
                     ?5,
                     ?6, 'owner-1', ?7, 0,
                     'src-1', 'p1',
                     ?8, '2024-01-01T00:00:00Z', 1)",
            params![
                id,
                status,
                priority,
                score,
                resumption_context,
                namespace,
                scope,
                event_id
            ],
        )
        .unwrap();
    }

    /// Insert a `goal_progress` row.
    fn insert_goal_progress(
        conn: &rusqlite::Connection,
        goal_id: &str,
        event_id: &str,
        observed_at: &str,
    ) {
        let progress_id = new_id_str();
        conn.execute(
            "INSERT INTO goal_progress(id, goal_id, event_id, state, summary, observed_at, revision)
             VALUES (?1, ?2, ?3, 'in_progress', 'some progress', ?4, 1)",
            params![progress_id, goal_id, event_id, observed_at],
        )
        .unwrap();
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn active_goal_is_returned() {
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        let goal_id = new_id_str();
        seed_event(&conn, &event_id);
        insert_goal(
            &conn, &goal_id, &event_id, "active", 5, None, "core", "global", None,
        );
        drop(conn);

        let results = retrieve_active_goals(&db, &req_default()).unwrap();
        assert_eq!(results.candidates.len(), 1);
        assert_eq!(results.candidates[0].goal_id, goal_id);
    }

    #[test]
    fn candidate_goal_contributes_zero() {
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        let goal_id = new_id_str();
        seed_event(&conn, &event_id);
        insert_goal(
            &conn,
            &goal_id,
            &event_id,
            "candidate",
            5,
            None,
            "core",
            "global",
            None,
        );
        drop(conn);

        let results = retrieve_active_goals(&db, &req_default()).unwrap();
        assert!(
            results.candidates.is_empty(),
            "candidate goal must not be returned"
        );
    }

    #[test]
    fn paused_goal_excluded() {
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        let goal_id = new_id_str();
        seed_event(&conn, &event_id);
        insert_goal(
            &conn, &goal_id, &event_id, "paused", 5, None, "core", "global", None,
        );
        drop(conn);

        let results = retrieve_active_goals(&db, &req_default()).unwrap();
        assert!(
            results.candidates.is_empty(),
            "paused goal must not be returned"
        );
    }

    #[test]
    fn completed_goal_excluded() {
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        let goal_id = new_id_str();
        seed_event(&conn, &event_id);
        insert_goal(
            &conn,
            &goal_id,
            &event_id,
            "completed",
            8,
            None,
            "core",
            "global",
            None,
        );
        drop(conn);

        let results = retrieve_active_goals(&db, &req_default()).unwrap();
        assert!(
            results.candidates.is_empty(),
            "completed goal must not be returned"
        );
    }

    #[test]
    fn deleted_goal_excluded() {
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        let goal_id = new_id_str();
        seed_event(&conn, &event_id);
        insert_goal(
            &conn, &goal_id, &event_id, "deleted", 10, None, "core", "global", None,
        );
        drop(conn);

        let results = retrieve_active_goals(&db, &req_default()).unwrap();
        assert!(
            results.candidates.is_empty(),
            "deleted goal must not be returned"
        );
    }

    #[test]
    fn goals_ordered_by_priority_descending() {
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        let goal_lo = new_id_str();
        let goal_hi = new_id_str();
        seed_event(&conn, &event_id);
        insert_goal(
            &conn, &goal_lo, &event_id, "active", 3, None, "core", "global", None,
        );
        insert_goal(
            &conn, &goal_hi, &event_id, "active", 8, None, "core", "global", None,
        );
        drop(conn);

        let results = retrieve_active_goals(&db, &req_default()).unwrap();
        assert_eq!(results.candidates.len(), 2);
        assert_eq!(
            results.candidates[0].priority, 8,
            "highest priority goal must come first"
        );
        assert_eq!(results.candidates[1].priority, 3);
    }

    #[test]
    fn policy_namespace_gate() {
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        let goal_id = new_id_str();
        seed_event(&conn, &event_id);
        // Insert goal in a different namespace.
        insert_goal(
            &conn, &goal_id, &event_id, "active", 7, None, "other-ns", "global", None,
        );
        drop(conn);

        let results = retrieve_active_goals(&db, &req_default()).unwrap();
        assert!(
            results.candidates.is_empty(),
            "goal in wrong namespace must be excluded"
        );
    }

    #[test]
    fn goal_contribution_normalized_to_unit_interval() {
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        seed_event(&conn, &event_id);
        // Insert goals with various priorities and scores to test normalization.
        for priority in [0, 3, 7, 10] {
            let goal_id = new_id_str();
            insert_goal(
                &conn,
                &goal_id,
                &event_id,
                "active",
                priority,
                Some(0.9),
                "core",
                "global",
                None,
            );
        }
        drop(conn);

        let results = retrieve_active_goals(&db, &req_default()).unwrap();
        assert!(!results.candidates.is_empty());
        for c in &results.candidates {
            assert!(
                c.goal_contribution >= 0.0 && c.goal_contribution <= 1.0,
                "goal_contribution {:.4} must be in [0.0, 1.0]",
                c.goal_contribution
            );
        }
    }

    #[test]
    fn recent_progress_boosts_contribution() {
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        let goal_id = new_id_str();
        seed_event(&conn, &event_id);
        insert_goal(
            &conn, &goal_id, &event_id, "active", 5, None, "core", "global", None,
        );

        // Insert progress with observed_at = now (within 24h).
        let now = Utc::now().to_rfc3339();
        insert_goal_progress(&conn, &goal_id, &event_id, &now);
        drop(conn);

        let results = retrieve_active_goals(&db, &req_default()).unwrap();
        assert_eq!(results.candidates.len(), 1);
        let base = 5.0_f32 / 10.0; // 0.5
        assert!(
            results.candidates[0].goal_contribution > base,
            "recent progress must boost contribution above base {base:.2}, got {:.4}",
            results.candidates[0].goal_contribution
        );
    }

    #[test]
    fn resumption_context_preserved() {
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        let goal_id = new_id_str();
        let ctx = "Resume from last checkpoint: step 3/5";
        seed_event(&conn, &event_id);
        insert_goal(
            &conn,
            &goal_id,
            &event_id,
            "active",
            4,
            None,
            "core",
            "global",
            Some(ctx),
        );
        drop(conn);

        let results = retrieve_active_goals(&db, &req_default()).unwrap();
        assert_eq!(results.candidates.len(), 1);
        assert_eq!(
            results.candidates[0].resumption_context.as_deref(),
            Some(ctx),
            "resumption_context must be preserved in the candidate"
        );
    }

    #[test]
    fn results_capped_at_max_results() {
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        seed_event(&conn, &event_id);
        for _ in 0..10 {
            let goal_id = new_id_str();
            insert_goal(
                &conn, &goal_id, &event_id, "active", 5, None, "core", "global", None,
            );
        }
        drop(conn);

        let mut req = req_default();
        req.max_results = 3;
        let results = retrieve_active_goals(&db, &req).unwrap();
        assert!(
            results.candidates.len() <= 3,
            "results must be capped at max_results=3, got {}",
            results.candidates.len()
        );
    }

    #[test]
    fn score_rationale_contains_profile() {
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        let goal_id = new_id_str();
        seed_event(&conn, &event_id);
        insert_goal(
            &conn, &goal_id, &event_id, "active", 6, None, "core", "global", None,
        );
        drop(conn);

        let results = retrieve_active_goals(&db, &req_default()).unwrap();
        assert_eq!(results.candidates.len(), 1);
        assert!(
            results.candidates[0].score_rationale.contains(PROFILE),
            "score_rationale must contain profile '{}', got: {}",
            PROFILE,
            results.candidates[0].score_rationale
        );
    }

    // ── Deadline and goal-status transition tests ─────────────────────────────

    #[test]
    fn deadline_expired_returns_partial_flag() {
        // An already-expired deadline sets partial=true on the result.
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        seed_event(&conn, &event_id);
        insert_goal(
            &conn,
            &new_id_str(),
            &event_id,
            "active",
            5,
            None,
            "core",
            "global",
            None,
        );
        drop(conn);

        let deadline = StrategyDeadline::from_millis(0);
        std::thread::sleep(std::time::Duration::from_millis(1));
        let mut req = req_default();
        req.deadline = deadline;
        let results = retrieve_active_goals(&db, &req).unwrap();
        assert!(
            results.partial,
            "result.partial must be true when deadline is already expired"
        );
    }

    #[test]
    fn all_non_active_statuses_contribute_zero() {
        // Candidate, paused, completed, conflicted, stale, superseded, deleted
        // — all must return empty candidates.
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        seed_event(&conn, &event_id);
        for status in &[
            "candidate",
            "paused",
            "completed",
            "conflicted",
            "stale",
            "superseded",
            "deleted",
        ] {
            insert_goal(
                &conn,
                &new_id_str(),
                &event_id,
                status,
                10,
                Some(1.0),
                "core",
                "global",
                None,
            );
        }
        drop(conn);

        let results = retrieve_active_goals(&db, &req_default()).unwrap();
        assert!(
            results.candidates.is_empty(),
            "all non-active statuses must contribute zero — got {} candidates",
            results.candidates.len()
        );
    }

    #[test]
    fn goal_status_transitions_active_to_paused_excluded() {
        // Insert active goal → returned; update to paused → not returned.
        let db = open();
        let conn = db.write();
        let event_id = new_id_str();
        let goal_id = new_id_str();
        seed_event(&conn, &event_id);
        insert_goal(
            &conn, &goal_id, &event_id, "active", 7, None, "core", "global", None,
        );
        drop(conn);

        // First query: goal is active → should be returned.
        let results = retrieve_active_goals(&db, &req_default()).unwrap();
        assert_eq!(
            results.candidates.len(),
            1,
            "active goal must be returned initially"
        );
        assert_eq!(results.candidates[0].goal_id, goal_id);

        // Simulate transition to paused.
        let conn = db.write();
        conn.execute(
            "UPDATE goals_v2 SET status = 'paused' WHERE id = ?1",
            rusqlite::params![goal_id],
        )
        .unwrap();
        drop(conn);

        // Second query: goal is paused → must NOT be returned.
        let results = retrieve_active_goals(&db, &req_default()).unwrap();
        assert!(
            results.candidates.is_empty(),
            "paused goal must not be returned after status transition"
        );
    }
}
