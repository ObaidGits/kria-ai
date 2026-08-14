//! Bounded episode management and versioned consolidation candidate selection
//! (design §4.3/§7.3, task F3.6.3).
//!
//! Implements:
//! * **Episode lifecycle**: open → record accumulation → close with boundary reason.
//!   Each episode has a configurable maximum record count cap; closure writes the
//!   boundary reason and advances the consolidation cursor.
//! * **Durable consolidation cursor**: tracks which `records` in a closed episode
//!   have been considered for consolidation.  The cursor is resumable — it
//!   survives crashes and scheduler cancellation.
//! * **Versioned consolidation candidate selection**: given a closed episode,
//!   produces a `ConsolidationCandidateSet` that uniquely identifies the run by
//!   `(algorithm, version, input_set_hash, level)` so a retry with the same inputs
//!   returns the existing run rather than creating a duplicate (idempotency).
//!   Selection is bounded by a configurable page size, respects the scheduler's
//!   resource policy (battery / memory pressure gate), and honours
//!   `CancellationToken` between pages so a P4 job yields promptly.

use std::sync::Arc;

use rusqlite::params;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::db::Database;
use crate::error::{MemoryResult, StorageError};
use crate::ids::new_id;
use crate::scheduler::{Priority, ResourceMonitor};

// ── Constants ────────────────────────────────────────────────────────────────

/// Default maximum record count per episode (design §4.3 / A6 boundedness).
pub const DEFAULT_EPISODE_MAX_RECORDS: usize = 500;

/// Default page size for consolidation candidate selection.
/// Bounded so a single pass never reads unbounded rows off the Tokio executor.
pub const DEFAULT_CANDIDATE_PAGE_SIZE: usize = 50;

/// Algorithm label used by the default consolidation selector.
pub const CONSOLIDATION_ALGORITHM: &str = "episode_selector";

/// Algorithm version.  Bump this when the selection logic changes so that the
/// `(algorithm, version, input_set_hash, level)` uniqueness key invalidates
/// old runs naturally (design §7.3).
pub const CONSOLIDATION_ALGORITHM_VERSION: &str = "1";

// ── Episode boundary reason ───────────────────────────────────────────────────

/// Reason code recorded on the `episodes_v2` row when an episode is closed.
/// Maps to `boundary_reason` (design §4.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpisodeBoundaryReason {
    /// The user's session ended normally.
    SessionEnd,
    /// A named task was completed.
    TaskCompletion,
    /// The episode exceeded its time limit.
    TimeLimit,
    /// The episode reached its maximum record count cap (design A6).
    RecordCountLimit,
    /// Explicit administrative close.
    Manual,
}

impl EpisodeBoundaryReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionEnd => "session_end",
            Self::TaskCompletion => "task_completion",
            Self::TimeLimit => "time_limit",
            Self::RecordCountLimit => "record_count_limit",
            Self::Manual => "manual",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "session_end" => Some(Self::SessionEnd),
            "task_completion" => Some(Self::TaskCompletion),
            "time_limit" => Some(Self::TimeLimit),
            "record_count_limit" => Some(Self::RecordCountLimit),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }
}

// ── Episode record ────────────────────────────────────────────────────────────

/// A lightweight view of an `episodes_v2` row.
#[derive(Clone, Debug, PartialEq)]
pub struct EpisodeV2 {
    pub id: String,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub namespace: String,
    pub owner_id: String,
    pub scope: String,
    pub sensitivity: i64,
    pub source_id: String,
    pub policy_version: String,
    pub opened_at: Option<String>,
    pub closed_at: Option<String>,
    pub boundary_reason: Option<String>,
    /// Last `records.id` included in a consolidation consideration; NULL = none yet.
    pub cursor_event_id: Option<String>,
    pub truth_state: Option<String>,
    pub revision: Option<i64>,
}

/// Spec for opening a new episode.
#[derive(Clone, Debug)]
pub struct NewEpisode {
    pub id: String,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub namespace: String,
    pub owner_id: String,
    pub scope: String,
    pub sensitivity: i64,
    pub source_id: String,
    pub policy_version: String,
}

impl NewEpisode {
    /// Convenience constructor for tests / non-policy contexts.
    pub fn simple(
        session_id: impl Into<String>,
        namespace: impl Into<String>,
        owner_id: impl Into<String>,
    ) -> Self {
        Self {
            id: new_id().to_string(),
            session_id: Some(session_id.into()),
            task_id: None,
            namespace: namespace.into(),
            owner_id: owner_id.into(),
            scope: "private".into(),
            sensitivity: 0,
            source_id: "system".into(),
            policy_version: "v1".into(),
        }
    }
}

// ── Consolidation candidate set ───────────────────────────────────────────────

/// Combined effective policy across all candidate records.
/// Sensitivity uses the most-restrictive (maximum) value across contributors
/// (design §7.3: "restrictive Effective Policy").
#[derive(Clone, Debug, PartialEq)]
pub struct EffectivePolicy {
    pub namespace: String,
    pub owner_id: String,
    pub scope: String,
    /// Most-restrictive contributing sensitivity (0..3).
    pub sensitivity: i64,
    pub source_id: String,
    pub policy_version: String,
}

/// The result of a versioned consolidation candidate selection pass.
///
/// A `ConsolidationCandidateSet` uniquely identifies a consolidation run via the
/// `(algorithm, version, input_set_hash, level)` quadruplet so that re-running
/// with the same inputs is idempotent (design §7.3 / `consolidation_runs` PK).
#[derive(Clone, Debug, PartialEq)]
pub struct ConsolidationCandidateSet {
    /// The ID of the `consolidation_runs` row that owns this selection.
    pub run_id: String,
    /// Lexicographically sorted record IDs selected for consolidation.
    pub record_ids: Vec<String>,
    /// Combined effective policy (most-restrictive across all records).
    pub effective_policy: EffectivePolicy,
    /// SHA-256 hex of the sorted record IDs — forms part of the identity key.
    pub input_set_hash: String,
    /// Compression level this set targets.
    pub level: ConsolidationLevel,
    /// Episode this set was selected from.
    pub episode_id: String,
}

/// The compression level of a consolidation run (design §7.3 Episode→Summary→Skill→Rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsolidationLevel {
    Episode,
    Summary,
    Skill,
    Rule,
}

impl ConsolidationLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Episode => "episode",
            Self::Summary => "summary",
            Self::Skill => "skill",
            Self::Rule => "rule",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "episode" => Some(Self::Episode),
            "summary" => Some(Self::Summary),
            "skill" => Some(Self::Skill),
            "rule" => Some(Self::Rule),
            _ => None,
        }
    }
}

// ── Episode manager ───────────────────────────────────────────────────────────

/// Error from episode or consolidation operations.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EpisodeError {
    #[error("episode not found: {0}")]
    NotFound(String),
    #[error("episode is already closed: {0}")]
    AlreadyClosed(String),
    #[error("episode is still open (not closed): {0}")]
    StillOpen(String),
    #[error("consolidation run already exists for identity key")]
    DuplicateRun {
        /// The existing run ID.
        existing_run_id: String,
    },
    #[error("scheduler policy denied: resource constraints active")]
    ResourcePolicyDenied,
    #[error("selection cancelled by scheduler")]
    Cancelled,
}

/// Configuration for episode and consolidation behaviour.
#[derive(Clone, Debug)]
pub struct EpisodeConfig {
    /// Maximum number of records an episode may contain before it is
    /// automatically closed with `RecordCountLimit` (design A6).
    pub max_records_per_episode: usize,
    /// Maximum number of candidate records returned per selection page.
    pub candidate_page_size: usize,
    /// Minimum scheduler priority required to run a candidate selection.
    /// Selection is P4Maintenance; it is suspended under battery / memory
    /// pressure unless the caller overrides this gate.
    pub required_priority: Priority,
}

impl Default for EpisodeConfig {
    fn default() -> Self {
        Self {
            max_records_per_episode: DEFAULT_EPISODE_MAX_RECORDS,
            candidate_page_size: DEFAULT_CANDIDATE_PAGE_SIZE,
            required_priority: Priority::P4Maintenance,
        }
    }
}

/// Bounded episode lifecycle manager and consolidation cursor.
pub struct EpisodeManager {
    db: Arc<Database>,
    config: EpisodeConfig,
}

impl EpisodeManager {
    pub fn new(db: Arc<Database>, config: EpisodeConfig) -> Self {
        Self { db, config }
    }

    pub fn with_defaults(db: Arc<Database>) -> Self {
        Self::new(db, EpisodeConfig::default())
    }

    // ── Open ─────────────────────────────────────────────────────────────────

    /// Open a new episode, recording `opened_at`.  Returns the episode ID.
    pub fn open_episode(&self, spec: &NewEpisode) -> MemoryResult<String> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.db.write();
        conn.execute(
            "INSERT INTO episodes_v2 (
                 id, session_id, task_id,
                 namespace, owner_id, scope, sensitivity, source_id, policy_version,
                 opened_at, closed_at, boundary_reason, cursor_event_id,
                 truth_state, revision
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,NULL,NULL,NULL,'current',1)",
            params![
                spec.id,
                spec.session_id,
                spec.task_id,
                spec.namespace,
                spec.owner_id,
                spec.scope,
                spec.sensitivity.clamp(0, 3),
                spec.source_id,
                spec.policy_version,
                now
            ],
        )
        .map_err(StorageError::Sqlite)?;
        Ok(spec.id.clone())
    }

    // ── Record count ─────────────────────────────────────────────────────────

    /// Count the `records` rows that reference this episode.
    pub fn record_count(&self, episode_id: &str) -> MemoryResult<usize> {
        self.db.with_read(|conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM records WHERE episode_id = ?1",
                    params![episode_id],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            Ok(count as usize)
        })
    }

    // ── Close ────────────────────────────────────────────────────────────────

    /// Close an episode with a boundary reason.  Returns `EpisodeError::AlreadyClosed`
    /// if the episode is already closed. On success advances the consolidation
    /// cursor to the last `records.id` lexicographically (so a subsequent
    /// candidate selection pass knows where to resume from).
    ///
    /// If `force_reason` is `None`, the method auto-detects `RecordCountLimit`
    /// when the record count has hit the configured cap; otherwise the provided
    /// reason is used.
    pub fn close_episode(
        &self,
        episode_id: &str,
        force_reason: Option<EpisodeBoundaryReason>,
    ) -> MemoryResult<()> {
        let now = chrono::Utc::now().to_rfc3339();

        // Verify episode exists and is open.
        let ep = self.get_episode(episode_id)?.ok_or_else(|| {
            crate::error::MemoryError::Internal(
                EpisodeError::NotFound(episode_id.to_string()).to_string(),
            )
        })?;
        if ep.closed_at.is_some() {
            return Err(crate::error::MemoryError::Internal(
                EpisodeError::AlreadyClosed(episode_id.to_string()).to_string(),
            ));
        }

        // Determine boundary reason.
        let reason = if let Some(r) = force_reason {
            r
        } else {
            let count = self.record_count(episode_id)?;
            if count >= self.config.max_records_per_episode {
                EpisodeBoundaryReason::RecordCountLimit
            } else {
                EpisodeBoundaryReason::Manual
            }
        };

        // Find the last record's created_event_id for this episode (cursor advance).
        // This is an events_v2 ID which satisfies the FK constraint on cursor_event_id.
        let cursor_id: Option<String> = self.db.with_read(|conn| {
            Ok(conn
                .query_row(
                    "SELECT created_event_id FROM records WHERE episode_id = ?1
                 ORDER BY id DESC LIMIT 1",
                    params![episode_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StorageError::Sqlite)?)
        })?;

        let conn = self.db.write();
        conn.execute(
            "UPDATE episodes_v2
             SET closed_at = ?1,
                 boundary_reason = ?2,
                 cursor_event_id = ?3,
                 revision = COALESCE(revision, 1) + 1
             WHERE id = ?4",
            params![now, reason.as_str(), cursor_id, episode_id],
        )
        .map_err(StorageError::Sqlite)?;
        Ok(())
    }

    // ── Get ──────────────────────────────────────────────────────────────────

    /// Fetch a single episode by ID.
    pub fn get_episode(&self, id: &str) -> MemoryResult<Option<EpisodeV2>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, task_id,
                            namespace, owner_id, scope, sensitivity, source_id, policy_version,
                            opened_at, closed_at, boundary_reason, cursor_event_id,
                            truth_state, revision
                     FROM episodes_v2 WHERE id = ?1",
                )
                .map_err(StorageError::Sqlite)?;
            let mut rows = stmt
                .query_map(params![id], row_to_episode)
                .map_err(StorageError::Sqlite)?;
            match rows.next() {
                Some(r) => Ok(Some(r.map_err(StorageError::Sqlite)?)),
                None => Ok(None),
            }
        })
    }

    // ── Auto-close when capped ────────────────────────────────────────────────

    /// After inserting a record into an episode, call this to auto-close the
    /// episode if it has reached the record count cap.  Returns `true` if the
    /// episode was auto-closed.
    pub fn auto_close_if_capped(&self, episode_id: &str) -> MemoryResult<bool> {
        let count = self.record_count(episode_id)?;
        if count >= self.config.max_records_per_episode {
            self.close_episode(episode_id, Some(EpisodeBoundaryReason::RecordCountLimit))?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ── Versioned candidate selection ─────────────────────────────────────────

    /// Select consolidation candidates for a **closed** episode, returning a
    /// `ConsolidationCandidateSet`.
    ///
    /// # Scheduler / resource policy gate
    /// The caller must pass `monitor`.  If `monitor.on_battery()` or
    /// `monitor.memory_pressure()` is true, selection is refused with
    /// `EpisodeError::ResourcePolicyDenied` unless the caller has set
    /// `config.required_priority` to `P3Cognition` or higher.  The default
    /// `required_priority` is `P4Maintenance`, which is suspended under
    /// pressure.
    ///
    /// # Idempotency
    /// If a `consolidation_runs` row with the same
    /// `(algorithm, version, input_set_hash, level)` already exists, the
    /// function returns `EpisodeError::DuplicateRun` with the existing run ID
    /// so the caller can reuse it.
    ///
    /// # Cancellation
    /// The selection is split into pages of `config.candidate_page_size`.
    /// After each page `cancel.is_cancelled()` is checked; if cancelled the
    /// cursor position is left at the last page boundary so work can resume.
    pub async fn select_candidates(
        &self,
        episode_id: &str,
        level: ConsolidationLevel,
        monitor: &dyn ResourceMonitor,
        cancel: &CancellationToken,
    ) -> Result<ConsolidationCandidateSet, EpisodeError> {
        // Resource policy gate.
        let ceiling = if monitor.on_battery() || monitor.memory_pressure() {
            Priority::P2Enrichment
        } else {
            Priority::P4Maintenance
        };
        if self.config.required_priority > ceiling {
            return Err(EpisodeError::ResourcePolicyDenied);
        }

        // Verify episode is closed.
        let ep = self
            .get_episode(episode_id)
            .map_err(|e| EpisodeError::NotFound(e.to_string()))?
            .ok_or_else(|| EpisodeError::NotFound(episode_id.to_string()))?;
        if ep.closed_at.is_none() {
            return Err(EpisodeError::StillOpen(episode_id.to_string()));
        }

        // Page through records for this episode lexicographically.
        let page_size = self.config.candidate_page_size as i64;
        let mut all_ids: Vec<String> = Vec::new();
        let mut cursor: Option<String> = None; // last seen id for keyset pagination

        loop {
            if cancel.is_cancelled() {
                return Err(EpisodeError::Cancelled);
            }

            let page: Vec<(String, i64, String, String, String, String)> = self
                .db
                .with_read(|conn| {
                    let sql = if cursor.is_none() {
                        "SELECT r.id, r.sensitivity, r.namespace, r.owner_id,
                                r.scope, r.source_id
                         FROM records r
                         WHERE r.episode_id = ?1
                           AND (r.truth_state IS NULL OR r.truth_state = 'current')
                         ORDER BY r.id ASC
                         LIMIT ?2"
                    } else {
                        "SELECT r.id, r.sensitivity, r.namespace, r.owner_id,
                                r.scope, r.source_id
                         FROM records r
                         WHERE r.episode_id = ?1
                           AND (r.truth_state IS NULL OR r.truth_state = 'current')
                           AND r.id > ?3
                         ORDER BY r.id ASC
                         LIMIT ?2"
                    };
                    let mut stmt = conn.prepare(sql).map_err(StorageError::Sqlite)?;
                    let rows: Vec<_> = if cursor.is_none() {
                        stmt.query_map(params![episode_id, page_size], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, i64>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, String>(5)?,
                            ))
                        })
                        .map_err(StorageError::Sqlite)?
                        .filter_map(|r| r.ok())
                        .collect()
                    } else {
                        stmt.query_map(
                            params![episode_id, page_size, cursor.as_ref().unwrap()],
                            |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, i64>(1)?,
                                    row.get::<_, String>(2)?,
                                    row.get::<_, String>(3)?,
                                    row.get::<_, String>(4)?,
                                    row.get::<_, String>(5)?,
                                ))
                            },
                        )
                        .map_err(StorageError::Sqlite)?
                        .filter_map(|r| r.ok())
                        .collect()
                    };
                    Ok(rows)
                })
                .map_err(|e| EpisodeError::NotFound(e.to_string()))?;

            if page.is_empty() {
                break;
            }
            cursor = Some(page.last().unwrap().0.clone());
            all_ids.extend(page.into_iter().map(|(id, ..)| id));
        }

        // Sort lexicographically (design §7.3: "sorted parent IDs").
        all_ids.sort();
        all_ids.dedup();

        // Derive effective policy (most restrictive across all records).
        let effective_policy = self.derive_effective_policy(episode_id, &all_ids)?;

        // Compute input_set_hash over sorted IDs.
        let input_set_hash = compute_input_set_hash(&all_ids);

        // Idempotency check: does this run already exist?
        let existing: Option<String> = self
            .db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT id FROM consolidation_runs
                     WHERE algorithm = ?1 AND version = ?2
                       AND input_set_hash = ?3 AND level = ?4",
                        params![
                            CONSOLIDATION_ALGORITHM,
                            CONSOLIDATION_ALGORITHM_VERSION,
                            input_set_hash,
                            level.as_str(),
                        ],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(StorageError::Sqlite)?)
            })
            .map_err(|e| EpisodeError::NotFound(e.to_string()))?;

        if let Some(run_id) = existing {
            return Err(EpisodeError::DuplicateRun {
                existing_run_id: run_id,
            });
        }

        // Insert a new consolidation_runs row.
        let run_id = new_id().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        {
            let conn = self.db.write();
            conn.execute(
                "INSERT INTO consolidation_runs
                     (id, algorithm, version, input_set_hash, level,
                      cursor, status, started_at, completed_at, output_id, error_code)
                 VALUES (?1,?2,?3,?4,?5,NULL,'pending',?6,NULL,NULL,NULL)",
                params![
                    run_id,
                    CONSOLIDATION_ALGORITHM,
                    CONSOLIDATION_ALGORITHM_VERSION,
                    input_set_hash,
                    level.as_str(),
                    now,
                ],
            )
            .map_err(StorageError::Sqlite)
            .map_err(|e| EpisodeError::NotFound(e.to_string()))?;
        }

        Ok(ConsolidationCandidateSet {
            run_id,
            record_ids: all_ids,
            effective_policy,
            input_set_hash,
            level,
            episode_id: episode_id.to_string(),
        })
    }

    // ── Cursor resume ─────────────────────────────────────────────────────────

    /// Return the current consolidation cursor for an episode (the `created_event_id`
    /// of the last record included in a consolidation pass, or `None` if none yet).
    /// This is a valid `events_v2.id` satisfying the FK constraint.
    pub fn get_cursor(&self, episode_id: &str) -> MemoryResult<Option<String>> {
        self.db.with_read(|conn| {
            Ok(conn
                .query_row(
                    "SELECT cursor_event_id FROM episodes_v2 WHERE id = ?1",
                    params![episode_id],
                    |r| r.get::<_, Option<String>>(0),
                )
                .map_err(StorageError::Sqlite)?)
        })
    }

    /// Advance the consolidation cursor for an episode to the `events_v2.id` of
    /// the last processed record (`record.created_event_id`). Call this after a
    /// successful consolidation pass so crash-resume can skip already-processed records.
    pub fn advance_cursor(&self, episode_id: &str, event_id: &str) -> MemoryResult<()> {
        let conn = self.db.write();
        conn.execute(
            "UPDATE episodes_v2
             SET cursor_event_id = ?1,
                 revision = COALESCE(revision, 1) + 1
             WHERE id = ?2",
            params![event_id, episode_id],
        )
        .map_err(StorageError::Sqlite)?;
        Ok(())
    }

    // ── Effective policy derivation ───────────────────────────────────────────

    /// Derive the effective policy (most restrictive) across the candidate record
    /// set.  If `record_ids` is empty, falls back to the episode's own policy.
    fn derive_effective_policy(
        &self,
        episode_id: &str,
        record_ids: &[String],
    ) -> Result<EffectivePolicy, EpisodeError> {
        // Start with the episode policy as the baseline.
        let ep = self
            .get_episode(episode_id)
            .map_err(|e| EpisodeError::NotFound(e.to_string()))?
            .ok_or_else(|| EpisodeError::NotFound(episode_id.to_string()))?;

        if record_ids.is_empty() {
            return Ok(EffectivePolicy {
                namespace: ep.namespace,
                owner_id: ep.owner_id,
                scope: ep.scope,
                sensitivity: ep.sensitivity,
                source_id: ep.source_id,
                policy_version: ep.policy_version,
            });
        }

        // Compute max sensitivity across records.
        let max_sensitivity: i64 =
            self.db
                .with_read(|conn| {
                    // Use a simple MAX aggregate — the records are already filtered to
                    // this episode so namespace/scope/owner are uniform by construction.
                    Ok(conn.query_row(
                    "SELECT COALESCE(MAX(sensitivity), 0) FROM records WHERE episode_id = ?1",
                    params![episode_id],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(StorageError::Sqlite)?)
                })
                .map_err(|e| EpisodeError::NotFound(e.to_string()))?;

        Ok(EffectivePolicy {
            namespace: ep.namespace,
            owner_id: ep.owner_id,
            scope: ep.scope,
            sensitivity: max_sensitivity,
            source_id: ep.source_id,
            policy_version: ep.policy_version,
        })
    }
}

// ── Row mapper ────────────────────────────────────────────────────────────────

fn row_to_episode(r: &rusqlite::Row<'_>) -> rusqlite::Result<EpisodeV2> {
    Ok(EpisodeV2 {
        id: r.get(0)?,
        session_id: r.get(1)?,
        task_id: r.get(2)?,
        namespace: r.get(3)?,
        owner_id: r.get(4)?,
        scope: r.get(5)?,
        sensitivity: r.get(6)?,
        source_id: r.get(7)?,
        policy_version: r.get(8)?,
        opened_at: r.get(9)?,
        closed_at: r.get(10)?,
        boundary_reason: r.get(11)?,
        cursor_event_id: r.get(12)?,
        truth_state: r.get(13)?,
        revision: r.get(14)?,
    })
}

// ── Hash utility ──────────────────────────────────────────────────────────────

/// Compute the SHA-256 input_set_hash from a sorted list of record IDs (design §7.3).
pub fn compute_input_set_hash(sorted_ids: &[String]) -> String {
    let mut hasher = Sha256::new();
    for id in sorted_ids {
        hasher.update(id.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

// ── Derived-record output identity ───────────────────────────────────────────

/// Compute the semantic output identity for a derived record (design §7.3).
///
/// `semantic_output_id = SHA-256(level || "\n" || algorithm_version || "\n" || sorted(parent_id || "\n"))`
///
/// This makes the identity deterministic and collision-resistant across
/// algorithm versions.  Two derivation runs that receive the same sorted
/// parent set and use the same algorithm/version will produce the same ID,
/// preventing duplicate derivations.
pub fn compute_semantic_output_id(
    level: ConsolidationLevel,
    algorithm_version: &str,
    sorted_parent_ids: &[String],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(level.as_str().as_bytes());
    hasher.update(b"\n");
    hasher.update(algorithm_version.as_bytes());
    hasher.update(b"\n");
    for id in sorted_parent_ids {
        hasher.update(id.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

// ── Truth propagation ─────────────────────────────────────────────────────────

/// Propagate truth state from parent records to a derived record (design §7.3).
///
/// Rules (in precedence order):
/// 1. If ANY parent is `contradicted`  → derived record is `contradicted`
/// 2. If ANY parent is `stale` (and none are `contradicted`) → derived record is `stale`
/// 3. If ALL parents are `current` → derived record is `current`
/// 4. Otherwise → `unverified`
pub fn propagate_truth_state(parent_truth_states: &[String]) -> &'static str {
    if parent_truth_states.iter().any(|s| s == "contradicted") {
        return "contradicted";
    }
    if parent_truth_states.iter().any(|s| s == "stale") {
        return "stale";
    }
    if !parent_truth_states.is_empty() && parent_truth_states.iter().all(|s| s == "current") {
        return "current";
    }
    "unverified"
}

// ── Evidence source metadata ──────────────────────────────────────────────────

/// Metadata about a single contributing evidence source for readiness checking.
///
/// Each parent record in a `ConsolidationCandidateSet` contributes one
/// `EvidenceSource`.  The `source_id` and `source_kind` fields are used to
/// measure independence and diversity (task F3.6.5).
#[derive(Clone, Debug, PartialEq)]
pub struct EvidenceSource {
    /// Stable identifier for this source (e.g., session ID, user ID, tool ID).
    /// Two records with the same `source_id` count as ONE independent source.
    pub source_id: String,
    /// Category of the source (e.g., `"user"`, `"tool"`, `"self_reflection"`).
    /// Used to measure source-kind diversity for Rule-level promotion.
    pub source_kind: String,
}

// ── Consolidation readiness checker ──────────────────────────────────────────

/// Outcome of a readiness check.
#[derive(Clone, Debug, PartialEq)]
pub enum ReadinessOutcome {
    /// Promotion to the requested level is permitted.
    Ready,
    /// Promotion is refused; reason records WHY (false-promotion reason, §F3.6.5).
    Refused {
        /// Human-readable explanation recorded as a false-promotion reason.
        reason: String,
    },
}

/// Validates whether a set of evidence sources is sufficient to be promoted to a
/// given `ConsolidationLevel` (design §F3.6.5 / MGR-035, MGR-038–039, MGR-045).
///
/// # Rules
/// | Level   | Min independent sources | Min distinct source_kinds | self_reflection allowed? |
/// |---------|------------------------|---------------------------|--------------------------|
/// | Episode | 1                      | any                       | no (as sole source)      |
/// | Summary | 1                      | any                       | no (as sole source)      |
/// | Skill   | 2                      | any                       | no (as sole source)      |
/// | Rule    | 3                      | 2                         | never                    |
///
/// `self_reflection` sources are **never** counted as independent evidence and
/// their effective confidence is **capped at `REFLECTION_CONFIDENCE_CAP` (0.6)**.
pub struct ConsolidationReadinessChecker;

impl ConsolidationReadinessChecker {
    /// Check whether `sources` satisfy the promotion gate for `level`.
    ///
    /// Returns [`ReadinessOutcome::Ready`] on success or
    /// [`ReadinessOutcome::Refused`] with a `reason` string on failure.
    pub fn check_readiness(
        level: ConsolidationLevel,
        sources: &[EvidenceSource],
    ) -> ReadinessOutcome {
        // Partition: self_reflection vs independent (non-self-reflection).
        let independent: Vec<&EvidenceSource> = sources
            .iter()
            .filter(|s| s.source_kind != "self_reflection")
            .collect();

        // Distinct independent source IDs (deduplicated by source_id).
        let mut distinct_source_ids: Vec<&str> =
            independent.iter().map(|s| s.source_id.as_str()).collect();
        distinct_source_ids.sort_unstable();
        distinct_source_ids.dedup();
        let n_independent = distinct_source_ids.len();

        // Distinct independent source kinds.
        let mut distinct_kinds: Vec<&str> =
            independent.iter().map(|s| s.source_kind.as_str()).collect();
        distinct_kinds.sort_unstable();
        distinct_kinds.dedup();
        let n_kinds = distinct_kinds.len();

        match level {
            ConsolidationLevel::Episode | ConsolidationLevel::Summary => {
                // Minimum 1 non-self-reflection source.
                if n_independent == 0 {
                    return ReadinessOutcome::Refused {
                        reason: format!(
                            "insufficient independent sources: 0 found, 1 required for {:?}; \
                             self_reflection cannot be the sole evidence source",
                            level
                        ),
                    };
                }
                ReadinessOutcome::Ready
            }
            ConsolidationLevel::Skill => {
                // Minimum 2 independent sources (distinct source_ids).
                if n_independent < 2 {
                    return ReadinessOutcome::Refused {
                        reason: format!(
                            "insufficient independent sources: {} found, 2 required for Skill; \
                             each independent source must have a distinct source_id",
                            n_independent
                        ),
                    };
                }
                ReadinessOutcome::Ready
            }
            ConsolidationLevel::Rule => {
                // Rule: never allow self_reflection sources AT ALL.
                let has_self_reflection =
                    sources.iter().any(|s| s.source_kind == "self_reflection");
                if has_self_reflection {
                    return ReadinessOutcome::Refused {
                        reason: "Rule-level promotion refused: self_reflection sources are never \
                             permitted as evidence for Rule derivation"
                            .to_string(),
                    };
                }
                // Minimum 3 independent sources.
                if n_independent < 3 {
                    return ReadinessOutcome::Refused {
                        reason: format!(
                            "insufficient independent sources: {} found, 3 required for Rule",
                            n_independent
                        ),
                    };
                }
                // Minimum 2 distinct source_kinds.
                if n_kinds < 2 {
                    return ReadinessOutcome::Refused {
                        reason: format!(
                            "insufficient source_kind diversity: {} distinct kind(s) found, \
                             2 required for Rule (e.g., 'user' and 'tool')",
                            n_kinds
                        ),
                    };
                }
                ReadinessOutcome::Ready
            }
        }
    }

    /// Cap the effective confidence of a self-reflection source at
    /// [`REFLECTION_CONFIDENCE_CAP`] (0.6).  Non-reflection values pass through
    /// unchanged (design §20, L11/D-9).
    pub fn cap_confidence(source_kind: &str, confidence: f32) -> f32 {
        use crate::cognition::REFLECTION_CONFIDENCE_CAP;
        if source_kind == "self_reflection" {
            confidence.min(REFLECTION_CONFIDENCE_CAP)
        } else {
            confidence
        }
    }
}

// ── Rule escalation approval ──────────────────────────────────────────────────

/// Explicit approval token required for Rule-level derivation (§F3.6.5).
///
/// Rule derivation via [`derive_record`] is **never automatic**.  The caller
/// must obtain and supply an explicit `RuleEscalationApproval` — without it,
/// [`derive_record`] returns
/// [`DeriveRecordError::AutomaticRuleEscalationDenied`].
///
/// This type is intentionally plain data (not an authority-signed token) so
/// tests can construct it; the enforcement invariant lives in [`derive_record`].
#[derive(Clone, Debug, PartialEq)]
pub struct RuleEscalationApproval {
    /// ID of the actor/policy that issued the approval.
    pub approved_by: String,
    /// Optional audit note explaining the reason for approval.
    pub reason: Option<String>,
}

impl RuleEscalationApproval {
    pub fn new(approved_by: impl Into<String>) -> Self {
        Self {
            approved_by: approved_by.into(),
            reason: None,
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

// ── Derive record error ───────────────────────────────────────────────────────

/// Error from `derive_record`.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DeriveRecordError {
    #[error("Rule-level inputs cannot be compressed further (compression ceiling)")]
    CompressionCeiling,
    #[error("Candidate set is empty — no parent records to derive from")]
    EmptyParentSet,
    /// Returned when a caller attempts to derive a Rule-level record without
    /// providing an explicit [`RuleEscalationApproval`].  Rule derivation is
    /// never automatic (§F3.6.5 / MGR-039).
    #[error("Automatic Rule escalation is denied: an explicit RuleEscalationApproval is required")]
    AutomaticRuleEscalationDenied,
    /// Returned when the evidence sources do not meet the minimum requirements
    /// for promotion to the requested level (§F3.6.5 / MGR-035).
    #[error("Promotion refused: {reason}")]
    InsufficientEvidence {
        /// The human-readable false-promotion reason.
        reason: String,
    },
    #[error("Internal storage error: {0}")]
    Storage(String),
}

impl From<crate::error::MemoryError> for DeriveRecordError {
    fn from(e: crate::error::MemoryError) -> Self {
        DeriveRecordError::Storage(e.to_string())
    }
}

impl From<StorageError> for DeriveRecordError {
    fn from(e: StorageError) -> Self {
        DeriveRecordError::Storage(e.to_string())
    }
}

// ── Derive record result ──────────────────────────────────────────────────────

/// The result of a successful `derive_record` call.
#[derive(Clone, Debug, PartialEq)]
pub struct DerivedRecord {
    /// The ID of the derived `records` row.
    pub record_id: String,
    /// The semantic output identity (SHA-256 over level + algorithm + sorted parents).
    /// Two identical runs return the same `semantic_id`.
    pub semantic_id: String,
    /// Whether this was a newly-created record (`false`) or an idempotent
    /// return of an already-existing record with the same semantic identity
    /// (`true`).
    pub was_existing: bool,
    /// Number of `memory_links` rows written (0 when `was_existing = true`).
    pub links_written: usize,
    /// The propagated truth state of the derived record.
    pub truth_state: String,
}

// ── Spec for derive_record ────────────────────────────────────────────────────

/// Specification for deriving a new record from a `ConsolidationCandidateSet`.
#[derive(Clone, Debug)]
pub struct DeriveRecordSpec {
    /// The derived content text.
    pub content: String,
    /// The `events_v2.id` of the consolidation-run event (provenance).
    pub created_event_id: String,
}

// ── derive_record ─────────────────────────────────────────────────────────────

/// Derive a new `summary`, `skill`, or `rule` record from a
/// `ConsolidationCandidateSet` (design §7.3, task F3.6.4).
///
/// # Evidence gating (§F3.6.5)
/// Before writing anything, `derive_record` calls
/// [`ConsolidationReadinessChecker::check_readiness`] on the provided
/// `sources`.  If the sources do not meet the minimum requirements for the
/// requested level the function returns
/// [`DeriveRecordError::InsufficientEvidence`] with the false-promotion
/// reason recorded in the error.
///
/// # Rule escalation gate (§F3.6.5 / MGR-039)
/// When the candidate set targets [`ConsolidationLevel::Rule`], the caller
/// **must** supply an explicit [`RuleEscalationApproval`].  Passing `None`
/// returns [`DeriveRecordError::AutomaticRuleEscalationDenied`] immediately —
/// no automatic Rule escalation is ever permitted.
///
/// # Identity and idempotency
/// The semantic output identity is:
/// ```text
/// SHA-256(level || "\n" || CONSOLIDATION_ALGORITHM_VERSION || "\n" || sorted(parent_ids))
/// ```
/// If a `records` row with `content_hash = semantic_id` already exists for
/// this episode and level, the function returns it without inserting a
/// duplicate (`was_existing = true`).
///
/// # Truth propagation (design §7.3)
/// - ANY parent `contradicted` → derived record is `contradicted`
/// - ANY parent `stale` (no `contradicted`) → derived record is `stale`
/// - ALL parents `current` → derived record is `current`
/// - Otherwise → `unverified`
///
/// # Restrictive Effective Policy (design §7.3)
/// The derived record inherits the most-restrictive (maximum) sensitivity
/// across all parent records, as computed by `ConsolidationCandidateSet`.
///
/// # Compression ceiling (design §7.3)
/// If the candidate set level is `Rule` (level 3), parent records must not
/// themselves be `rule`-kind; any Rule-level input causes an immediate
/// `DeriveRecordError::CompressionCeiling`.
///
/// # Immediate `derived_from` links (design §7.3)
/// For each parent record ID, a `memory_links` row of type `derived_from`
/// (version 1) is inserted from the parent to the new record.
///
/// # Source history retention
/// The `created_event_id` field on the derived record is set from
/// `spec.created_event_id` so the consolidation event is preserved in the
/// provenance chain.
pub fn derive_record(
    db: &Database,
    candidate_set: &ConsolidationCandidateSet,
    spec: &DeriveRecordSpec,
    sources: &[EvidenceSource],
    rule_approval: Option<&RuleEscalationApproval>,
) -> Result<DerivedRecord, DeriveRecordError> {
    // ── Rule escalation gate: no automatic Rule derivation ───────────────────
    if candidate_set.level == ConsolidationLevel::Rule && rule_approval.is_none() {
        return Err(DeriveRecordError::AutomaticRuleEscalationDenied);
    }

    // ── Evidence readiness gate (§F3.6.5) ────────────────────────────────────
    match ConsolidationReadinessChecker::check_readiness(candidate_set.level, sources) {
        ReadinessOutcome::Ready => {}
        ReadinessOutcome::Refused { reason } => {
            return Err(DeriveRecordError::InsufficientEvidence { reason });
        }
    }

    // ── Compression ceiling: Rule-level inputs cannot be further compressed ──
    if candidate_set.level == ConsolidationLevel::Rule {
        // Query whether any parent records are themselves 'rule' kind.
        let rule_parent_count: i64 = db
            .with_read(|conn| {
                let placeholders: String = candidate_set
                    .record_ids
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT COUNT(*) FROM records WHERE id IN ({placeholders}) AND record_kind = 'rule'"
                );
                let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
                let count: i64 = stmt
                    .query_row(
                        rusqlite::params_from_iter(candidate_set.record_ids.iter()),
                        |r| r.get(0),
                    )
                    .map_err(StorageError::Sqlite)?;
                Ok(count)
            })
            .map_err(DeriveRecordError::from)?;

        if rule_parent_count > 0 {
            return Err(DeriveRecordError::CompressionCeiling);
        }
    }

    // ── Empty parent set guard ──
    if candidate_set.record_ids.is_empty() {
        return Err(DeriveRecordError::EmptyParentSet);
    }

    // ── Compute semantic output identity ──
    let semantic_id = compute_semantic_output_id(
        candidate_set.level,
        CONSOLIDATION_ALGORITHM_VERSION,
        &candidate_set.record_ids,
    );

    // ── Idempotency check: does a record with this semantic identity exist? ──
    let existing_id: Option<String> = db
        .with_read(|conn| {
            Ok(conn
                .query_row(
                    "SELECT id FROM records WHERE content_hash = ?1 AND episode_id = ?2",
                    params![semantic_id, candidate_set.episode_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(StorageError::Sqlite)?)
        })
        .map_err(DeriveRecordError::from)?;

    if let Some(record_id) = existing_id {
        return Ok(DerivedRecord {
            record_id,
            semantic_id,
            was_existing: true,
            links_written: 0,
            truth_state: "current".to_string(), // returned as-is; caller may re-query for full state
        });
    }

    // ── Determine truth state via propagation ──
    let parent_truth_states: Vec<String> = {
        let placeholders: String = candidate_set
            .record_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT COALESCE(truth_state, 'current') FROM records WHERE id IN ({placeholders})"
        );
        db.with_read(|conn| {
            let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(
                    rusqlite::params_from_iter(candidate_set.record_ids.iter()),
                    |r| r.get::<_, String>(0),
                )
                .map_err(StorageError::Sqlite)?;
            let mut states = Vec::new();
            for row in rows {
                states.push(row.map_err(StorageError::Sqlite)?);
            }
            Ok(states)
        })
        .map_err(DeriveRecordError::from)?
    };

    let truth_state = propagate_truth_state(&parent_truth_states).to_string();

    // ── Determine record_kind from level ──
    let record_kind = match candidate_set.level {
        ConsolidationLevel::Episode => "memory",
        ConsolidationLevel::Summary => "summary",
        ConsolidationLevel::Skill => "skill",
        ConsolidationLevel::Rule => "rule",
    };

    // ── Insert the derived record ──
    let record_id = new_id().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let ep = &candidate_set.effective_policy;

    {
        let conn = db.write();
        conn.execute(
            "INSERT INTO records (
                 id, record_kind, schema_version, content, content_hash,
                 truth_state, staleness_class,
                 namespace, owner_id, scope, sensitivity, source_id, policy_version,
                 created_event_id, created_at, episode_id
             ) VALUES (?1,?2,1,?3,?4,?5,'slow',?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                record_id,
                record_kind,
                spec.content,
                semantic_id,
                truth_state,
                ep.namespace,
                ep.owner_id,
                ep.scope,
                ep.sensitivity,
                ep.source_id,
                ep.policy_version,
                spec.created_event_id,
                now,
                candidate_set.episode_id,
            ],
        )
        .map_err(StorageError::Sqlite)
        .map_err(DeriveRecordError::from)?;
    }

    // ── Create immediate derived_from memory links ──
    // One link per parent: parent → derived record, link_type = 'derived_from', version 1.
    let mut links_written = 0usize;
    for parent_id in &candidate_set.record_ids {
        let link_id = new_id().to_string();
        let conn = db.write();
        conn.execute(
            "INSERT OR IGNORE INTO memory_links (
                 id, source_kind, source_id, target_kind, target_id,
                 link_type, link_version, truth_state,
                 namespace, owner_id, scope, sensitivity, source_policy_id, policy_version,
                 created_event_id, revision
             ) VALUES (?1,'memory',?2,'memory',?3,'derived_from',1,'current',?4,?5,?6,?7,?8,?9,?10,1)",
            params![
                link_id,
                parent_id,
                record_id,
                ep.namespace,
                ep.owner_id,
                ep.scope,
                ep.sensitivity,
                ep.source_id,
                ep.policy_version,
                spec.created_event_id,
            ],
        )
        .map_err(StorageError::Sqlite)
        .map_err(DeriveRecordError::from)?;
        links_written += 1;
    }

    // ── Update consolidation_runs row with output_id and completed status ──
    {
        let conn = db.write();
        conn.execute(
            "UPDATE consolidation_runs
             SET output_id = ?1, status = 'completed', completed_at = ?2
             WHERE id = ?3",
            params![record_id, now, candidate_set.run_id],
        )
        .map_err(StorageError::Sqlite)
        .map_err(DeriveRecordError::from)?;
    }

    Ok(DerivedRecord {
        record_id,
        semantic_id,
        was_existing: false,
        links_written,
        truth_state,
    })
}

// ── Durable resume / idempotency ─────────────────────────────────────────────

/// The outcome returned by [`resume_or_create_run`].
#[derive(Clone, Debug, PartialEq)]
pub enum ResumeOrCreateOutcome {
    /// A new `consolidation_runs` row was created with `status = 'pending'`.
    Created {
        /// The freshly-assigned run ID.
        run_id: String,
    },
    /// An existing row with `status = 'pending'` (or `'in_progress'`) was found.
    /// The caller should resume from this run rather than create a duplicate.
    Resumed {
        /// The existing run ID.
        run_id: String,
    },
    /// An existing row with `status = 'completed'` was found.
    /// The result is idempotent — no new work is required.
    Completed {
        /// The existing run ID.
        run_id: String,
        /// The `output_id` produced by the completed run (may be `None` in
        /// degenerate cases where the run completed without an output, e.g.,
        /// an empty candidate set).
        output_id: Option<String>,
    },
}

/// Look up an existing `consolidation_runs` row for the given identity key
/// `(algorithm, version, input_set_hash, level)` and decide whether to resume
/// it or create a new run.
///
/// # Semantics
/// | Existing row status  | Return value                      |
/// |---------------------|-----------------------------------|
/// | `pending`           | `Resumed { run_id }`              |
/// | `in_progress`       | `Resumed { run_id }`              |
/// | `completed`         | `Completed { run_id, output_id }` |
/// | (none)              | `Created { run_id }`  — new row inserted |
///
/// This makes the crash-resume path explicit: a `pending` run that survived a
/// crash is returned directly so the caller can reuse its run ID and continue
/// from the cursor position without re-inserting a duplicate row.
pub fn resume_or_create_run(
    db: &Database,
    algorithm: &str,
    version: &str,
    input_set_hash: &str,
    level: ConsolidationLevel,
) -> MemoryResult<ResumeOrCreateOutcome> {
    // Look up any existing row for this identity key.
    let existing: Option<(String, String, Option<String>)> = db.with_read(|conn| {
        Ok(conn
            .query_row(
                "SELECT id, status, output_id FROM consolidation_runs
                 WHERE algorithm = ?1 AND version = ?2
                   AND input_set_hash = ?3 AND level = ?4",
                params![algorithm, version, input_set_hash, level.as_str()],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(StorageError::Sqlite)?)
    })?;

    match existing {
        Some((run_id, status, output_id)) => match status.as_str() {
            "completed" => Ok(ResumeOrCreateOutcome::Completed { run_id, output_id }),
            // pending or in_progress — resume
            _ => Ok(ResumeOrCreateOutcome::Resumed { run_id }),
        },
        None => {
            // Create a fresh pending run.
            let run_id = new_id().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            let conn = db.write();
            conn.execute(
                "INSERT INTO consolidation_runs
                     (id, algorithm, version, input_set_hash, level,
                      cursor, status, started_at, completed_at, output_id, error_code)
                 VALUES (?1,?2,?3,?4,?5,NULL,'pending',?6,NULL,NULL,NULL)",
                params![
                    run_id,
                    algorithm,
                    version,
                    input_set_hash,
                    level.as_str(),
                    now
                ],
            )
            .map_err(StorageError::Sqlite)?;
            Ok(ResumeOrCreateOutcome::Created { run_id })
        }
    }
}

// ── Downstream stale propagation ──────────────────────────────────────────────

/// Propagate `stale` truth state to all derived records that have a
/// `derived_from` link originating from `source_record_id`, recursively up to
/// `max_depth` levels (design §F3.6.6, invariant: depth 3 covers
/// Episode→Summary→Skill→Rule).
///
/// # Rules
/// * Only records whose `truth_state` is **not already** `contradicted`,
///   `forgotten`, or `deleted` are updated — those states are terminal and must
///   not be downgraded.
/// * The propagation is recursive: a Summary that becomes `stale` will itself
///   trigger propagation to any Skill derived from it, and so on, until
///   `max_depth` is exhausted.
/// * A `source_record_id` that does not exist (or has no derived records) is a
///   no-op — the function returns an empty set rather than an error.
///
/// # Returns
/// The set of `records.id` values that were updated to `stale`.
pub fn propagate_stale_to_derived_records(
    db: &Database,
    source_record_id: &str,
    max_depth: usize,
) -> MemoryResult<Vec<String>> {
    let mut updated: Vec<String> = Vec::new();
    // BFS / iterative depth-limited traversal to avoid recursion stack issues.
    // `frontier` holds the IDs whose derived children we need to examine next.
    let mut frontier: Vec<String> = vec![source_record_id.to_string()];

    for _depth in 0..max_depth {
        if frontier.is_empty() {
            break;
        }
        let mut next_frontier: Vec<String> = Vec::new();

        for parent_id in &frontier {
            // Find all records that are derived from this parent via a
            // `derived_from` link in `memory_links`.
            let derived_ids: Vec<String> = db.with_read(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT ml.target_id
                         FROM memory_links ml
                         WHERE ml.source_id = ?1
                           AND ml.link_type = 'derived_from'
                           AND (ml.truth_state IS NULL
                                OR ml.truth_state NOT IN ('superseded','forgotten','deleted'))",
                    )
                    .map_err(StorageError::Sqlite)?;
                let rows = stmt
                    .query_map(params![parent_id], |r| r.get::<_, String>(0))
                    .map_err(StorageError::Sqlite)?;
                let mut ids = Vec::new();
                for row in rows {
                    ids.push(row.map_err(StorageError::Sqlite)?);
                }
                Ok(ids)
            })?;

            for derived_id in derived_ids {
                // Skip if already updated this run (avoid re-processing in
                // diamond / multi-parent graphs).
                if updated.contains(&derived_id) || source_record_id == derived_id.as_str() {
                    continue;
                }

                // Fetch the current truth_state of the derived record.
                let current_state: Option<String> = db.with_read(|conn| {
                    Ok(conn
                        .query_row(
                            "SELECT truth_state FROM records WHERE id = ?1",
                            params![derived_id],
                            |r| r.get::<_, Option<String>>(0),
                        )
                        .optional()
                        .map_err(StorageError::Sqlite)?
                        .flatten())
                })?;

                // Skip terminal states — do not downgrade contradicted/forgotten/deleted.
                let skip = matches!(
                    current_state.as_deref(),
                    Some("contradicted") | Some("forgotten") | Some("deleted") | Some("stale")
                );
                if skip {
                    // Still add to next_frontier so deeper derived records are reached,
                    // but don't count this one as newly updated.
                    next_frontier.push(derived_id);
                    continue;
                }

                // Mark as stale.
                {
                    let conn = db.write();
                    conn.execute(
                        "UPDATE records
                         SET truth_state = 'stale'
                         WHERE id = ?1",
                        params![derived_id],
                    )
                    .map_err(StorageError::Sqlite)?;
                }
                updated.push(derived_id.clone());
                next_frontier.push(derived_id);
            }
        }

        frontier = next_frontier;
    }

    Ok(updated)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::scheduler::StaticResourceMonitor;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn manager(db: Arc<Database>) -> EpisodeManager {
        EpisodeManager::with_defaults(db)
    }

    fn manager_capped(db: Arc<Database>, max: usize) -> EpisodeManager {
        EpisodeManager::new(
            db,
            EpisodeConfig {
                max_records_per_episode: max,
                ..Default::default()
            },
        )
    }

    fn ok_monitor() -> StaticResourceMonitor {
        StaticResourceMonitor {
            on_battery: false,
            memory_pressure: false,
            thermal_pressure: false,
            model_pressure: false,
        }
    }

    fn constrained_monitor() -> StaticResourceMonitor {
        StaticResourceMonitor {
            on_battery: true,
            memory_pressure: false,
            thermal_pressure: false,
            model_pressure: false,
        }
    }

    /// Insert a minimal `records` row belonging to an episode.
    /// Returns `(record_id, event_id)`.
    fn insert_record(db: &Arc<Database>, episode_id: &str) -> (String, String) {
        let record_id = new_id().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        // We need a valid events_v2 row for the FK on created_event_id.
        let event_id = new_id().to_string();
        // Use a UUID-derived HLC string to guarantee uniqueness across rapid calls.
        let hlc = format!(
            "{:016x}{:08x}",
            chrono::Utc::now().timestamp_millis() as u64,
            uuid::Uuid::now_v7().as_u128() as u32
        );
        let conn = db.write();
        conn.execute(
            "INSERT INTO events_v2 (
                 id, phase, hlc, ts_utc, tz_offset_min, event_type,
                 source_kind, source_id, actor_id,
                 namespace, owner_id, scope, sensitivity, policy_version,
                 payload_plain, payload_encoding, payload_checksum, schema_version)
             VALUES (?1,'start',?2,?3,0,'observation','user','src-1','actor-1',
                     'ns','owner-1','private',0,'v1','{}','utf8','chk',1)",
            params![event_id, hlc, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO records (
                 id, record_kind, schema_version, content, content_hash,
                 truth_state, namespace, owner_id, scope, sensitivity,
                 source_id, policy_version, created_event_id, created_at, episode_id)
             VALUES (?1,'memory',1,'test content','hash-x','current',
                     'ns','owner-1','private',0,'src-1','v1',?2,?3,?4)",
            params![record_id, event_id, now, episode_id],
        )
        .unwrap();
        (record_id, event_id)
    }

    // ── Test 1: open → records added → close with boundary reason ────────────

    #[test]
    fn open_records_close_with_session_end_reason() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        let spec = NewEpisode::simple("session-1", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        // Episode exists and is open.
        let ep = mgr.get_episode(&ep_id).unwrap().unwrap();
        assert!(ep.opened_at.is_some(), "opened_at should be set after open");
        assert!(ep.closed_at.is_none(), "episode should be open");

        // Insert a record to associate with the episode.
        insert_record(&db, &ep_id);
        assert_eq!(mgr.record_count(&ep_id).unwrap(), 1);

        // Close the episode with a specific reason.
        mgr.close_episode(&ep_id, Some(EpisodeBoundaryReason::SessionEnd))
            .unwrap();

        let closed_ep = mgr.get_episode(&ep_id).unwrap().unwrap();
        assert!(
            closed_ep.closed_at.is_some(),
            "closed_at should be set after close"
        );
        assert_eq!(
            closed_ep.boundary_reason.as_deref(),
            Some("session_end"),
            "boundary_reason should be session_end"
        );
        // Cursor should be set to the last record id.
        assert!(
            closed_ep.cursor_event_id.is_some(),
            "cursor_event_id should be advanced on close"
        );
    }

    // ── Test 2: double-close returns AlreadyClosed ────────────────────────────

    #[test]
    fn double_close_returns_already_closed_error() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        let spec = NewEpisode::simple("session-2", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();
        mgr.close_episode(&ep_id, Some(EpisodeBoundaryReason::Manual))
            .unwrap();

        let err = mgr
            .close_episode(&ep_id, Some(EpisodeBoundaryReason::Manual))
            .unwrap_err();
        assert!(
            err.to_string().contains("already closed"),
            "expected AlreadyClosed error, got: {err}"
        );
    }

    // ── Test 3: cursor advances correctly and is resumable ───────────────────

    #[test]
    fn cursor_advances_and_is_resumable() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        let spec = NewEpisode::simple("session-3", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        // Insert two records.
        let (_, ev1) = insert_record(&db, &ep_id);
        let (_, ev2) = insert_record(&db, &ep_id);

        // Close the episode — cursor should advance to the event_id of the last record
        // (ordered by record.id DESC, which maps to created_event_id).
        mgr.close_episode(&ep_id, Some(EpisodeBoundaryReason::TaskCompletion))
            .unwrap();

        let cursor = mgr.get_cursor(&ep_id).unwrap();
        assert!(
            cursor.is_some(),
            "cursor should be set after close with records"
        );
        // The cursor is the created_event_id of the record with the highest id (DESC).
        // Both ev1 and ev2 are valid; the cursor must be one of them.
        let cursor_val = cursor.unwrap();
        assert!(
            cursor_val == ev1 || cursor_val == ev2,
            "cursor should be one of the inserted event IDs, got {cursor_val}"
        );

        // Manually advance the cursor (simulating a resume after crash).
        mgr.advance_cursor(&ep_id, &ev1).unwrap();
        let updated_cursor = mgr.get_cursor(&ep_id).unwrap();
        assert_eq!(
            updated_cursor.unwrap(),
            ev1,
            "cursor should be updated to ev1"
        );
    }

    // ── Test 4: RecordCountLimit auto-close ───────────────────────────────────

    #[test]
    fn auto_close_triggers_record_count_limit() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        // Cap at 2 records.
        let mgr = manager_capped(db.clone(), 2);

        let spec = NewEpisode::simple("session-4", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        insert_record(&db, &ep_id);
        insert_record(&db, &ep_id);

        // Should auto-close.
        let auto_closed = mgr.auto_close_if_capped(&ep_id).unwrap();
        assert!(auto_closed, "episode should be auto-closed when at cap");

        let ep = mgr.get_episode(&ep_id).unwrap().unwrap();
        assert_eq!(
            ep.boundary_reason.as_deref(),
            Some("record_count_limit"),
            "boundary_reason should be record_count_limit"
        );
    }

    // ── Test 5: candidate selection returns bounded results ───────────────────

    #[tokio::test]
    async fn candidate_selection_returns_bounded_results_under_policy() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());
        let monitor = ok_monitor();

        let spec = NewEpisode::simple("session-5", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        // Insert 3 records.
        for _ in 0..3 {
            insert_record(&db, &ep_id);
        }

        // Close the episode so candidates can be selected.
        mgr.close_episode(&ep_id, Some(EpisodeBoundaryReason::SessionEnd))
            .unwrap();

        let cancel = CancellationToken::new();
        let candidate_set = mgr
            .select_candidates(&ep_id, ConsolidationLevel::Episode, &monitor, &cancel)
            .await
            .unwrap();

        assert_eq!(
            candidate_set.record_ids.len(),
            3,
            "should select all 3 records"
        );
        // Record IDs should be lexicographically sorted.
        let mut sorted = candidate_set.record_ids.clone();
        sorted.sort();
        assert_eq!(
            candidate_set.record_ids, sorted,
            "record_ids must be lexicographically sorted"
        );
        assert_eq!(candidate_set.level, ConsolidationLevel::Episode);
        assert_eq!(candidate_set.episode_id, ep_id);
        assert!(!candidate_set.run_id.is_empty());
        assert!(!candidate_set.input_set_hash.is_empty());
    }

    // ── Test 6: idempotency — duplicate run returns DuplicateRun ─────────────

    #[tokio::test]
    async fn duplicate_run_returns_existing_run_id() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());
        let monitor = ok_monitor();

        let spec = NewEpisode::simple("session-6", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();
        insert_record(&db, &ep_id);
        mgr.close_episode(&ep_id, Some(EpisodeBoundaryReason::SessionEnd))
            .unwrap();

        let cancel = CancellationToken::new();
        // First selection — should succeed.
        let first = mgr
            .select_candidates(&ep_id, ConsolidationLevel::Episode, &monitor, &cancel)
            .await
            .unwrap();

        // Second selection with identical inputs — should return DuplicateRun.
        let err = mgr
            .select_candidates(&ep_id, ConsolidationLevel::Episode, &monitor, &cancel)
            .await
            .unwrap_err();
        match &err {
            EpisodeError::DuplicateRun { existing_run_id } => {
                assert_eq!(
                    *existing_run_id, first.run_id,
                    "returned run_id should match the first run"
                );
            }
            other => panic!("expected DuplicateRun, got {other:?}"),
        }
    }

    // ── Test 7: resource policy denies selection on battery ──────────────────

    #[tokio::test]
    async fn resource_policy_denies_selection_on_battery() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());
        let monitor = constrained_monitor(); // on_battery = true

        let spec = NewEpisode::simple("session-7", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();
        insert_record(&db, &ep_id);
        mgr.close_episode(&ep_id, Some(EpisodeBoundaryReason::Manual))
            .unwrap();

        let cancel = CancellationToken::new();
        let err = mgr
            .select_candidates(&ep_id, ConsolidationLevel::Episode, &monitor, &cancel)
            .await
            .unwrap_err();
        assert_eq!(
            err,
            EpisodeError::ResourcePolicyDenied,
            "P4Maintenance should be denied when on battery"
        );
    }

    // ── Test 8: cancellation mid-selection leaves cursor recoverable ──────────

    #[tokio::test]
    async fn cancellation_mid_selection_leaves_cursor_recoverable() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        // Small page size to force multiple pages; we cancel immediately.
        let mgr = EpisodeManager::new(
            db.clone(),
            EpisodeConfig {
                candidate_page_size: 1,
                ..Default::default()
            },
        );
        let monitor = ok_monitor();

        let spec = NewEpisode::simple("session-8", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();
        insert_record(&db, &ep_id);
        insert_record(&db, &ep_id);
        mgr.close_episode(&ep_id, Some(EpisodeBoundaryReason::SessionEnd))
            .unwrap();

        // Cancel immediately before selection starts.
        let cancel = CancellationToken::new();
        cancel.cancel();

        let err = mgr
            .select_candidates(&ep_id, ConsolidationLevel::Episode, &monitor, &cancel)
            .await
            .unwrap_err();
        assert_eq!(
            err,
            EpisodeError::Cancelled,
            "cancelled token should produce Cancelled error"
        );

        // The episode cursor (set on close) must still be retrievable — no corruption.
        let cursor = mgr.get_cursor(&ep_id).unwrap();
        assert!(
            cursor.is_some(),
            "cursor set on close must survive a cancelled selection"
        );
    }

    // ── Test 9: selecting from an open episode returns StillOpen ─────────────

    #[tokio::test]
    async fn selection_from_open_episode_returns_still_open() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());
        let monitor = ok_monitor();

        let spec = NewEpisode::simple("session-9", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();
        insert_record(&db, &ep_id);
        // Deliberately NOT closing.

        let cancel = CancellationToken::new();
        let err = mgr
            .select_candidates(&ep_id, ConsolidationLevel::Episode, &monitor, &cancel)
            .await
            .unwrap_err();
        assert_eq!(
            err,
            EpisodeError::StillOpen(ep_id),
            "selecting from open episode must return StillOpen"
        );
    }

    // ── Test 10: input_set_hash is deterministic ──────────────────────────────

    #[test]
    fn input_set_hash_is_deterministic() {
        let ids_a = vec!["b".to_string(), "a".to_string(), "c".to_string()];
        let ids_b = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        // Hash must be order-independent when IDs are sorted before hashing.
        let mut sorted_a = ids_a.clone();
        sorted_a.sort();
        let mut sorted_b = ids_b.clone();
        sorted_b.sort();
        assert_eq!(
            compute_input_set_hash(&sorted_a),
            compute_input_set_hash(&sorted_b),
            "hash must be order-independent for sorted inputs"
        );
    }

    // ── Test 11: effective policy uses most-restrictive sensitivity ──────────

    #[tokio::test]
    async fn effective_policy_uses_most_restrictive_sensitivity() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        // We need records with different sensitivity values.
        // Insert manually with sensitivity 2 in one record.
        let mgr = manager(db.clone());
        let monitor = ok_monitor();

        let spec = NewEpisode::simple("session-11", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        // Insert a sensitivity=0 record.
        insert_record(&db, &ep_id);

        // Insert a sensitivity=2 record manually.
        let record_id = new_id().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let event_id = new_id().to_string();
        {
            use crate::ids::new_id as gen_new_id;
            let hlc = format!(
                "{:016x}{:08x}",
                chrono::Utc::now().timestamp_millis() as u64,
                gen_new_id().as_u128() as u32
            );
            let conn = db.write();
            conn.execute(
                "INSERT INTO events_v2 (
                     id, phase, hlc, ts_utc, tz_offset_min, event_type,
                     source_kind, source_id, actor_id,
                     namespace, owner_id, scope, sensitivity, policy_version,
                     payload_plain, payload_encoding, payload_checksum, schema_version)
                 VALUES (?1,'start',?2,?3,0,'observation','user','src-1','actor-1',
                         'ns','owner-1','private',0,'v1','{}','utf8','chk',1)",
                params![event_id, hlc, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO records (
                     id, record_kind, schema_version, content, content_hash,
                     truth_state, namespace, owner_id, scope, sensitivity,
                     source_id, policy_version, created_event_id, created_at, episode_id)
                 VALUES (?1,'memory',1,'sensitive content','hash-s','current',
                         'ns','owner-1','private',2,'src-1','v1',?2,?3,?4)",
                params![record_id, event_id, now, ep_id],
            )
            .unwrap();
        }

        mgr.close_episode(&ep_id, Some(EpisodeBoundaryReason::SessionEnd))
            .unwrap();

        let cancel = CancellationToken::new();
        let candidate_set = mgr
            .select_candidates(&ep_id, ConsolidationLevel::Episode, &monitor, &cancel)
            .await
            .unwrap();

        assert_eq!(
            candidate_set.effective_policy.sensitivity, 2,
            "effective sensitivity must be the maximum (most restrictive) across records"
        );
    }

    // ── Helper: insert a record with a specific truth_state ───────────────────

    fn insert_record_with_truth(
        db: &Arc<Database>,
        episode_id: &str,
        truth_state: &str,
    ) -> (String, String) {
        let record_id = new_id().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let event_id = new_id().to_string();
        let hlc = format!(
            "{:016x}{:08x}",
            chrono::Utc::now().timestamp_millis() as u64,
            uuid::Uuid::now_v7().as_u128() as u32
        );
        let conn = db.write();
        conn.execute(
            "INSERT INTO events_v2 (
                 id, phase, hlc, ts_utc, tz_offset_min, event_type,
                 source_kind, source_id, actor_id,
                 namespace, owner_id, scope, sensitivity, policy_version,
                 payload_plain, payload_encoding, payload_checksum, schema_version)
             VALUES (?1,'start',?2,?3,0,'observation','user','src-1','actor-1',
                     'ns','owner-1','private',0,'v1','{}','utf8','chk',1)",
            params![event_id, hlc, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO records (
                 id, record_kind, schema_version, content, content_hash,
                 truth_state, namespace, owner_id, scope, sensitivity,
                 source_id, policy_version, created_event_id, created_at, episode_id)
             VALUES (?1,'memory',1,'test content','hash-y',?2,
                     'ns','owner-1','private',0,'src-1','v1',?3,?4,?5)",
            params![record_id, truth_state, event_id, now, episode_id],
        )
        .unwrap();
        (record_id, event_id)
    }

    /// Build a minimal `ConsolidationCandidateSet` manually (without running
    /// the full `select_candidates` pipeline) for derive_record tests.
    fn make_candidate_set(
        db: &Arc<Database>,
        episode_id: &str,
        record_ids: Vec<String>,
        level: ConsolidationLevel,
    ) -> ConsolidationCandidateSet {
        // Insert a consolidation_runs row so derive_record can UPDATE it.
        // Use INSERT OR IGNORE so a duplicate call (idempotency test) doesn't fail.
        let input_set_hash = compute_input_set_hash(&record_ids);
        let now = chrono::Utc::now().to_rfc3339();

        // Check for an existing run first.
        let existing_run_id: Option<String> = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT id FROM consolidation_runs
                         WHERE algorithm = ?1 AND version = ?2
                           AND input_set_hash = ?3 AND level = ?4",
                        params![
                            CONSOLIDATION_ALGORITHM,
                            CONSOLIDATION_ALGORITHM_VERSION,
                            input_set_hash,
                            level.as_str(),
                        ],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(StorageError::Sqlite)
                    .unwrap())
            })
            .unwrap();

        let run_id = if let Some(id) = existing_run_id {
            id
        } else {
            let run_id = new_id().to_string();
            {
                let conn = db.write();
                conn.execute(
                    "INSERT INTO consolidation_runs
                         (id, algorithm, version, input_set_hash, level,
                          cursor, status, started_at, completed_at, output_id, error_code)
                     VALUES (?1,?2,?3,?4,?5,NULL,'pending',?6,NULL,NULL,NULL)",
                    params![
                        run_id,
                        CONSOLIDATION_ALGORITHM,
                        CONSOLIDATION_ALGORITHM_VERSION,
                        input_set_hash,
                        level.as_str(),
                        now,
                    ],
                )
                .unwrap();
            }
            run_id
        };

        ConsolidationCandidateSet {
            run_id,
            record_ids,
            effective_policy: EffectivePolicy {
                namespace: "ns".to_string(),
                owner_id: "owner-1".to_string(),
                scope: "private".to_string(),
                sensitivity: 0,
                source_id: "src-1".to_string(),
                policy_version: "v1".to_string(),
            },
            input_set_hash,
            level,
            episode_id: episode_id.to_string(),
        }
    }

    /// Two independent (non-self-reflection) sources with distinct source_ids.
    /// Satisfies the Summary (≥1) and Skill (≥2) minimum requirements.
    fn two_independent_sources() -> Vec<EvidenceSource> {
        vec![
            EvidenceSource {
                source_id: "source-a".to_string(),
                source_kind: "user".to_string(),
            },
            EvidenceSource {
                source_id: "source-b".to_string(),
                source_kind: "tool".to_string(),
            },
        ]
    }

    /// Three independent sources from 2+ distinct source_kinds.
    /// Satisfies the Rule minimum requirements.
    fn three_independent_sources() -> Vec<EvidenceSource> {
        vec![
            EvidenceSource {
                source_id: "source-a".to_string(),
                source_kind: "user".to_string(),
            },
            EvidenceSource {
                source_id: "source-b".to_string(),
                source_kind: "tool".to_string(),
            },
            EvidenceSource {
                source_id: "source-c".to_string(),
                source_kind: "user".to_string(),
            },
        ]
    }

    /// Insert an events_v2 row to use as a `created_event_id` provenance anchor.
    fn insert_event(db: &Arc<Database>) -> String {
        let event_id = new_id().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let hlc = format!(
            "{:016x}{:08x}",
            chrono::Utc::now().timestamp_millis() as u64,
            uuid::Uuid::now_v7().as_u128() as u32
        );
        let conn = db.write();
        conn.execute(
            "INSERT INTO events_v2 (
                 id, phase, hlc, ts_utc, tz_offset_min, event_type,
                 source_kind, source_id, actor_id,
                 namespace, owner_id, scope, sensitivity, policy_version,
                 payload_plain, payload_encoding, payload_checksum, schema_version)
             VALUES (?1,'start',?2,?3,0,'observation','user','src-1','actor-1',
                     'ns','owner-1','private',0,'v1','{}','utf8','chk',1)",
            params![event_id, hlc, now],
        )
        .unwrap();
        event_id
    }

    // ── Test 12: derive_record — summary from episodes has correct identity, links, truth ──

    #[test]
    fn derive_summary_from_episodes_has_correct_identity_links_and_truth() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        // Open episode and insert 3 'current' records.
        let spec = NewEpisode::simple("session-12", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();
        let (r1, _) = insert_record(&db, &ep_id);
        let (r2, _) = insert_record(&db, &ep_id);
        let (r3, _) = insert_record(&db, &ep_id);

        let mut record_ids = vec![r1.clone(), r2.clone(), r3.clone()];
        record_ids.sort();

        let event_id = insert_event(&db);
        let candidate_set =
            make_candidate_set(&db, &ep_id, record_ids.clone(), ConsolidationLevel::Summary);
        let spec_dr = DeriveRecordSpec {
            content: "Derived summary of 3 episodes".to_string(),
            created_event_id: event_id.clone(),
        };

        let result = derive_record(
            &db,
            &candidate_set,
            &spec_dr,
            &two_independent_sources(),
            None,
        )
        .unwrap();
        // (b) Truth state should be 'current' (all parents are current).
        assert_eq!(
            result.truth_state, "current",
            "all-current parents → current derived truth"
        );
        // (c) Semantic identity is deterministic.
        let expected_semantic_id = compute_semantic_output_id(
            ConsolidationLevel::Summary,
            CONSOLIDATION_ALGORITHM_VERSION,
            &record_ids,
        );
        assert_eq!(
            result.semantic_id, expected_semantic_id,
            "semantic identity must match"
        );
        // (d) Correct number of links (one per parent).
        assert_eq!(
            result.links_written, 3,
            "must create one derived_from link per parent"
        );
        // (e) Links exist in memory_links table.
        let link_count: i64 = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT COUNT(*) FROM memory_links WHERE target_id = ?1 AND link_type = 'derived_from'",
                        params![result.record_id],
                        |r| r.get(0),
                    )
                    .unwrap())
            })
            .unwrap();
        assert_eq!(
            link_count, 3,
            "all 3 derived_from links must be in memory_links"
        );
        // (f) Record exists with correct kind.
        let kind: String = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT record_kind FROM records WHERE id = ?1",
                        params![result.record_id],
                        |r| r.get(0),
                    )
                    .unwrap())
            })
            .unwrap();
        assert_eq!(kind, "summary", "derived record_kind must be 'summary'");
    }

    // ── Test 13: contradicted parent propagates contradicted truth ────────────

    #[test]
    fn contradicted_parent_propagates_contradicted_truth_state() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        let spec = NewEpisode::simple("session-13", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        let (r1, _) = insert_record(&db, &ep_id); // truth_state = 'current'
        let (r2, _) = insert_record_with_truth(&db, &ep_id, "contradicted");

        let mut record_ids = vec![r1, r2];
        record_ids.sort();

        let event_id = insert_event(&db);
        let candidate_set =
            make_candidate_set(&db, &ep_id, record_ids, ConsolidationLevel::Summary);
        let spec_dr = DeriveRecordSpec {
            content: "Summary with contradicted parent".to_string(),
            created_event_id: event_id,
        };

        let result = derive_record(
            &db,
            &candidate_set,
            &spec_dr,
            &two_independent_sources(),
            None,
        )
        .unwrap();
        assert_eq!(
            result.truth_state, "contradicted",
            "any contradicted parent must propagate contradicted to derived record"
        );
    }

    // ── Test 14: stale parent propagates stale truth state ────────────────────

    #[test]
    fn stale_parent_propagates_stale_truth_state() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        let spec = NewEpisode::simple("session-14", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        let (r1, _) = insert_record(&db, &ep_id); // truth_state = 'current'
        let (r2, _) = insert_record_with_truth(&db, &ep_id, "stale");

        let mut record_ids = vec![r1, r2];
        record_ids.sort();

        let event_id = insert_event(&db);
        let candidate_set = make_candidate_set(&db, &ep_id, record_ids, ConsolidationLevel::Skill);
        let spec_dr = DeriveRecordSpec {
            content: "Skill with stale parent".to_string(),
            created_event_id: event_id,
        };

        let result = derive_record(
            &db,
            &candidate_set,
            &spec_dr,
            &two_independent_sources(),
            None,
        )
        .unwrap();
        assert_eq!(
            result.truth_state, "stale",
            "any stale parent (no contradicted) must propagate stale to derived record"
        );
    }

    // ── Test 15: contradicted beats stale in truth propagation ───────────────

    #[test]
    fn contradicted_beats_stale_in_truth_propagation() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        let spec = NewEpisode::simple("session-15", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        let (r1, _) = insert_record_with_truth(&db, &ep_id, "stale");
        let (r2, _) = insert_record_with_truth(&db, &ep_id, "contradicted");

        let mut record_ids = vec![r1, r2];
        record_ids.sort();

        let event_id = insert_event(&db);
        let candidate_set =
            make_candidate_set(&db, &ep_id, record_ids, ConsolidationLevel::Summary);
        let spec_dr = DeriveRecordSpec {
            content: "Summary with both stale and contradicted parents".to_string(),
            created_event_id: event_id,
        };

        let result = derive_record(
            &db,
            &candidate_set,
            &spec_dr,
            &two_independent_sources(),
            None,
        )
        .unwrap();
        assert_eq!(
            result.truth_state, "contradicted",
            "contradicted must take precedence over stale"
        );
    }

    // ── Test 16: duplicate call with same inputs returns existing record (idempotency) ──

    #[test]
    fn duplicate_derive_call_returns_existing_record() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        let spec = NewEpisode::simple("session-16", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        let (r1, _) = insert_record(&db, &ep_id);
        let (r2, _) = insert_record(&db, &ep_id);
        let mut record_ids = vec![r1, r2];
        record_ids.sort();

        let event_id = insert_event(&db);

        // First call — creates the record.
        let candidate_set1 =
            make_candidate_set(&db, &ep_id, record_ids.clone(), ConsolidationLevel::Summary);
        let spec_dr = DeriveRecordSpec {
            content: "Derived summary".to_string(),
            created_event_id: event_id.clone(),
        };
        let first = derive_record(
            &db,
            &candidate_set1,
            &spec_dr,
            &two_independent_sources(),
            None,
        )
        .unwrap();
        assert!(!first.was_existing, "first call must create a new record");
        assert_eq!(first.links_written, 2, "first call must create 2 links");

        // Second call with identical inputs — returns existing.
        let candidate_set2 =
            make_candidate_set(&db, &ep_id, record_ids, ConsolidationLevel::Summary);
        let second = derive_record(
            &db,
            &candidate_set2,
            &spec_dr,
            &two_independent_sources(),
            None,
        )
        .unwrap();
        assert!(
            second.was_existing,
            "duplicate call must return existing record"
        );
        assert_eq!(
            first.record_id, second.record_id,
            "idempotent call must return the same record_id"
        );
        assert_eq!(
            first.semantic_id, second.semantic_id,
            "idempotent call must return the same semantic_id"
        );
        assert_eq!(second.links_written, 0, "duplicate call must write 0 links");
    }

    // ── Test 17: Rule-level compression ceiling is enforced ──────────────────

    #[test]
    fn rule_level_compression_ceiling_is_enforced_for_rule_parents() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        let spec = NewEpisode::simple("session-17", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        // Insert a 'rule'-kind parent record.
        let rule_record_id = new_id().to_string();
        let event_id = insert_event(&db);
        let now = chrono::Utc::now().to_rfc3339();
        {
            let conn = db.write();
            conn.execute(
                "INSERT INTO records (
                     id, record_kind, schema_version, content, content_hash,
                     truth_state, namespace, owner_id, scope, sensitivity,
                     source_id, policy_version, created_event_id, created_at, episode_id)
                 VALUES (?1,'rule',1,'a rule','rule-hash','current',
                         'ns','owner-1','private',0,'src-1','v1',?2,?3,?4)",
                params![rule_record_id, event_id, now, ep_id],
            )
            .unwrap();
        }

        let record_ids = vec![rule_record_id];
        let candidate_set = make_candidate_set(&db, &ep_id, record_ids, ConsolidationLevel::Rule);
        let spec_dr = DeriveRecordSpec {
            content: "Rule of Rule — should be rejected".to_string(),
            created_event_id: event_id,
        };

        // Must supply RuleEscalationApproval and valid sources to reach the
        // CompressionCeiling check (the approval + evidence gates fire first).
        let approval = RuleEscalationApproval::new("test-policy");
        let err = derive_record(
            &db,
            &candidate_set,
            &spec_dr,
            &three_independent_sources(),
            Some(&approval),
        )
        .unwrap_err();
        assert_eq!(
            err,
            DeriveRecordError::CompressionCeiling,
            "derive_record must refuse Rule-of-Rule compression"
        );
    }

    // ── Test 18: derive_record propagate_truth_state unit tests ──────────────

    #[test]
    fn propagate_truth_state_all_current_returns_current() {
        let states: Vec<String> = vec!["current".into(), "current".into()];
        assert_eq!(propagate_truth_state(&states), "current");
    }

    #[test]
    fn propagate_truth_state_any_contradicted_wins() {
        let states: Vec<String> = vec!["current".into(), "contradicted".into(), "stale".into()];
        assert_eq!(propagate_truth_state(&states), "contradicted");
    }

    #[test]
    fn propagate_truth_state_any_stale_no_contradicted() {
        let states: Vec<String> = vec!["current".into(), "stale".into()];
        assert_eq!(propagate_truth_state(&states), "stale");
    }

    #[test]
    fn propagate_truth_state_empty_returns_unverified() {
        let states: Vec<String> = vec![];
        assert_eq!(propagate_truth_state(&states), "unverified");
    }

    #[test]
    fn propagate_truth_state_mixed_unverified_returns_unverified() {
        let states: Vec<String> = vec!["current".into(), "unverified".into()];
        assert_eq!(propagate_truth_state(&states), "unverified");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // F3.6.5 — Evidence gate / source diversity / self-reflection cap tests
    // ═══════════════════════════════════════════════════════════════════════

    // ── Test 19: self-reflection confidence is capped at 0.6 ─────────────────

    /// Validates: Requirements MGR-035, MGR-038 (self-reflection trust cap, §F3.6.5)
    #[test]
    fn self_reflection_confidence_is_capped_at_reflection_cap() {
        use crate::cognition::REFLECTION_CONFIDENCE_CAP;

        // Confidence above the cap must be clamped to exactly 0.6.
        let capped = ConsolidationReadinessChecker::cap_confidence("self_reflection", 0.9);
        assert_eq!(
            capped, REFLECTION_CONFIDENCE_CAP,
            "self_reflection confidence 0.9 must be capped to {REFLECTION_CONFIDENCE_CAP}"
        );

        // Confidence already at the cap must be unchanged.
        let at_cap = ConsolidationReadinessChecker::cap_confidence("self_reflection", 0.6);
        assert_eq!(at_cap, 0.6, "confidence already at cap must be unchanged");

        // Confidence below the cap must pass through.
        let below_cap = ConsolidationReadinessChecker::cap_confidence("self_reflection", 0.4);
        assert_eq!(
            below_cap, 0.4,
            "confidence below cap must pass through unchanged"
        );

        // Non-reflection sources are never capped.
        let user_high = ConsolidationReadinessChecker::cap_confidence("user", 1.0);
        assert_eq!(
            user_high, 1.0,
            "user source confidence must never be capped"
        );

        let tool_high = ConsolidationReadinessChecker::cap_confidence("tool", 0.95);
        assert_eq!(
            tool_high, 0.95,
            "tool source confidence must never be capped"
        );
    }

    // ── Test 20: single source fails Skill minimum ────────────────────────────

    /// Validates: Requirements MGR-035 (minimum 2 independent sources for Skill, §F3.6.5)
    #[test]
    fn single_source_fails_skill_minimum() {
        let sources = vec![EvidenceSource {
            source_id: "source-a".to_string(),
            source_kind: "user".to_string(),
        }];
        let outcome =
            ConsolidationReadinessChecker::check_readiness(ConsolidationLevel::Skill, &sources);
        match outcome {
            ReadinessOutcome::Refused { reason } => {
                assert!(
                    reason.contains("1"),
                    "reason must mention the 1 source found; got: {reason}"
                );
                assert!(
                    reason.contains("2"),
                    "reason must mention the 2 required; got: {reason}"
                );
                assert!(
                    reason.contains("Skill"),
                    "reason must mention 'Skill'; got: {reason}"
                );
            }
            ReadinessOutcome::Ready => {
                panic!("single source must not pass Skill readiness gate")
            }
        }
    }

    // ── Test 21: two independent sources passes Skill minimum ─────────────────

    /// Validates: Requirements MGR-035 (minimum 2 independent sources for Skill, §F3.6.5)
    #[test]
    fn two_independent_sources_pass_skill_minimum() {
        let sources = two_independent_sources();
        let outcome =
            ConsolidationReadinessChecker::check_readiness(ConsolidationLevel::Skill, &sources);
        assert_eq!(
            outcome,
            ReadinessOutcome::Ready,
            "two independent non-reflection sources must pass Skill readiness gate"
        );
    }

    // ── Test 22: three independent sources from 2+ source_kinds passes Rule ──

    /// Validates: Requirements MGR-039 (Rule needs 3 independent sources + 2 kinds, §F3.6.5)
    #[test]
    fn three_independent_sources_with_two_kinds_pass_rule_minimum() {
        let sources = three_independent_sources(); // user + tool + user = 2 kinds, 3 distinct IDs
        let outcome =
            ConsolidationReadinessChecker::check_readiness(ConsolidationLevel::Rule, &sources);
        assert_eq!(
            outcome,
            ReadinessOutcome::Ready,
            "3 independent sources with 2 distinct source_kinds must pass Rule readiness gate"
        );
    }

    // ── Test 23: three independent sources with only 1 source_kind FAILS Rule ─

    /// Validates: Requirements MGR-039 (Rule needs ≥2 distinct source_kinds, §F3.6.5)
    #[test]
    fn three_independent_sources_with_one_kind_fails_rule_minimum() {
        // All three are "user" kind — only 1 distinct source_kind.
        let sources = vec![
            EvidenceSource {
                source_id: "source-a".to_string(),
                source_kind: "user".to_string(),
            },
            EvidenceSource {
                source_id: "source-b".to_string(),
                source_kind: "user".to_string(),
            },
            EvidenceSource {
                source_id: "source-c".to_string(),
                source_kind: "user".to_string(),
            },
        ];
        let outcome =
            ConsolidationReadinessChecker::check_readiness(ConsolidationLevel::Rule, &sources);
        match outcome {
            ReadinessOutcome::Refused { reason } => {
                assert!(
                    reason.contains("1"),
                    "reason must mention 1 kind found; got: {reason}"
                );
                assert!(
                    reason.contains("2"),
                    "reason must mention 2 required; got: {reason}"
                );
            }
            ReadinessOutcome::Ready => {
                panic!("single source_kind must not pass Rule readiness gate")
            }
        }
    }

    // ── Test 24: Rule derivation without RuleEscalationApproval fails ─────────

    /// Validates: Requirements MGR-039 (no automatic Rule escalation, §F3.6.5)
    #[test]
    fn rule_derivation_without_approval_fails_with_automatic_escalation_denied() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        let spec = NewEpisode::simple("session-24", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        let (r1, _) = insert_record(&db, &ep_id);
        let (r2, _) = insert_record(&db, &ep_id);
        let (r3, _) = insert_record(&db, &ep_id);
        let mut record_ids = vec![r1, r2, r3];
        record_ids.sort();

        let event_id = insert_event(&db);
        let candidate_set = make_candidate_set(&db, &ep_id, record_ids, ConsolidationLevel::Rule);
        let spec_dr = DeriveRecordSpec {
            content: "Rule derived without approval — should be denied".to_string(),
            created_event_id: event_id,
        };

        // Pass valid sources but NO RuleEscalationApproval.
        let err = derive_record(
            &db,
            &candidate_set,
            &spec_dr,
            &three_independent_sources(),
            None, // ← no approval
        )
        .unwrap_err();

        assert_eq!(
            err,
            DeriveRecordError::AutomaticRuleEscalationDenied,
            "Rule derivation without explicit RuleEscalationApproval must return AutomaticRuleEscalationDenied"
        );
    }

    // ── Test 25: false_promotion_reason is recorded in InsufficientEvidence ──

    /// Validates: Requirements MGR-035 (false-promotion reasons recorded, §F3.6.5)
    #[test]
    fn false_promotion_reason_is_recorded_when_promotion_refused() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        let spec = NewEpisode::simple("session-25", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();
        let (r1, _) = insert_record(&db, &ep_id);
        let record_ids = vec![r1];

        let event_id = insert_event(&db);
        // Targeting Skill with only ONE source — must be refused.
        let candidate_set = make_candidate_set(&db, &ep_id, record_ids, ConsolidationLevel::Skill);
        let spec_dr = DeriveRecordSpec {
            content: "Skill with insufficient evidence".to_string(),
            created_event_id: event_id,
        };

        let sole_source = vec![EvidenceSource {
            source_id: "only-one".to_string(),
            source_kind: "user".to_string(),
        }];

        let err = derive_record(&db, &candidate_set, &spec_dr, &sole_source, None).unwrap_err();

        match err {
            DeriveRecordError::InsufficientEvidence { reason } => {
                assert!(
                    !reason.is_empty(),
                    "false_promotion_reason must be a non-empty string"
                );
                // The reason must be human-readable and informative.
                assert!(
                    reason.contains("Skill") || reason.contains("independent"),
                    "reason should explain WHY promotion was refused; got: {reason}"
                );
            }
            other => panic!(
                "expected InsufficientEvidence with a false_promotion_reason, got: {other:?}"
            ),
        }
    }

    // ── Test 26: self_reflection-only sources refused for any level ───────────

    /// Validates: Requirements MGR-038 (self_reflection cannot be sole source, §F3.6.5)
    #[test]
    fn self_reflection_only_sources_refused_for_all_levels() {
        let reflection_only = vec![EvidenceSource {
            source_id: "reflect-1".to_string(),
            source_kind: "self_reflection".to_string(),
        }];

        for level in [
            ConsolidationLevel::Episode,
            ConsolidationLevel::Summary,
            ConsolidationLevel::Skill,
            ConsolidationLevel::Rule,
        ] {
            let outcome = ConsolidationReadinessChecker::check_readiness(level, &reflection_only);
            assert!(
                matches!(outcome, ReadinessOutcome::Refused { .. }),
                "self_reflection-only source must be refused for level {:?}",
                level
            );
        }
    }

    // ── Test 27: Rule derivation WITH approval and valid sources succeeds ─────

    /// Validates: Requirements MGR-039 (explicit RuleEscalationApproval allows Rule, §F3.6.5)
    #[test]
    fn rule_derivation_with_approval_and_valid_sources_succeeds() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        let spec = NewEpisode::simple("session-27", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        // Insert 3 non-rule records (so CompressionCeiling doesn't fire).
        let (r1, _) = insert_record(&db, &ep_id);
        let (r2, _) = insert_record(&db, &ep_id);
        let (r3, _) = insert_record(&db, &ep_id);
        let mut record_ids = vec![r1, r2, r3];
        record_ids.sort();

        let event_id = insert_event(&db);
        let candidate_set = make_candidate_set(&db, &ep_id, record_ids, ConsolidationLevel::Rule);
        let spec_dr = DeriveRecordSpec {
            content: "Valid rule derived with approval".to_string(),
            created_event_id: event_id,
        };

        let approval = RuleEscalationApproval::new("governance-policy")
            .with_reason("Three independent cross-kind sources validated");

        let result = derive_record(
            &db,
            &candidate_set,
            &spec_dr,
            &three_independent_sources(),
            Some(&approval),
        )
        .unwrap();

        assert!(
            !result.was_existing,
            "first Rule derivation with approval must create a new record"
        );
        assert_eq!(
            result.links_written, 3,
            "three derived_from links must be written for the 3 parent records"
        );
        assert_eq!(result.truth_state, "current");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // F3.6.6 — Durable resume / idempotency and stale propagation tests
    // ═══════════════════════════════════════════════════════════════════════

    // ── Helper: insert a derived_from link from parent to child ──────────────

    /// Insert a `derived_from` link in `memory_links` from `parent_id` to `derived_id`.
    fn insert_derived_from_link(db: &Arc<Database>, parent_id: &str, derived_id: &str) {
        let link_id = new_id().to_string();
        let event_id = insert_event(db);
        let conn = db.write();
        conn.execute(
            "INSERT OR IGNORE INTO memory_links (
                 id, source_kind, source_id, target_kind, target_id,
                 link_type, link_version, truth_state,
                 namespace, owner_id, scope, sensitivity, source_policy_id, policy_version,
                 created_event_id, revision
             ) VALUES (?1,'memory',?2,'memory',?3,'derived_from',1,'current',
                       'ns','owner-1','private',0,'src-1','v1',?4,1)",
            params![link_id, parent_id, derived_id, event_id],
        )
        .unwrap();
    }

    // ── Test 28: resume_or_create_run creates a new run when none exists ──────

    /// Validates: Requirements MGR-035 (durable resume, §F3.6.6)
    #[test]
    fn resume_or_create_run_creates_new_run_when_none_exists() {
        let db = Arc::new(Database::open_in_memory().unwrap());

        let hash = compute_input_set_hash(&["record-a".to_string(), "record-b".to_string()]);
        let outcome = resume_or_create_run(
            &db,
            CONSOLIDATION_ALGORITHM,
            CONSOLIDATION_ALGORITHM_VERSION,
            &hash,
            ConsolidationLevel::Summary,
        )
        .unwrap();

        match outcome {
            ResumeOrCreateOutcome::Created { run_id } => {
                assert!(!run_id.is_empty(), "created run_id must be non-empty");
                // Verify the row was actually written.
                let status: String = db
                    .with_read(|conn| {
                        Ok(conn
                            .query_row(
                                "SELECT status FROM consolidation_runs WHERE id = ?1",
                                params![run_id],
                                |r| r.get(0),
                            )
                            .unwrap())
                    })
                    .unwrap();
                assert_eq!(status, "pending", "new run must have status 'pending'");
            }
            other => panic!("expected Created, got {other:?}"),
        }
    }

    // ── Test 29: resume_or_create_run resumes a pending run (not a duplicate) ─

    /// Validates: Requirements MGR-035, MGR-039 (durable resume — crash recovery, §F3.6.6)
    #[test]
    fn resume_or_create_run_returns_existing_pending_run_not_duplicate() {
        let db = Arc::new(Database::open_in_memory().unwrap());

        let hash = compute_input_set_hash(&["record-x".to_string()]);
        // First call — creates the run.
        let first = resume_or_create_run(
            &db,
            CONSOLIDATION_ALGORITHM,
            CONSOLIDATION_ALGORITHM_VERSION,
            &hash,
            ConsolidationLevel::Episode,
        )
        .unwrap();
        let first_run_id = match first {
            ResumeOrCreateOutcome::Created { run_id } => run_id,
            other => panic!("expected Created on first call, got {other:?}"),
        };

        // Second call with the same identity key — must resume, not duplicate.
        let second = resume_or_create_run(
            &db,
            CONSOLIDATION_ALGORITHM,
            CONSOLIDATION_ALGORITHM_VERSION,
            &hash,
            ConsolidationLevel::Episode,
        )
        .unwrap();
        match second {
            ResumeOrCreateOutcome::Resumed { run_id } => {
                assert_eq!(
                    run_id, first_run_id,
                    "resumed run_id must equal the original pending run_id"
                );
            }
            other => panic!("expected Resumed on second call, got {other:?}"),
        }

        // Verify only ONE row exists in the table (no duplicate was inserted).
        let count: i64 = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT COUNT(*) FROM consolidation_runs \
                         WHERE algorithm = ?1 AND version = ?2 \
                           AND input_set_hash = ?3 AND level = ?4",
                        params![
                            CONSOLIDATION_ALGORITHM,
                            CONSOLIDATION_ALGORITHM_VERSION,
                            hash,
                            ConsolidationLevel::Episode.as_str(),
                        ],
                        |r| r.get(0),
                    )
                    .unwrap())
            })
            .unwrap();
        assert_eq!(
            count, 1,
            "must not create duplicate consolidation_runs rows"
        );
    }

    // ── Test 30: resume_or_create_run returns Completed for an existing completed run ──

    /// Validates: Requirements MGR-038 (idempotency, §F3.6.6)
    #[test]
    fn resume_or_create_run_returns_completed_for_completed_run() {
        let db = Arc::new(Database::open_in_memory().unwrap());

        let hash = compute_input_set_hash(&["record-y".to_string()]);
        let now = chrono::Utc::now().to_rfc3339();

        // Manually insert a completed run.
        let run_id = new_id().to_string();
        let output_id = new_id().to_string();
        {
            let conn = db.write();
            conn.execute(
                "INSERT INTO consolidation_runs
                     (id, algorithm, version, input_set_hash, level,
                      cursor, status, started_at, completed_at, output_id, error_code)
                 VALUES (?1,?2,?3,?4,'summary',NULL,'completed',?5,?5,?6,NULL)",
                params![
                    run_id,
                    CONSOLIDATION_ALGORITHM,
                    CONSOLIDATION_ALGORITHM_VERSION,
                    hash,
                    now,
                    output_id,
                ],
            )
            .unwrap();
        }

        let outcome = resume_or_create_run(
            &db,
            CONSOLIDATION_ALGORITHM,
            CONSOLIDATION_ALGORITHM_VERSION,
            &hash,
            ConsolidationLevel::Summary,
        )
        .unwrap();

        match outcome {
            ResumeOrCreateOutcome::Completed {
                run_id: returned_id,
                output_id: returned_output,
            } => {
                assert_eq!(
                    returned_id, run_id,
                    "Completed must return the existing run_id"
                );
                assert_eq!(
                    returned_output,
                    Some(output_id),
                    "Completed must return the existing output_id"
                );
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    // ── Test 31: propagate_stale marks a direct derived record stale ──────────

    /// Validates: Requirements MGR-045 (downstream stale propagation, §F3.6.6)
    #[test]
    fn propagate_stale_marks_direct_derived_record_stale() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        let spec = NewEpisode::simple("session-31", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        // source: the corrected parent record
        let (source_id, _) = insert_record(&db, &ep_id);
        // derived: a child summary derived from source
        let (derived_id, _) = insert_record(&db, &ep_id);
        insert_derived_from_link(&db, &source_id, &derived_id);

        // Verify derived record starts as 'current'.
        let before: String = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT truth_state FROM records WHERE id = ?1",
                        params![derived_id],
                        |r| r.get(0),
                    )
                    .unwrap())
            })
            .unwrap();
        assert_eq!(before, "current");

        let updated = propagate_stale_to_derived_records(&db, &source_id, 3).unwrap();

        assert_eq!(
            updated,
            vec![derived_id.clone()],
            "exactly the derived record must be updated"
        );

        // Verify the truth_state in the DB.
        let after: String = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT truth_state FROM records WHERE id = ?1",
                        params![derived_id],
                        |r| r.get(0),
                    )
                    .unwrap())
            })
            .unwrap();
        assert_eq!(after, "stale", "derived record must be marked stale");
    }

    // ── Test 32: propagate_stale is recursive up to depth 3 ──────────────────

    /// Validates: Requirements MGR-045 (recursive propagation bounded at depth 3, §F3.6.6)
    #[test]
    fn propagate_stale_is_recursive_up_to_depth_3() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        let spec = NewEpisode::simple("session-32", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        // Build a 4-level chain: episode → summary → skill → rule
        // episode is source_id (depth 0)
        let (episode_id, _) = insert_record(&db, &ep_id);
        let (summary_id, _) = insert_record(&db, &ep_id);
        let (skill_id, _) = insert_record(&db, &ep_id);
        let (rule_id, _) = insert_record(&db, &ep_id);
        // A record even deeper (depth 4) — must NOT be updated with max_depth=3.
        let (too_deep_id, _) = insert_record(&db, &ep_id);

        insert_derived_from_link(&db, &episode_id, &summary_id); // depth 1
        insert_derived_from_link(&db, &summary_id, &skill_id); // depth 2
        insert_derived_from_link(&db, &skill_id, &rule_id); // depth 3
        insert_derived_from_link(&db, &rule_id, &too_deep_id); // depth 4 — beyond limit

        let updated = propagate_stale_to_derived_records(&db, &episode_id, 3).unwrap();

        // Summary, Skill, Rule must be stale; too_deep must NOT be updated.
        assert!(
            updated.contains(&summary_id),
            "depth-1 summary must be marked stale"
        );
        assert!(
            updated.contains(&skill_id),
            "depth-2 skill must be marked stale"
        );
        assert!(
            updated.contains(&rule_id),
            "depth-3 rule must be marked stale"
        );
        assert!(
            !updated.contains(&too_deep_id),
            "depth-4 record must NOT be updated with max_depth=3"
        );
        assert_eq!(updated.len(), 3, "exactly 3 records must be marked stale");
    }

    // ── Test 33: terminal states are not re-marked stale ─────────────────────

    /// Validates: Requirements MGR-045 (contradicted/forgotten/deleted not overwritten, §F3.6.6)
    #[test]
    fn propagate_stale_does_not_overwrite_terminal_truth_states() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let mgr = manager(db.clone());

        let spec = NewEpisode::simple("session-33", "ns", "owner-1");
        let ep_id = spec.id.clone();
        mgr.open_episode(&spec).unwrap();

        let (source_id, _) = insert_record(&db, &ep_id);

        // Three derived records — each in a terminal state.
        let (contradicted_id, _) = insert_record_with_truth(&db, &ep_id, "contradicted");
        let (forgotten_id, _) = insert_record_with_truth(&db, &ep_id, "forgotten");
        let (deleted_id, _) = insert_record_with_truth(&db, &ep_id, "deleted");
        // One that should be updated.
        let (current_id, _) = insert_record(&db, &ep_id);

        insert_derived_from_link(&db, &source_id, &contradicted_id);
        insert_derived_from_link(&db, &source_id, &forgotten_id);
        insert_derived_from_link(&db, &source_id, &deleted_id);
        insert_derived_from_link(&db, &source_id, &current_id);

        let updated = propagate_stale_to_derived_records(&db, &source_id, 3).unwrap();

        // Only the 'current' record must be updated.
        assert_eq!(
            updated,
            vec![current_id],
            "only 'current' records must be marked stale"
        );

        // Terminal states must remain unchanged.
        for (id, expected_state) in [
            (&contradicted_id, "contradicted"),
            (&forgotten_id, "forgotten"),
            (&deleted_id, "deleted"),
        ] {
            let state: String = db
                .with_read(|conn| {
                    Ok(conn
                        .query_row(
                            "SELECT truth_state FROM records WHERE id = ?1",
                            params![id],
                            |r| r.get(0),
                        )
                        .unwrap())
                })
                .unwrap();
            assert_eq!(
                state, expected_state,
                "terminal state '{expected_state}' must not be overwritten by stale propagation"
            );
        }
    }

    // ── Test 34: non-existent parent returns empty set (no-op) ───────────────

    /// Validates: Requirements MGR-045 (no-op for non-existent parent, §F3.6.6)
    #[test]
    fn propagate_stale_for_nonexistent_parent_returns_empty_set() {
        let db = Arc::new(Database::open_in_memory().unwrap());

        // No records inserted; source ID is fictitious.
        let updated = propagate_stale_to_derived_records(&db, "nonexistent-id-xyz", 3).unwrap();

        assert!(
            updated.is_empty(),
            "propagating stale from a non-existent parent must return an empty set"
        );
    }
}
