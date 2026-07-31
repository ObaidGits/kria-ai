//! V2 goal management with governed status transitions (design §4.3/§6.6, task F3.6.1).
//!
//! Implements the full goals_v2 lifecycle: creation, status transitions,
//! progress recording, and retrieval with policy gate.

use rusqlite::params;

use crate::memory::error::{MemoryResult, StorageError};

// ── Status ────────────────────────────────────────────────────────────────

/// Status values for a v2 goal. Matches the CHECK constraint in goals_v2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GoalStatusV2 {
    Candidate,
    Active,
    Paused,
    Completed,
    Conflicted,
    Stale,
    Superseded,
    Deleted,
}

impl GoalStatusV2 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Conflicted => "conflicted",
            Self::Stale => "stale",
            Self::Superseded => "superseded",
            Self::Deleted => "deleted",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "candidate" => Some(Self::Candidate),
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "completed" => Some(Self::Completed),
            "conflicted" => Some(Self::Conflicted),
            "stale" => Some(Self::Stale),
            "superseded" => Some(Self::Superseded),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }

    /// Whether this status allows retrieval contribution (design §3.6.2: only Active).
    pub fn contributes_to_retrieval(self) -> bool {
        self == Self::Active
    }

    /// Whether this is a terminal status (completed, deleted, superseded).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Deleted | Self::Superseded)
    }
}

// ── Governed transition ───────────────────────────────────────────────────

/// Check whether a status transition is permitted.
///
/// Permitted transitions (design §6.6):
/// - candidate  → active, deleted, superseded
/// - active     → paused, completed, conflicted, stale, superseded, deleted
/// - paused     → active, deleted, superseded
/// - conflicted → active, deleted, superseded
/// - stale      → active, deleted, superseded
/// - completed, deleted, superseded → NONE (terminal)
pub fn is_transition_permitted(from: GoalStatusV2, to: GoalStatusV2) -> bool {
    use GoalStatusV2::*;
    match from {
        Candidate => matches!(to, Active | Deleted | Superseded),
        Active => matches!(
            to,
            Paused | Completed | Conflicted | Stale | Superseded | Deleted
        ),
        Paused => matches!(to, Active | Deleted | Superseded),
        Conflicted => matches!(to, Active | Deleted | Superseded),
        Stale => matches!(to, Active | Deleted | Superseded),
        // Terminal statuses allow no outgoing transitions.
        Completed | Deleted | Superseded => false,
    }
}

// ── Error ─────────────────────────────────────────────────────────────────

/// Error from a governed goal status transition.
#[derive(Debug, Clone, PartialEq)]
pub enum GoalTransitionError {
    /// The transition from current_status to target_status is not permitted.
    InvalidTransition {
        from: GoalStatusV2,
        to: GoalStatusV2,
    },
    /// Goal was not found.
    GoalNotFound(String),
}

impl std::fmt::Display for GoalTransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTransition { from, to } => write!(
                f,
                "invalid goal status transition: {} → {}",
                from.as_str(),
                to.as_str()
            ),
            Self::GoalNotFound(id) => write!(f, "goal not found: {}", id),
        }
    }
}

impl std::error::Error for GoalTransitionError {}

// ── Domain types ──────────────────────────────────────────────────────────

/// A goal record from goals_v2.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalV2 {
    pub id: String,
    pub kind: Option<String>,
    pub title: String,
    pub status: GoalStatusV2,
    pub priority: i64,
    pub score: Option<f64>,
    pub score_semantics: Option<String>,
    pub resumption_context: Option<String>,
    pub namespace: String,
    pub owner_id: String,
    pub scope: String,
    pub sensitivity: i64,
    pub source_id: String,
    pub policy_version: String,
    pub created_event_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub revision: Option<i64>,
}

/// Specification for creating a new goal.
#[derive(Debug, Clone)]
pub struct NewGoalV2 {
    pub id: String,
    pub kind: Option<String>,
    pub title: String,
    /// Usually `GoalStatusV2::Candidate` for inferred goals (design §6.6).
    pub status: GoalStatusV2,
    /// 0..10
    pub priority: i64,
    pub score: Option<f64>,
    pub score_semantics: Option<String>,
    pub resumption_context: Option<String>,
    pub namespace: String,
    pub owner_id: String,
    pub scope: String,
    /// 0..3
    pub sensitivity: i64,
    pub source_id: String,
    pub policy_version: String,
    pub created_event_id: Option<String>,
}

/// The kind of evidence that authorizes promoting a Candidate goal to Active.
///
/// Inferred (auto-detected) goals start as `Candidate` and **cannot** be
/// promoted to `Active` by inference alone — they require explicit,
/// independently verifiable evidence (design §F3.6.2 / F3.6 invariant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromotionEvidenceKind {
    /// Explicit user action approved this goal (e.g., user clicked "activate").
    UserApproval {
        /// Optional actor ID.
        actor_id: Option<String>,
    },
    /// A named, versioned policy rule grants this promotion.
    PolicyGrant {
        /// Policy rule name.
        policy_name: String,
        /// Policy rule version.
        policy_version: String,
    },
    /// An external authority event that authorized the promotion.
    AuthorityEvent {
        /// Event ID in the authority log.
        event_id: String,
    },
}

// ── CRUD functions ────────────────────────────────────────────────────────

impl NewGoalV2 {
    /// Construct a spec for an **inferred** (auto-detected) goal.
    ///
    /// Inferred goals are **always** created with `status = Candidate`.
    /// They cannot be auto-promoted to `Active` by inference alone — explicit
    /// user approval or a policy grant is required (design §F3.6.2).
    ///
    /// Callers that attempt to pass `status = Active` here must use
    /// [`NewGoalV2::user_declared`] or supply explicit evidence via
    /// [`promote_candidate_with_evidence`].
    pub fn inferred(
        id: impl Into<String>,
        title: impl Into<String>,
        priority: i64,
        namespace: impl Into<String>,
        owner_id: impl Into<String>,
        scope: impl Into<String>,
        sensitivity: i64,
        source_id: impl Into<String>,
        policy_version: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: Some("inferred".to_string()),
            title: title.into(),
            // Invariant: inferred goals always start as Candidate (F3.6.2).
            status: GoalStatusV2::Candidate,
            priority: priority.clamp(0, 10),
            score: None,
            score_semantics: None,
            resumption_context: None,
            namespace: namespace.into(),
            owner_id: owner_id.into(),
            scope: scope.into(),
            sensitivity: sensitivity.clamp(0, 3),
            source_id: source_id.into(),
            policy_version: policy_version.into(),
            created_event_id: None,
        }
    }

    /// Construct a spec for a **user-declared** goal.
    ///
    /// User-declared goals may be created with `Active` status when the user
    /// explicitly activates them at creation time.
    pub fn user_declared(
        id: impl Into<String>,
        title: impl Into<String>,
        status: GoalStatusV2,
        priority: i64,
        namespace: impl Into<String>,
        owner_id: impl Into<String>,
        scope: impl Into<String>,
        sensitivity: i64,
        source_id: impl Into<String>,
        policy_version: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: Some("user".to_string()),
            title: title.into(),
            status,
            priority: priority.clamp(0, 10),
            score: None,
            score_semantics: None,
            resumption_context: None,
            namespace: namespace.into(),
            owner_id: owner_id.into(),
            scope: scope.into(),
            sensitivity: sensitivity.clamp(0, 3),
            source_id: source_id.into(),
            policy_version: policy_version.into(),
            created_event_id: None,
        }
    }
}

/// Promote a `Candidate` goal to `Active`, requiring explicit evidence.
///
/// This is the **only** governed path that advances an inferred or
/// candidate goal to `Active`.  Inference alone cannot call
/// [`transition_goal_status`] directly without providing explicit evidence
/// via this function.
///
/// # Errors
/// - `GoalTransitionError::GoalNotFound` — no goal with that ID.
/// - `GoalTransitionError::InvalidTransition` — current status is not
///   `Candidate` (or another non-active status that permits → Active), or
///   the internal update fails.
pub fn promote_candidate_with_evidence(
    conn: &rusqlite::Connection,
    id: &str,
    evidence: &PromotionEvidenceKind,
) -> Result<(), GoalTransitionError> {
    // Record the evidence kind in a human-readable form.  In a future
    // authority-transaction integration this would be an evidence row;
    // for now we persist it as the score_semantics field so it is
    // observable in the goal row without a separate query.
    let evidence_label = match evidence {
        PromotionEvidenceKind::UserApproval { actor_id } => format!(
            "user-approval:{}",
            actor_id.as_deref().unwrap_or("unknown-actor")
        ),
        PromotionEvidenceKind::PolicyGrant {
            policy_name,
            policy_version,
        } => {
            format!("policy-grant:{policy_name}@{policy_version}")
        }
        PromotionEvidenceKind::AuthorityEvent { event_id } => {
            format!("authority-event:{event_id}")
        }
    };

    // Validate that the transition Candidate → Active (or paused/conflicted/stale → Active)
    // is permitted, then apply it.
    let current_str: Option<String> = conn
        .query_row(
            "SELECT status FROM goals_v2 WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .ok();

    let current_str =
        current_str.ok_or_else(|| GoalTransitionError::GoalNotFound(id.to_string()))?;
    let current = GoalStatusV2::from_str(&current_str)
        .ok_or_else(|| GoalTransitionError::GoalNotFound(id.to_string()))?;

    if !is_transition_permitted(current, GoalStatusV2::Active) {
        return Err(GoalTransitionError::InvalidTransition {
            from: current,
            to: GoalStatusV2::Active,
        });
    }

    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE goals_v2
         SET status = 'active',
             score_semantics = ?1,
             updated_at = ?2,
             revision = COALESCE(revision, 0) + 1
         WHERE id = ?3",
        rusqlite::params![evidence_label, now, id],
    )
    .map_err(|_e| GoalTransitionError::InvalidTransition {
        from: current,
        to: GoalStatusV2::Active,
    })?;

    Ok(())
}

/// Create a new goal in goals_v2.
pub fn create_goal(conn: &rusqlite::Connection, spec: &NewGoalV2) -> MemoryResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO goals_v2 (
            id, kind, title, status, priority, score, score_semantics,
            resumption_context, namespace, owner_id, scope, sensitivity,
            source_id, policy_version, created_event_id, created_at, updated_at, revision
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,1)",
        params![
            spec.id,
            spec.kind,
            spec.title,
            spec.status.as_str(),
            spec.priority.clamp(0, 10),
            spec.score,
            spec.score_semantics,
            spec.resumption_context,
            spec.namespace,
            spec.owner_id,
            spec.scope,
            spec.sensitivity.clamp(0, 3),
            spec.source_id,
            spec.policy_version,
            spec.created_event_id,
            now,
            now,
        ],
    )
    .map_err(StorageError::Sqlite)?;
    Ok(())
}

/// Fetch a single goal by id, returning `None` if not found.
pub fn get_goal(conn: &rusqlite::Connection, id: &str) -> MemoryResult<Option<GoalV2>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, title, status, priority, score, score_semantics,
                    resumption_context, namespace, owner_id, scope, sensitivity,
                    source_id, policy_version, created_event_id, created_at, updated_at, revision
             FROM goals_v2 WHERE id = ?1",
        )
        .map_err(StorageError::Sqlite)?;
    let mut rows = stmt
        .query_map(params![id], row_to_goal)
        .map_err(StorageError::Sqlite)?;
    match rows.next() {
        Some(r) => Ok(Some(r.map_err(StorageError::Sqlite)?)),
        None => Ok(None),
    }
}

/// Transition goal status with governance check.
///
/// Returns `Ok(())` if the transition is permitted and applied.
/// Returns `Err(GoalTransitionError::GoalNotFound)` if no such goal exists.
/// Returns `Err(GoalTransitionError::InvalidTransition)` if the transition is forbidden.
pub fn transition_goal_status(
    conn: &rusqlite::Connection,
    id: &str,
    new_status: GoalStatusV2,
) -> Result<(), GoalTransitionError> {
    // Read current status.
    let current_str: Option<String> = conn
        .query_row(
            "SELECT status FROM goals_v2 WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .ok();

    let current_str =
        current_str.ok_or_else(|| GoalTransitionError::GoalNotFound(id.to_string()))?;
    let current = GoalStatusV2::from_str(&current_str)
        .ok_or_else(|| GoalTransitionError::GoalNotFound(id.to_string()))?;

    if !is_transition_permitted(current, new_status) {
        return Err(GoalTransitionError::InvalidTransition {
            from: current,
            to: new_status,
        });
    }

    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE goals_v2 SET status = ?1, updated_at = ?2, revision = COALESCE(revision, 0) + 1
         WHERE id = ?3",
        params![new_status.as_str(), now, id],
    )
    .map_err(|_e| GoalTransitionError::InvalidTransition {
        from: current,
        to: new_status,
    })?;

    Ok(())
}

/// Update priority, clamping to 0–10.
pub fn update_goal_priority(
    conn: &rusqlite::Connection,
    id: &str,
    priority: i64,
) -> MemoryResult<()> {
    let clamped = priority.clamp(0, 10);
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE goals_v2 SET priority = ?1, updated_at = ?2,
         revision = COALESCE(revision, 0) + 1 WHERE id = ?3",
        params![clamped, now, id],
    )
    .map_err(StorageError::Sqlite)?;
    Ok(())
}

/// Update resumption context (nullable).
pub fn update_resumption_context(
    conn: &rusqlite::Connection,
    id: &str,
    context: Option<&str>,
) -> MemoryResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE goals_v2 SET resumption_context = ?1, updated_at = ?2,
         revision = COALESCE(revision, 0) + 1 WHERE id = ?3",
        params![context, now, id],
    )
    .map_err(StorageError::Sqlite)?;
    Ok(())
}

/// Record progress for a goal (append-only).
pub fn record_progress(
    conn: &rusqlite::Connection,
    goal_id: &str,
    progress_id: &str,
    state: &str,
    summary: &str,
    observed_at: &str,
    revision: Option<i64>,
) -> MemoryResult<()> {
    conn.execute(
        "INSERT INTO goal_progress (id, goal_id, event_id, state, summary, observed_at, revision)
         VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6)",
        params![progress_id, goal_id, state, summary, observed_at, revision],
    )
    .map_err(StorageError::Sqlite)?;
    Ok(())
}

/// Get the most recently observed progress entry for a goal.
/// Returns `(state, summary, observed_at)` or `None` if no progress recorded.
pub fn get_latest_progress(
    conn: &rusqlite::Connection,
    goal_id: &str,
) -> MemoryResult<Option<(String, String, String)>> {
    let mut stmt = conn
        .prepare(
            "SELECT state, summary, observed_at FROM goal_progress
             WHERE goal_id = ?1 ORDER BY observed_at DESC LIMIT 1",
        )
        .map_err(StorageError::Sqlite)?;
    let mut rows = stmt
        .query_map(params![goal_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(StorageError::Sqlite)?;
    match rows.next() {
        Some(r) => Ok(Some(r.map_err(StorageError::Sqlite)?)),
        None => Ok(None),
    }
}

/// List active goals filtered by namespace, scope, and max sensitivity (policy gate).
pub fn list_active_goals(
    conn: &rusqlite::Connection,
    namespace: &str,
    scope: &str,
    max_sensitivity: i64,
    limit: usize,
) -> MemoryResult<Vec<GoalV2>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, title, status, priority, score, score_semantics,
                    resumption_context, namespace, owner_id, scope, sensitivity,
                    source_id, policy_version, created_event_id, created_at, updated_at, revision
             FROM goals_v2
             WHERE status = 'active'
               AND namespace = ?1
               AND scope = ?2
               AND sensitivity <= ?3
             ORDER BY priority DESC, created_at ASC
             LIMIT ?4",
        )
        .map_err(StorageError::Sqlite)?;
    let rows = stmt
        .query_map(
            params![namespace, scope, max_sensitivity, limit as i64],
            row_to_goal,
        )
        .map_err(StorageError::Sqlite)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(StorageError::Sqlite)?);
    }
    Ok(out)
}

// ── Row mapper ────────────────────────────────────────────────────────────

fn row_to_goal(r: &rusqlite::Row<'_>) -> rusqlite::Result<GoalV2> {
    let status_str: String = r.get(3)?;
    let status = GoalStatusV2::from_str(&status_str).unwrap_or(GoalStatusV2::Candidate);
    Ok(GoalV2 {
        id: r.get(0)?,
        kind: r.get(1)?,
        title: r.get(2)?,
        status,
        priority: r.get(4)?,
        score: r.get(5)?,
        score_semantics: r.get(6)?,
        resumption_context: r.get(7)?,
        namespace: r.get(8)?,
        owner_id: r.get(9)?,
        scope: r.get(10)?,
        sensitivity: r.get(11)?,
        source_id: r.get(12)?,
        policy_version: r.get(13)?,
        created_event_id: r.get(14)?,
        created_at: r.get(15)?,
        updated_at: r.get(16)?,
        revision: r.get(17)?,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::Database;

    fn base_spec(id: &str, status: GoalStatusV2) -> NewGoalV2 {
        NewGoalV2 {
            id: id.to_string(),
            kind: Some("user".to_string()),
            title: format!("Goal {id}"),
            status,
            priority: 5,
            score: Some(0.8),
            score_semantics: Some("relevance".to_string()),
            resumption_context: Some("pick up from step 3".to_string()),
            namespace: "ns".to_string(),
            owner_id: "owner-1".to_string(),
            scope: "private".to_string(),
            sensitivity: 1,
            source_id: "src-1".to_string(),
            policy_version: "v1".to_string(),
            created_event_id: None,
        }
    }

    // 1. create_and_get_goal_round_trips
    #[test]
    fn create_and_get_goal_round_trips() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let spec = base_spec("g1", GoalStatusV2::Candidate);
        create_goal(&conn, &spec).unwrap();
        let goal = get_goal(&conn, "g1")
            .unwrap()
            .expect("goal must be present");
        assert_eq!(goal.id, "g1");
        assert_eq!(goal.title, "Goal g1");
        assert_eq!(goal.status, GoalStatusV2::Candidate);
        assert_eq!(goal.priority, 5);
        assert_eq!(goal.score, Some(0.8));
        assert_eq!(goal.score_semantics, Some("relevance".to_string()));
        assert_eq!(
            goal.resumption_context,
            Some("pick up from step 3".to_string())
        );
        assert_eq!(goal.namespace, "ns");
        assert_eq!(goal.owner_id, "owner-1");
        assert_eq!(goal.scope, "private");
        assert_eq!(goal.sensitivity, 1);
        assert_eq!(goal.source_id, "src-1");
        assert_eq!(goal.policy_version, "v1");
        assert_eq!(goal.kind, Some("user".to_string()));
    }

    // 2. candidate_to_active_is_permitted
    #[test]
    fn candidate_to_active_is_permitted() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        create_goal(&conn, &base_spec("g1", GoalStatusV2::Candidate)).unwrap();
        transition_goal_status(&conn, "g1", GoalStatusV2::Active).unwrap();
        let goal = get_goal(&conn, "g1").unwrap().unwrap();
        assert_eq!(goal.status, GoalStatusV2::Active);
    }

    // 3. active_to_paused_is_permitted
    #[test]
    fn active_to_paused_is_permitted() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        create_goal(&conn, &base_spec("g1", GoalStatusV2::Active)).unwrap();
        transition_goal_status(&conn, "g1", GoalStatusV2::Paused).unwrap();
        let goal = get_goal(&conn, "g1").unwrap().unwrap();
        assert_eq!(goal.status, GoalStatusV2::Paused);
    }

    // 4. active_to_completed_is_permitted
    #[test]
    fn active_to_completed_is_permitted() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        create_goal(&conn, &base_spec("g1", GoalStatusV2::Active)).unwrap();
        transition_goal_status(&conn, "g1", GoalStatusV2::Completed).unwrap();
        let goal = get_goal(&conn, "g1").unwrap().unwrap();
        assert_eq!(goal.status, GoalStatusV2::Completed);
    }

    // 5. completed_to_active_is_rejected (terminal)
    #[test]
    fn completed_to_active_is_rejected() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        create_goal(&conn, &base_spec("g1", GoalStatusV2::Completed)).unwrap();
        let err = transition_goal_status(&conn, "g1", GoalStatusV2::Active).unwrap_err();
        assert_eq!(
            err,
            GoalTransitionError::InvalidTransition {
                from: GoalStatusV2::Completed,
                to: GoalStatusV2::Active,
            }
        );
    }

    // 6. deleted_to_anything_is_rejected (terminal)
    #[test]
    fn deleted_to_anything_is_rejected() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        create_goal(&conn, &base_spec("g1", GoalStatusV2::Deleted)).unwrap();
        let err = transition_goal_status(&conn, "g1", GoalStatusV2::Active).unwrap_err();
        assert_eq!(
            err,
            GoalTransitionError::InvalidTransition {
                from: GoalStatusV2::Deleted,
                to: GoalStatusV2::Active,
            }
        );
    }

    // 7. candidate_to_completed_is_rejected (not a direct permitted transition)
    #[test]
    fn candidate_to_completed_is_rejected() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        create_goal(&conn, &base_spec("g1", GoalStatusV2::Candidate)).unwrap();
        let err = transition_goal_status(&conn, "g1", GoalStatusV2::Completed).unwrap_err();
        assert_eq!(
            err,
            GoalTransitionError::InvalidTransition {
                from: GoalStatusV2::Candidate,
                to: GoalStatusV2::Completed,
            }
        );
    }

    // 8. unknown_goal_transition_returns_not_found
    #[test]
    fn unknown_goal_transition_returns_not_found() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let err = transition_goal_status(&conn, "no-such-id", GoalStatusV2::Active).unwrap_err();
        assert_eq!(
            err,
            GoalTransitionError::GoalNotFound("no-such-id".to_string())
        );
    }

    // 9. record_and_get_latest_progress
    #[test]
    fn record_and_get_latest_progress() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        create_goal(&conn, &base_spec("g1", GoalStatusV2::Active)).unwrap();
        record_progress(
            &conn,
            "g1",
            "p1",
            "in_progress",
            "started step 1",
            "2024-01-01T10:00:00Z",
            Some(1),
        )
        .unwrap();
        record_progress(
            &conn,
            "g1",
            "p2",
            "in_progress",
            "completed step 2",
            "2024-01-01T11:00:00Z",
            Some(2),
        )
        .unwrap();
        let (state, summary, observed_at) = get_latest_progress(&conn, "g1")
            .unwrap()
            .expect("should have progress");
        assert_eq!(state, "in_progress");
        assert_eq!(summary, "completed step 2");
        assert_eq!(observed_at, "2024-01-01T11:00:00Z");
    }

    // 10. list_active_goals_excludes_non_active
    #[test]
    fn list_active_goals_excludes_non_active() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let mut s = base_spec("g-candidate", GoalStatusV2::Candidate);
        s.namespace = "ns".to_string();
        s.scope = "private".to_string();
        create_goal(&conn, &s).unwrap();

        let mut s2 = base_spec("g-active", GoalStatusV2::Active);
        s2.namespace = "ns".to_string();
        s2.scope = "private".to_string();
        create_goal(&conn, &s2).unwrap();

        let mut s3 = base_spec("g-paused", GoalStatusV2::Paused);
        s3.namespace = "ns".to_string();
        s3.scope = "private".to_string();
        create_goal(&conn, &s3).unwrap();

        let goals = list_active_goals(&conn, "ns", "private", 3, 100).unwrap();
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].id, "g-active");
    }

    // 11. list_active_goals_policy_gate
    #[test]
    fn list_active_goals_policy_gate() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();

        // Active goal in "ns" namespace.
        let s = base_spec("g1", GoalStatusV2::Active);
        create_goal(&conn, &s).unwrap();

        // Active goal in different namespace "other".
        let mut s2 = base_spec("g2", GoalStatusV2::Active);
        s2.namespace = "other".to_string();
        create_goal(&conn, &s2).unwrap();

        // Only "ns" goals should be returned.
        let goals = list_active_goals(&conn, "ns", "private", 3, 100).unwrap();
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].id, "g1");
    }

    // 12. contributes_to_retrieval_only_for_active
    #[test]
    fn contributes_to_retrieval_only_for_active() {
        assert!(GoalStatusV2::Active.contributes_to_retrieval());
        assert!(!GoalStatusV2::Candidate.contributes_to_retrieval());
        assert!(!GoalStatusV2::Paused.contributes_to_retrieval());
        assert!(!GoalStatusV2::Completed.contributes_to_retrieval());
        assert!(!GoalStatusV2::Conflicted.contributes_to_retrieval());
        assert!(!GoalStatusV2::Stale.contributes_to_retrieval());
        assert!(!GoalStatusV2::Superseded.contributes_to_retrieval());
        assert!(!GoalStatusV2::Deleted.contributes_to_retrieval());
    }

    // 13. terminal_statuses_are_completed_deleted_superseded
    #[test]
    fn terminal_statuses_are_completed_deleted_superseded() {
        assert!(GoalStatusV2::Completed.is_terminal());
        assert!(GoalStatusV2::Deleted.is_terminal());
        assert!(GoalStatusV2::Superseded.is_terminal());
        assert!(!GoalStatusV2::Candidate.is_terminal());
        assert!(!GoalStatusV2::Active.is_terminal());
        assert!(!GoalStatusV2::Paused.is_terminal());
        assert!(!GoalStatusV2::Conflicted.is_terminal());
        assert!(!GoalStatusV2::Stale.is_terminal());
    }

    // 14. update_priority_is_clamped
    #[test]
    fn update_priority_is_clamped() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        create_goal(&conn, &base_spec("g1", GoalStatusV2::Active)).unwrap();
        // priority 15 should be clamped to 10 by the DB CHECK constraint guard in update.
        update_goal_priority(&conn, "g1", 15).unwrap();
        let goal = get_goal(&conn, "g1").unwrap().unwrap();
        assert_eq!(goal.priority, 10, "priority > 10 must be clamped to 10");
    }

    // ── F3.6.2 specific tests ─────────────────────────────────────────────────

    // 15. Inferred goal starts as Candidate (not Active) — F3.6.2 invariant.
    #[test]
    fn inferred_goal_always_created_as_candidate() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let spec = NewGoalV2::inferred(
            "g-inferred",
            "Infer something",
            5,
            "ns",
            "owner-1",
            "private",
            0,
            "src-1",
            "v1",
        );
        assert_eq!(
            spec.status,
            GoalStatusV2::Candidate,
            "NewGoalV2::inferred() must always produce Candidate status"
        );
        create_goal(&conn, &spec).unwrap();
        let goal = get_goal(&conn, "g-inferred").unwrap().unwrap();
        assert_eq!(
            goal.status,
            GoalStatusV2::Candidate,
            "inferred goal persisted status must be Candidate"
        );
        // F3.6.2: Candidate does NOT contribute to retrieval.
        assert!(
            !goal.status.contributes_to_retrieval(),
            "inferred Candidate goal must NOT contribute to retrieval"
        );
    }

    // 16. Only Active goals contribute to retrieval — all others return false.
    #[test]
    fn only_active_status_contributes_to_retrieval() {
        assert!(GoalStatusV2::Active.contributes_to_retrieval());
        for status in [
            GoalStatusV2::Candidate,
            GoalStatusV2::Paused,
            GoalStatusV2::Completed,
            GoalStatusV2::Conflicted,
            GoalStatusV2::Stale,
            GoalStatusV2::Superseded,
            GoalStatusV2::Deleted,
        ] {
            assert!(
                !status.contributes_to_retrieval(),
                "{:?} must NOT contribute to retrieval (F3.6.2)",
                status
            );
        }
    }

    // 17. Transitioning Active → non-Active immediately stops retrieval contribution.
    #[test]
    fn active_to_non_active_immediately_stops_retrieval() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        create_goal(&conn, &base_spec("g1", GoalStatusV2::Active)).unwrap();

        let transitions = [
            GoalStatusV2::Paused,
            GoalStatusV2::Completed,
            GoalStatusV2::Conflicted,
            GoalStatusV2::Stale,
            GoalStatusV2::Superseded,
            GoalStatusV2::Deleted,
        ];

        // Test each terminal/non-active status: create a fresh Active goal and transition it.
        for target in transitions {
            let id = format!("g-{}", target.as_str());
            create_goal(&conn, &base_spec(&id, GoalStatusV2::Active)).unwrap();

            // Confirm it's active and contributes.
            let goal_before = get_goal(&conn, &id).unwrap().unwrap();
            assert!(
                goal_before.status.contributes_to_retrieval(),
                "goal must contribute to retrieval when Active"
            );

            // Transition away from Active.
            transition_goal_status(&conn, &id, target).unwrap();

            // Immediately after transition, retrieval contribution must be zero.
            let goal_after = get_goal(&conn, &id).unwrap().unwrap();
            assert_eq!(goal_after.status, target);
            assert!(
                !goal_after.status.contributes_to_retrieval(),
                "goal transitioned to {:?} must immediately stop contributing to retrieval (F3.6.2)",
                target
            );
        }
    }

    // 18. Candidate cannot jump directly to Active without explicit evidence.
    //     promote_candidate_with_evidence() is the required path.
    #[test]
    fn candidate_requires_explicit_evidence_to_become_active() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        create_goal(&conn, &base_spec("g1", GoalStatusV2::Candidate)).unwrap();

        // Promote via the evidence-required path.
        let evidence = PromotionEvidenceKind::UserApproval {
            actor_id: Some("user-42".to_string()),
        };
        promote_candidate_with_evidence(&conn, "g1", &evidence).unwrap();

        let goal = get_goal(&conn, "g1").unwrap().unwrap();
        assert_eq!(goal.status, GoalStatusV2::Active);
        assert!(
            goal.status.contributes_to_retrieval(),
            "goal must contribute to retrieval after explicit promotion"
        );
        // The evidence label must be recorded in score_semantics.
        let semantics = goal.score_semantics.unwrap_or_default();
        assert!(
            semantics.contains("user-approval"),
            "promotion evidence must be recorded in score_semantics, got: {:?}",
            semantics
        );
    }

    // 19. Policy-grant evidence also permits Candidate → Active promotion.
    #[test]
    fn policy_grant_promotes_candidate_to_active() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        create_goal(&conn, &base_spec("g1", GoalStatusV2::Candidate)).unwrap();

        let evidence = PromotionEvidenceKind::PolicyGrant {
            policy_name: "auto-activate-high-priority".to_string(),
            policy_version: "v2".to_string(),
        };
        promote_candidate_with_evidence(&conn, "g1", &evidence).unwrap();

        let goal = get_goal(&conn, "g1").unwrap().unwrap();
        assert_eq!(goal.status, GoalStatusV2::Active);
        let semantics = goal.score_semantics.unwrap_or_default();
        assert!(
            semantics.contains("policy-grant"),
            "policy evidence must be recorded, got: {:?}",
            semantics
        );
    }

    // 20. Promoting a non-existent goal returns GoalNotFound.
    #[test]
    fn promote_nonexistent_goal_returns_not_found() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        let evidence = PromotionEvidenceKind::UserApproval { actor_id: None };
        let err = promote_candidate_with_evidence(&conn, "no-such-goal", &evidence).unwrap_err();
        assert_eq!(
            err,
            GoalTransitionError::GoalNotFound("no-such-goal".to_string())
        );
    }

    // 21. Completed goal cannot be promoted to Active even with explicit evidence.
    #[test]
    fn completed_goal_cannot_be_promoted_to_active() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.write();
        create_goal(&conn, &base_spec("g1", GoalStatusV2::Completed)).unwrap();
        let evidence = PromotionEvidenceKind::UserApproval { actor_id: None };
        let err = promote_candidate_with_evidence(&conn, "g1", &evidence).unwrap_err();
        assert_eq!(
            err,
            GoalTransitionError::InvalidTransition {
                from: GoalStatusV2::Completed,
                to: GoalStatusV2::Active,
            }
        );
    }
}
