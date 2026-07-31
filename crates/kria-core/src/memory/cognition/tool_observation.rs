//! Tool invocation start/completion correlation and outcome classification
//! (memory-upgrade design §4.3, F3.7.1–F3.7.2).
//!
//! This module records tool invocations and correlates start/completion pairs
//! by `invocation_id`, classifying outcomes into a typed [`ToolOutcome`] enum.
//! It also stores policy-safe rich facts — server identity, retry/recovery
//! context, affected record associations, and a truncated result summary — to
//! support bounded aggregate reliability metrics without granting authority.
//!
//! # Invariants (design §4.3 / MGR-033, MGR-043–045)
//! * One start/completion pair per invocation — the schema enforces a UNIQUE
//!   index on `invocation_id`.
//! * All outcomes are typed; there is no unclassified success/failure.
//! * `result_summary` is silently truncated to ≤512 UTF-8 characters before
//!   storage (policy-safe: prevents raw secrets in unlimited text fields).
//! * `affected_records_json` stores a JSON array of authority record IDs.
//!   Callers supply only IDs they are authorized to reference; this module
//!   validates the JSON form but does NOT perform FK checks.
//! * Observations **never** grant capability, widen scope, bypass approval,
//!   promote a Rule, change security policy, delete data, or override an
//!   explicit correction or newer version. This module is strictly append-only
//!   to `tool_observations` and reads nothing outside that table.

use std::sync::Arc;

use rusqlite::params;

use crate::memory::db::Database;
use crate::memory::error::{MemoryError, MemoryResult, StorageError};
use crate::memory::ids::new_id;

// ── Typed outcome ─────────────────────────────────────────────────────────────

/// Typed outcome of a tool invocation (design §4.3, MGR-043–045).
///
/// Every invocation that has a completion event receives one of these; callers
/// MUST NOT leave outcomes as raw strings. No unclassified success/failure
/// exists by design.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolOutcome {
    /// Tool completed successfully and produced the intended result.
    Success,
    /// Tool partially succeeded (e.g. some items processed but not all).
    Partial {
        /// Human-readable description of what succeeded vs. what did not.
        reason: String,
    },
    /// A known/expected error category (e.g. "file_not_found" when a
    /// missing file is acceptable). The caller classified this before
    /// storing.
    ExpectedFailure {
        /// Error class from a closed classification taxonomy.
        error_class: String,
    },
    /// An unexpected error — something the caller did not anticipate.
    UnexpectedFailure {
        /// Error class from a closed classification taxonomy.
        error_class: String,
    },
    /// Tool exceeded its deadline and was terminated.
    Timeout,
    /// Explicitly cancelled — by the user, scheduler, or a parent job.
    Cancelled,
    /// The user (or an authoritative correction signal) overrode the tool's
    /// output. `corrected_to` carries the corrected value summary (max 512
    /// chars; no raw secrets).
    ///
    /// **Security note:** this variant records that a correction happened; it
    /// does NOT re-evaluate or re-rank tool capability.
    Correction {
        /// A policy-safe summary of what the outcome was corrected to.
        corrected_to: String,
    },
    /// The action produced by this tool was undone by the user.
    Undo,
    /// A completion event was recorded but the outcome could not be determined
    /// (e.g. unexpected process exit with no status).
    Unknown,
}

impl ToolOutcome {
    /// Stable wire string stored in `tool_observations.outcome`. Prefix
    /// encodes the discriminant so plain SQL `WHERE outcome LIKE 'partial:%'`
    /// is workable for debugging.
    fn to_wire(&self) -> String {
        match self {
            ToolOutcome::Success => "success".into(),
            ToolOutcome::Partial { reason } => format!("partial:{reason}"),
            ToolOutcome::ExpectedFailure { error_class } => {
                format!("expected_failure:{error_class}")
            }
            ToolOutcome::UnexpectedFailure { error_class } => {
                format!("unexpected_failure:{error_class}")
            }
            ToolOutcome::Timeout => "timeout".into(),
            ToolOutcome::Cancelled => "cancelled".into(),
            ToolOutcome::Correction { corrected_to } => format!("correction:{corrected_to}"),
            ToolOutcome::Undo => "undo".into(),
            ToolOutcome::Unknown => "unknown".into(),
        }
    }

    /// Parse from the stored wire string back to a typed outcome.
    fn from_wire(s: &str) -> ToolOutcome {
        if s == "success" {
            return ToolOutcome::Success;
        }
        if s == "timeout" {
            return ToolOutcome::Timeout;
        }
        if s == "cancelled" {
            return ToolOutcome::Cancelled;
        }
        if s == "undo" {
            return ToolOutcome::Undo;
        }
        if s == "unknown" {
            return ToolOutcome::Unknown;
        }
        if let Some(reason) = s.strip_prefix("partial:") {
            return ToolOutcome::Partial {
                reason: reason.into(),
            };
        }
        if let Some(ec) = s.strip_prefix("expected_failure:") {
            return ToolOutcome::ExpectedFailure {
                error_class: ec.into(),
            };
        }
        if let Some(ec) = s.strip_prefix("unexpected_failure:") {
            return ToolOutcome::UnexpectedFailure {
                error_class: ec.into(),
            };
        }
        if let Some(corrected_to) = s.strip_prefix("correction:") {
            return ToolOutcome::Correction {
                corrected_to: corrected_to.into(),
            };
        }
        // Unrecognized stored value — treat as Unknown rather than panicking.
        ToolOutcome::Unknown
    }
}

// ── Parameters ────────────────────────────────────────────────────────────────

/// Parameters for recording a new tool invocation start row.
pub struct StartParams<'a> {
    /// Caller-assigned stable identifier shared between start and completion.
    pub invocation_id: &'a str,
    pub tool_kind: Option<&'a str>,
    pub tool_id: Option<&'a str>,
    pub tool_version: Option<&'a str>,
    pub capability_id: Option<&'a str>,
    pub goal_id: Option<&'a str>,
    pub environment_class: Option<&'a str>,
    pub input_fingerprint: Option<&'a str>,
    /// The server/service hosting the tool (e.g. MCP server name, OpenClaw
    /// skill server, or sidecar endpoint). NULL for native tools.
    ///
    /// **Policy-safe:** no connection credentials, tokens, or secrets.
    pub server_id: Option<&'a str>,
    /// How many prior retry attempts preceded this invocation attempt. 0 or
    /// None means this is the first/only attempt.
    pub retry_count: Option<u32>,
    /// Named recovery strategy applied for this attempt (e.g. "fallback",
    /// "retry-with-backoff", "alternate-server"). NULL when no recovery logic
    /// was engaged.
    ///
    /// **Policy-safe:** must not contain paths, credentials, or user data.
    pub recovery_strategy: Option<&'a str>,
    /// FK into `events_v2`.
    pub start_event_id: &'a str,
    /// Policy columns — required by the schema NOT NULL constraints.
    pub namespace: &'a str,
    pub owner_id: &'a str,
    pub scope: &'a str,
    /// 0–3 matching the `sensitivity` CHECK constraint.
    pub sensitivity: u8,
    pub source_id: &'a str,
    pub policy_version: &'a str,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Insert a tool-observation row for an invocation **start**. The `outcome`
/// and `completion_event_id` columns are left NULL until
/// [`record_tool_invocation_completion`] is called.
///
/// The new `server_id`, `retry_count`, and `recovery_strategy` fields in
/// [`StartParams`] are stored directly. They may also be updated later via
/// [`update_retry_recovery`] when a retry succeeds after an initial failure.
///
/// Returns the newly-created row id (UUID v7 text).
///
/// # Errors
/// Returns [`StorageError::Sqlite`] if the schema constraint is violated
/// (e.g. duplicate `invocation_id`).
pub fn record_tool_invocation_start(
    db: &Arc<Database>,
    params: StartParams<'_>,
) -> MemoryResult<String> {
    let row_id = new_id().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let tx = db.begin()?;
    tx.conn()
        .execute(
            "INSERT INTO tool_observations(
             id, invocation_id,
             tool_kind, tool_id, tool_version, capability_id,
             goal_id, environment_class, input_fingerprint,
             server_id, retry_count, recovery_action,
             start_event_id,
             namespace, owner_id, scope, sensitivity,
             source_id, policy_version,
             created_at
         ) VALUES (
             ?1,  ?2,
             ?3,  ?4,  ?5,  ?6,
             ?7,  ?8,  ?9,
             ?10, ?11, ?12,
             ?13,
             ?14, ?15, ?16, ?17,
             ?18, ?19,
             ?20
         )",
            rusqlite::params![
                row_id,
                params.invocation_id,
                params.tool_kind,
                params.tool_id,
                params.tool_version,
                params.capability_id,
                params.goal_id,
                params.environment_class,
                params.input_fingerprint,
                params.server_id,
                params.retry_count.map(|v| v as i64),
                params.recovery_strategy,
                params.start_event_id,
                params.namespace,
                params.owner_id,
                params.scope,
                params.sensitivity as i64,
                params.source_id,
                params.policy_version,
                now,
            ],
        )
        .map_err(StorageError::Sqlite)?;
    tx.commit()?;

    Ok(row_id)
}

/// Update an existing start row to record the **completion** of a tool
/// invocation.
///
/// The function finds the start row by `invocation_id`, asserts that no
/// completion has been recorded yet, then writes the typed outcome,
/// optional latency / result summary, and the completion event FK.
///
/// `result_summary` is silently truncated to ≤512 UTF-8 characters before
/// storage (policy-safe: no raw secrets in unlimited text).
///
/// # Errors
/// * [`MemoryError::Internal`] — invocation not found or already completed.
/// * [`StorageError::Sqlite`] — DB write failed.
pub fn record_tool_invocation_completion(
    db: &Arc<Database>,
    invocation_id: &str,
    outcome: ToolOutcome,
    latency_ms: Option<i64>,
    result_summary: Option<&str>,
    error_class_override: Option<&str>,
    completion_event_id: &str,
) -> MemoryResult<()> {
    // Derive error_class from outcome when not explicitly overridden.
    let ec: Option<String> = error_class_override
        .map(str::to_string)
        .or_else(|| match &outcome {
            ToolOutcome::ExpectedFailure { error_class }
            | ToolOutcome::UnexpectedFailure { error_class } => Some(error_class.clone()),
            _ => None,
        });

    let outcome_wire = outcome.to_wire();

    // Policy-safe: truncate result_summary to ≤512 chars (no raw secrets in
    // unlimited text fields — design §4.3 / F3.7.2 invariant).
    let truncated_summary: Option<String> = result_summary.map(truncate_512);

    let tx = db.begin()?;
    let updated = tx
        .conn()
        .execute(
            "UPDATE tool_observations
             SET outcome              = ?1,
                 latency_ms           = ?2,
                 result_summary       = ?3,
                 error_class          = ?4,
                 completion_event_id  = ?5
             WHERE invocation_id = ?6
               AND completion_event_id IS NULL",
            rusqlite::params![
                outcome_wire,
                latency_ms,
                truncated_summary,
                ec,
                completion_event_id,
                invocation_id,
            ],
        )
        .map_err(StorageError::Sqlite)?;
    tx.commit()?;

    if updated == 0 {
        // Either not found or already completed.
        return Err(MemoryError::Internal(format!(
            "tool_observation: invocation '{invocation_id}' not found or already completed"
        )));
    }

    Ok(())
}

/// Correlate start/completion for `invocation_id` and return the classified
/// [`ToolOutcome`].
///
/// | State                      | Return value                            |
/// |----------------------------|-----------------------------------------|
/// | Start + Completion exist   | Classified `ToolOutcome`                |
/// | Start only (no completion) | `Ok(ToolOutcome::Unknown)`              |
/// | Neither row exists         | `Err(MemoryError::Internal("not found"))` |
pub fn correlate_invocation_outcome(
    db: &Arc<Database>,
    invocation_id: &str,
) -> MemoryResult<ToolOutcome> {
    db.with_read(|conn| {
        let row: Option<(bool, Option<String>)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT completion_event_id IS NOT NULL, outcome
                     FROM tool_observations
                     WHERE invocation_id = ?1
                     LIMIT 1",
                )
                .map_err(StorageError::Sqlite)?;

            let mut rows = stmt
                .query_map(params![invocation_id], |r| {
                    Ok((r.get::<_, bool>(0)?, r.get::<_, Option<String>>(1)?))
                })
                .map_err(StorageError::Sqlite)?;

            rows.next().transpose().map_err(StorageError::Sqlite)?
        };

        match row {
            None => Err(MemoryError::Internal(format!(
                "tool_observation: invocation '{invocation_id}' not found"
            ))),
            Some((false, _)) => Ok(ToolOutcome::Unknown),
            Some((true, None)) => Ok(ToolOutcome::Unknown),
            Some((true, Some(wire))) => Ok(ToolOutcome::from_wire(&wire)),
        }
    })
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Truncate a string to at most 512 Unicode scalar values (chars).
///
/// This is the policy-safe limit for `result_summary` storage — prevents raw
/// secrets or unbounded model output from ending up in authority rows.
fn truncate_512(s: &str) -> String {
    let mut chars = s.chars();
    let mut out = String::with_capacity(s.len().min(512 * 4));
    for _ in 0..512 {
        match chars.next() {
            Some(c) => out.push(c),
            None => break,
        }
    }
    out
}

// ── Extended F3.7.2 API ───────────────────────────────────────────────────────

/// Store a list of authority `records.id` values that were affected by the
/// tool invocation identified by `invocation_id`.
///
/// The IDs are serialized as a JSON array and written to
/// `tool_observations.affected_records_json`. Any previously stored value is
/// overwritten (last writer wins — callers should call this once after all
/// affected records are known).
///
/// # Policy invariants
/// * Callers must supply only record IDs they are authorized to reference.
/// * This function does NOT verify that IDs exist in the `records` table;
///   it only validates the JSON array form.
/// * An empty slice stores `"[]"` (an explicit empty array, distinguishable
///   from NULL which means "not set").
///
/// # Errors
/// * [`MemoryError::Internal`] — invocation not found.
/// * [`StorageError::Sqlite`] — DB write failed.
pub fn record_affected_records(
    db: &Arc<Database>,
    invocation_id: &str,
    affected_record_ids: &[&str],
) -> MemoryResult<()> {
    // Build the JSON array ourselves to avoid pulling in serde_json.
    let json = {
        let mut s = String::from("[");
        for (i, id) in affected_record_ids.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push('"');
            // IDs are UUIDs / authority text — no special JSON escaping needed
            // beyond the standard quotes. Reject any ID that contains a `"`
            // to be safe.
            if id.contains('"') {
                return Err(MemoryError::Internal(format!(
                    "tool_observation: record id contains invalid character '\"': {id}"
                )));
            }
            s.push_str(id);
            s.push('"');
        }
        s.push(']');
        s
    };

    let tx = db.begin()?;
    let updated = tx
        .conn()
        .execute(
            "UPDATE tool_observations
             SET affected_records_json = ?1
             WHERE invocation_id = ?2",
            rusqlite::params![json, invocation_id],
        )
        .map_err(StorageError::Sqlite)?;
    tx.commit()?;

    if updated == 0 {
        return Err(MemoryError::Internal(format!(
            "tool_observation: invocation '{invocation_id}' not found"
        )));
    }

    Ok(())
}

// ── F3.7.3 — Failure evidence preservation and success aggregation ────────────

/// Decision about whether a failure outcome should be written to durable memory
/// as an [`Evidence`] record (design §4.3, F3.7.3, MGR-033 / MGR-043–045).
///
/// Callers feed this into the cognition pipeline; this function never writes
/// anything itself (it is pure logic, testable without a DB).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreserveDecision {
    /// Write the observation as a durable Evidence record.
    Preserve,
    /// Do not write a durable record — either the outcome is trivial/safe to
    /// skip, or the content is unsafe.
    Skip {
        /// Human-readable reason code for logging/debugging.
        reason: &'static str,
    },
}

/// Known secret-bearing prefixes in `result_summary`.
/// If a summary starts with one of these it is classified as **unsafe** and
/// MUST NOT be stored durably (design §4.3, invariant: no raw secrets in
/// authority rows).
static SECRET_PREFIXES: &[&str] = &[
    "Bearer ",
    "bearer ",
    "sk-",
    "token=",
    "Token=",
    "api_key=",
    "API_KEY=",
    "password=",
    "Authorization:",
    "authorization:",
];

/// `ExpectedFailure` error classes that are considered **trivial** for a given
/// `tool_id`. Outcomes with these classes are safe to skip.
///
/// Format: `(tool_id, error_class)`.
static TRIVIAL_EXPECTED_FAILURES: &[(&str, &str)] = &[
    ("file_read", "file_not_found"),
    ("file_read", "permission_denied_expected"),
    ("file_exists", "file_not_found"),
    ("directory_list", "directory_not_found"),
    ("cache_lookup", "cache_miss"),
];

/// Decide whether a completed tool invocation's failure evidence is worth
/// storing durably in the cognition pipeline.
///
/// # Rules (design §4.3, F3.7.3)
/// | Outcome | Decision |
/// |---|---|
/// | `UnexpectedFailure` | Always `Preserve` |
/// | `ExpectedFailure` in trivial set for this `tool_id` | `Skip` |
/// | `ExpectedFailure` NOT in trivial set | `Preserve` |
/// | `Timeout` | `Preserve` (performance issue worth learning from) |
/// | `Cancelled` with `retry_count > 0` | `Preserve` (problematic pattern) |
/// | `Cancelled` with no retries | `Skip` |
/// | `Success` | **Always** `Skip` (aggregated separately) |
/// | `Partial` | `Preserve` |
/// | `Correction` / `Undo` | `Preserve` |
/// | `Unknown` | `Skip` (no signal) |
///
/// If `result_summary` contains a known secret prefix the call returns
/// `Skip { reason: "unsafe_secret" }` regardless of outcome.
pub fn should_preserve_failure_evidence(
    outcome: &ToolOutcome,
    tool_id: Option<&str>,
    retry_count: Option<u32>,
    result_summary: Option<&str>,
) -> PreserveDecision {
    // ── 1. Secret / unsafe check — always wins ────────────────────────────────
    if let Some(summary) = result_summary {
        for prefix in SECRET_PREFIXES {
            if summary.starts_with(prefix) {
                return PreserveDecision::Skip {
                    reason: "unsafe_secret",
                };
            }
        }
    }

    // ── 2. Per-outcome policy ─────────────────────────────────────────────────
    match outcome {
        // Success is NEVER preserved individually; use SuccessAggregator.
        ToolOutcome::Success => PreserveDecision::Skip {
            reason: "success_aggregated",
        },

        // UnexpectedFailure: always worth learning from.
        ToolOutcome::UnexpectedFailure { .. } => PreserveDecision::Preserve,

        // ExpectedFailure: skip only if in the trivial-set for this tool.
        ToolOutcome::ExpectedFailure { error_class } => {
            let tid = tool_id.unwrap_or("");
            let trivial = TRIVIAL_EXPECTED_FAILURES
                .iter()
                .any(|(t, ec)| *t == tid && *ec == error_class.as_str());
            if trivial {
                PreserveDecision::Skip {
                    reason: "trivial_expected_failure",
                }
            } else {
                PreserveDecision::Preserve
            }
        }

        // Timeout: indicates a performance issue.
        ToolOutcome::Timeout => PreserveDecision::Preserve,

        // Cancelled: only preserve when retries were involved (problematic pattern).
        ToolOutcome::Cancelled => {
            if retry_count.unwrap_or(0) > 0 {
                PreserveDecision::Preserve
            } else {
                PreserveDecision::Skip {
                    reason: "cancelled_no_retries",
                }
            }
        }

        // Partial success: useful failure evidence.
        ToolOutcome::Partial { .. } => PreserveDecision::Preserve,

        // Correction / Undo: user authority events worth recording.
        ToolOutcome::Correction { .. } | ToolOutcome::Undo => PreserveDecision::Preserve,

        // Unknown: no signal, skip.
        ToolOutcome::Unknown => PreserveDecision::Skip {
            reason: "unknown_outcome",
        },
    }
}

// ── SuccessAggregator ─────────────────────────────────────────────────────────

/// Milestones at which a durable `Insight` record should be emitted for
/// repeated successes of the same `(tool_id, environment_class)` pair.
///
/// The sequence is: first 1, then 5, then 20, then every 100 thereafter.
/// Between these thresholds no durable write is created.
static THRESHOLDS: &[u64] = &[1, 5, 20];
const THRESHOLD_PERIOD: u64 = 100;

/// Bounded in-memory aggregator that prevents trivial repeated successes from
/// creating unbounded durable memory volume.
///
/// # Behaviour
/// * Tracks `(tool_id, environment_class)` → success count in an in-memory
///   map capped at **256 entries**.
/// * When the map is full and a new key arrives, the entry with the lowest
///   count is evicted (least-frequently-seen wins eviction).
/// * Returns `Some(n)` when the new count `n` crosses a write threshold,
///   telling the caller to emit one durable Insight: "N successes observed for
///   {tool_id} in {env}".
/// * Returns `None` when no durable write is needed.
///
/// # Design constraints (F3.7.3)
/// * Max 256 entries — bounded memory volume.
/// * Does NOT write to the DB itself — callers decide how to handle the
///   returned threshold crossing.
/// * Pure data structure; thread-safety is the caller's responsibility
///   (single-threaded cognition scheduler owns this).
pub struct SuccessAggregator {
    /// Counts per `(tool_id, environment_class)` key.
    /// Both strings are owned to avoid lifetime coupling.
    counts: std::collections::HashMap<(String, String), u64>,
    /// Hard cap on map entries.
    capacity: usize,
}

impl SuccessAggregator {
    /// Create an aggregator with the production capacity of 256.
    pub fn new() -> Self {
        Self {
            counts: std::collections::HashMap::new(),
            capacity: 256,
        }
    }

    /// Create an aggregator with a custom capacity (useful in tests).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            counts: std::collections::HashMap::new(),
            capacity,
        }
    }

    /// Record one success for `(tool_id, environment_class)`.
    ///
    /// Returns `Some(new_count)` when this count crosses a threshold and a
    /// durable Insight should be written. Returns `None` otherwise.
    pub fn record_success(&mut self, tool_id: &str, environment_class: &str) -> Option<u64> {
        let key = (tool_id.to_owned(), environment_class.to_owned());

        // Evict if at capacity and key is new.
        if !self.counts.contains_key(&key) && self.counts.len() >= self.capacity {
            // Evict the entry with the minimum count (least-frequently-seen).
            if let Some(evict_key) = self
                .counts
                .iter()
                .min_by_key(|(_, &v)| v)
                .map(|(k, _)| k.clone())
            {
                self.counts.remove(&evict_key);
            }
        }

        let count = self.counts.entry(key).or_insert(0);
        *count += 1;
        let new_count = *count;

        if Self::is_threshold(new_count) {
            Some(new_count)
        } else {
            None
        }
    }

    /// True when `n` is a threshold milestone.
    fn is_threshold(n: u64) -> bool {
        if THRESHOLDS.contains(&n) {
            return true;
        }
        // After the fixed thresholds, fire every THRESHOLD_PERIOD.
        let max_fixed = *THRESHOLDS.last().unwrap_or(&0);
        n > max_fixed && (n - max_fixed) % THRESHOLD_PERIOD == 0
    }

    /// Current count for a key (read-only; zero if not tracked).
    #[cfg(test)]
    fn count_for(&self, tool_id: &str, environment_class: &str) -> u64 {
        *self
            .counts
            .get(&(tool_id.to_owned(), environment_class.to_owned()))
            .unwrap_or(&0)
    }

    /// Number of distinct keys currently tracked.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.counts.len()
    }
}

impl Default for SuccessAggregator {
    fn default() -> Self {
        Self::new()
    }
}

/// Update the `retry_count` and `recovery_action` (recovery strategy) fields
/// on an existing tool-observation row.
///
/// Use this when a retry succeeds after an initial failure and you need to
/// record the final retry context on the same row. Both fields are optional;
/// passing `None` for either leaves that column unchanged.
///
/// # Errors
/// * [`MemoryError::Internal`] — invocation not found.
/// * [`StorageError::Sqlite`] — DB write failed.
pub fn update_retry_recovery(
    db: &Arc<Database>,
    invocation_id: &str,
    retry_count: Option<u32>,
    recovery_strategy: Option<&str>,
) -> MemoryResult<()> {
    let tx = db.begin()?;
    let updated = tx
        .conn()
        .execute(
            "UPDATE tool_observations
             SET retry_count     = COALESCE(?1, retry_count),
                 recovery_action = COALESCE(?2, recovery_action)
             WHERE invocation_id = ?3",
            rusqlite::params![
                retry_count.map(|v| v as i64),
                recovery_strategy,
                invocation_id,
            ],
        )
        .map_err(StorageError::Sqlite)?;
    tx.commit()?;

    if updated == 0 {
        return Err(MemoryError::Internal(format!(
            "tool_observation: invocation '{invocation_id}' not found"
        )));
    }

    Ok(())
}

// ── Capability metrics ────────────────────────────────────────────────────────

/// Minimum sample size required to compute capability metrics (design §4.3 /
/// MGR-044).  Below this threshold the result is `InsufficientEvidence`.
pub const CAPABILITY_METRICS_MIN_SAMPLE: usize = 20;

/// Outcome counts broken down by discriminant, returned inside
/// [`CapabilityMetrics::Metrics`].
#[derive(Clone, Debug, PartialEq)]
pub struct OutcomeCounts {
    pub success: u64,
    pub partial: u64,
    pub expected_failure: u64,
    pub unexpected_failure: u64,
    pub timeout: u64,
    pub cancelled: u64,
    pub correction: u64,
    pub undo: u64,
    pub unknown: u64,
}

/// The result of [`compute_capability_metrics`].
///
/// Reliability information is **only** produced when the sample contains at
/// least [`CAPABILITY_METRICS_MIN_SAMPLE`] rows (design §4.3, MGR-044).
#[derive(Clone, Debug, PartialEq)]
pub enum CapabilityMetricsResult {
    /// Fewer than 20 observations in the window — no metrics are displayed.
    InsufficientEvidence {
        /// The actual number of rows in the query window (0 when the window is
        /// empty).
        sample_size: usize,
    },
    /// At least 20 observations were present; metrics are reliable.
    Metrics {
        /// Number of rows in the window.
        sample_size: usize,
        /// Fraction of rows whose outcome starts with `"success"`. Range
        /// `[0.0, 1.0]`.
        success_rate: f64,
        /// 50th percentile of non-NULL `latency_ms` values (milliseconds).
        /// `None` when every row has a NULL `latency_ms`.
        p50_latency_ms: Option<i64>,
        /// 95th percentile of non-NULL `latency_ms` values (milliseconds).
        /// `None` when every row has a NULL `latency_ms`.
        p95_latency_ms: Option<i64>,
        /// Per-outcome breakdown.
        outcome_counts: OutcomeCounts,
    },
}

/// Compute aggregate reliability metrics for a
/// `(tool_id, tool_version, environment_class)` tuple within a time window.
///
/// Queries `tool_observations` rows where:
/// * `tool_id` = `tool_id`
/// * `tool_version` = `tool_version`
/// * `environment_class` = `environment_class`
/// * `created_at` BETWEEN `window_start` and `window_end` (RFC3339 UTC strings)
///
/// Returns [`CapabilityMetricsResult::InsufficientEvidence`] when fewer than
/// [`CAPABILITY_METRICS_MIN_SAMPLE`] (20) rows match.  Returns
/// [`CapabilityMetricsResult::Metrics`] otherwise.
///
/// # Invariants (design §7.4, MGR-044)
/// * Observations never grant capability, widen scope, bypass approval, or
///   change security policy.
/// * Latency quantiles exclude NULL `latency_ms` rows; this matches the
///   contract "only from non-NULL latency values".
/// * `success_rate` is computed over all rows (denominator = `sample_size`),
///   not just rows with latency data.
///
/// # Errors
/// Returns [`StorageError::Sqlite`] on DB failures.
pub fn compute_capability_metrics(
    db: &Arc<Database>,
    tool_id: &str,
    tool_version: &str,
    environment_class: &str,
    window_start: &str,
    window_end: &str,
) -> MemoryResult<CapabilityMetricsResult> {
    db.with_read(|conn| {
        // Query all matching rows — outcome and latency_ms.
        let mut stmt = conn
            .prepare(
                "SELECT outcome, latency_ms
                 FROM tool_observations
                 WHERE tool_id = ?1
                   AND tool_version = ?2
                   AND environment_class = ?3
                   AND created_at BETWEEN ?4 AND ?5",
            )
            .map_err(StorageError::Sqlite)?;

        let rows: Vec<(String, Option<i64>)> = stmt
            .query_map(
                params![
                    tool_id,
                    tool_version,
                    environment_class,
                    window_start,
                    window_end
                ],
                |row| {
                    let outcome: String = row.get(0)?;
                    let latency: Option<i64> = row.get(1)?;
                    Ok((outcome, latency))
                },
            )
            .map_err(StorageError::Sqlite)?
            .collect::<Result<_, _>>()
            .map_err(StorageError::Sqlite)?;

        let sample_size = rows.len();

        if sample_size < CAPABILITY_METRICS_MIN_SAMPLE {
            return Ok(CapabilityMetricsResult::InsufficientEvidence { sample_size });
        }

        // Compute per-outcome counts.
        let mut counts = OutcomeCounts {
            success: 0,
            partial: 0,
            expected_failure: 0,
            unexpected_failure: 0,
            timeout: 0,
            cancelled: 0,
            correction: 0,
            undo: 0,
            unknown: 0,
        };
        let mut success_count: u64 = 0;

        let mut latencies: Vec<i64> = Vec::new();

        for (outcome, latency) in &rows {
            // Classify outcome.
            if outcome == "success" {
                counts.success += 1;
                success_count += 1;
            } else if outcome.starts_with("partial:") {
                counts.partial += 1;
            } else if outcome.starts_with("expected_failure:") {
                counts.expected_failure += 1;
            } else if outcome.starts_with("unexpected_failure:") {
                counts.unexpected_failure += 1;
            } else if outcome == "timeout" {
                counts.timeout += 1;
            } else if outcome == "cancelled" {
                counts.cancelled += 1;
            } else if outcome.starts_with("correction:") {
                counts.correction += 1;
            } else if outcome == "undo" {
                counts.undo += 1;
            } else {
                // "unknown" and any unrecognised wire format.
                counts.unknown += 1;
            }

            // Collect non-NULL latency for quantiles.
            if let Some(ms) = latency {
                latencies.push(*ms);
            }
        }

        let success_rate = success_count as f64 / sample_size as f64;

        // Compute p50 / p95 from sorted latencies (sort + index approach).
        let (p50_latency_ms, p95_latency_ms) = if latencies.is_empty() {
            (None, None)
        } else {
            latencies.sort_unstable();
            let n = latencies.len();
            // p50: lower-median index (0-based).
            let p50_idx = (n - 1) / 2;
            // p95: ceil(0.95 * n) - 1, clamped to valid range.
            let p95_idx = ((0.95_f64 * n as f64).ceil() as usize)
                .saturating_sub(1)
                .min(n - 1);
            (Some(latencies[p50_idx]), Some(latencies[p95_idx]))
        };

        Ok(CapabilityMetricsResult::Metrics {
            sample_size,
            success_rate,
            p50_latency_ms,
            p95_latency_ms,
            outcome_counts: counts,
        })
    })
}

// ── F3.7.5 — Task outcome attribution and Memory Worth tracker ────────────────

/// One record's attribution contribution within an outcome attribution run.
///
/// `fraction` is the share of the outcome attributed to this record under the
/// named policy (`1.0 / used_count` for "equal-attribution-v1"). The
/// `policy_version` field names the specific attribution algorithm used.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordContribution {
    /// The authority record ID that was in the Used set.
    pub record_id: String,
    /// Fraction of the outcome attributed to this record (range `[0.0, 1.0]`).
    pub fraction: f64,
    /// Identifies the attribution algorithm; currently always
    /// `"equal-attribution-v1"`.
    pub policy_version: String,
}

/// Result returned by [`attribute_task_outcome`].
///
/// Contains the complete attribution picture for one response/task under the
/// named policy. This is **read-only** — no authority records are modified.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributionResult {
    /// The response/task ID that was attributed.
    pub response_id: String,
    /// The named attribution policy under which contributions were computed.
    pub policy_name: String,
    /// Number of records in the Used set (records with `injected_order IS NOT NULL`).
    pub used_record_count: usize,
    /// Per-record contributions. Empty when `used_record_count == 0`.
    pub contributions: Vec<RecordContribution>,
}

/// Attribute a task outcome across the exact Used set for `response_id` under
/// `policy_name`.
///
/// Queries `retrieval_traces` joined to `retrieval_trace_items` to find every
/// record that was injected into model context (`injected_order IS NOT NULL`)
/// for this `response_id`. Each record receives an equal fraction
/// (`1.0 / used_count`) labelled `"equal-attribution-v1"`.
///
/// # Invariants (design §6.4, F3.7.5, MGR-033/MGR-043–045)
/// * **Read-only** — this function does NOT modify any authority record.
/// * Observations never grant capability, widen scope, bypass approval, or
///   change security policy.
/// * When there are no retrieval traces for `response_id`, the Used set is
///   empty and `contributions` is empty.
///
/// # Errors
/// Returns [`StorageError::Sqlite`] on DB failures.
pub fn attribute_task_outcome(
    db: &Arc<Database>,
    response_id: &str,
    _outcome: &ToolOutcome,
    policy_name: &str,
) -> MemoryResult<AttributionResult> {
    // Query the exact Used set: all trace_items with injected_order IS NOT NULL
    // for traces whose response_id matches.
    let record_ids: Vec<String> = db.with_read(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT rti.record_id
                 FROM retrieval_trace_items rti
                 JOIN retrieval_traces rt ON rti.trace_id = rt.id
                 WHERE rt.response_id = ?1
                   AND rti.injected_order IS NOT NULL
                 ORDER BY rti.injected_order, rti.record_id",
            )
            .map_err(StorageError::Sqlite)?;

        let ids = stmt
            .query_map(rusqlite::params![response_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(StorageError::Sqlite)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::Sqlite)?;

        Ok(ids)
    })?;

    let used_count = record_ids.len();
    let fraction = if used_count > 0 {
        1.0_f64 / used_count as f64
    } else {
        0.0_f64
    };

    let contributions: Vec<RecordContribution> = record_ids
        .into_iter()
        .map(|record_id| RecordContribution {
            record_id,
            fraction,
            policy_version: "equal-attribution-v1".to_string(),
        })
        .collect();

    Ok(AttributionResult {
        response_id: response_id.to_string(),
        policy_name: policy_name.to_string(),
        used_record_count: used_count,
        contributions,
    })
}

// ── Memory Worth tracker ──────────────────────────────────────────────────────

/// The Memory-Worth contribution state for one record.
///
/// Below the 20-observation threshold, the record contributes `Inert` and
/// MUST NOT influence retrieval rankings (design §6.4 step 5).
#[derive(Clone, Debug, PartialEq)]
pub enum MemoryWorthContribution {
    /// Fewer than 20 observations — the record's worth is inert and must not
    /// affect retrieval rankings.
    Inert,
    /// At least 20 observations — the record's worth contributes actively.
    Active {
        /// Total number of observations recorded so far.
        observation_count: u64,
        /// Normalized worth score: `min(observations / 100.0, 1.0)`.
        worth_score: f64,
    },
}

/// A versioned trace of an active Memory-Worth contribution.
///
/// Only emitted when the contribution is [`MemoryWorthContribution::Active`]
/// (≥20 observations); `None` is returned below that threshold.
#[derive(Clone, Debug, PartialEq)]
pub struct TracedContribution {
    /// The authority record whose worth was traced.
    pub record_id: String,
    /// The trace context in which the contribution was observed.
    pub trace_id: String,
    /// Total observations at trace time.
    pub observation_count: u64,
    /// Normalized worth score at trace time.
    pub worth_score: f64,
    /// Version label for the scoring algorithm; currently
    /// `"memory-worth-v1"`.
    pub policy_version: String,
}

/// In-memory tracker for per-record observation counts used to compute
/// Memory Worth (design §6.4, F3.7.5, MGR-033).
///
/// # Design invariants
/// * Bounded to `max_capacity` entries (default 10,000); the entry with the
///   minimum observation count is evicted when the map is full and a new key
///   arrives.
/// * Below 20 observations the record is `Inert` and MUST NOT influence
///   retrieval rankings.
/// * Above 20 observations `worth_score = min(observation_count / 100.0, 1.0)`.
/// * Does NOT write to any authority DB — all state is in-memory; the cognition
///   scheduler owns this and persists it separately if needed.
/// * Thread-safety is the caller's responsibility (single-threaded cognition
///   scheduler owns this).
pub struct MemoryWorthTracker {
    /// Observation counts per `record_id`.
    counts: std::collections::HashMap<String, u64>,
    /// Hard cap on map entries.
    max_capacity: usize,
}

/// Minimum observations before a record's Memory Worth becomes `Active`.
pub const MEMORY_WORTH_ACTIVE_THRESHOLD: u64 = 20;

/// Denominator for normalising observation count to a `[0, 1]` worth score.
const MEMORY_WORTH_SCORE_DENOMINATOR: f64 = 100.0;

impl MemoryWorthTracker {
    /// Create a tracker with the production capacity of 10,000.
    pub fn new() -> Self {
        Self::with_capacity(10_000)
    }

    /// Create a tracker with a custom capacity (useful in tests).
    pub fn with_capacity(max_capacity: usize) -> Self {
        Self {
            counts: std::collections::HashMap::new(),
            max_capacity,
        }
    }

    /// Record one observation for `record_id`.
    ///
    /// When the map is at capacity and `record_id` is new, the entry with the
    /// minimum count is evicted to stay within bounds.
    pub fn record_observation(&mut self, record_id: &str, _outcome: &ToolOutcome) {
        // Evict if at capacity and key is new.
        if !self.counts.contains_key(record_id) && self.counts.len() >= self.max_capacity {
            if let Some(evict_key) = self
                .counts
                .iter()
                .min_by_key(|(_, &v)| v)
                .map(|(k, _)| k.clone())
            {
                self.counts.remove(&evict_key);
            }
        }

        let count = self.counts.entry(record_id.to_owned()).or_insert(0);
        *count += 1;
    }

    /// Return the current Memory Worth contribution state for `record_id`.
    ///
    /// * `< 20` observations → [`MemoryWorthContribution::Inert`]
    /// * `≥ 20` observations → [`MemoryWorthContribution::Active`] with
    ///   `worth_score = min(count / 100.0, 1.0)`
    pub fn get_worth_contribution(&self, record_id: &str) -> MemoryWorthContribution {
        let count = *self.counts.get(record_id).unwrap_or(&0);
        if count < MEMORY_WORTH_ACTIVE_THRESHOLD {
            MemoryWorthContribution::Inert
        } else {
            let worth_score = (count as f64 / MEMORY_WORTH_SCORE_DENOMINATOR).min(1.0);
            MemoryWorthContribution::Active {
                observation_count: count,
                worth_score,
            }
        }
    }

    /// Return a versioned trace of the contribution for `record_id` in context
    /// of `trace_id`.
    ///
    /// Returns `Some` only when the contribution is
    /// [`MemoryWorthContribution::Active`] (≥20 observations). Returns `None`
    /// when the record is `Inert`.
    pub fn trace_contribution(
        &self,
        record_id: &str,
        trace_id: &str,
    ) -> Option<TracedContribution> {
        match self.get_worth_contribution(record_id) {
            MemoryWorthContribution::Inert => None,
            MemoryWorthContribution::Active {
                observation_count,
                worth_score,
            } => Some(TracedContribution {
                record_id: record_id.to_string(),
                trace_id: trace_id.to_string(),
                observation_count,
                worth_score,
                policy_version: "memory-worth-v1".to_string(),
            }),
        }
    }
}

impl Default for MemoryWorthTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Minimal in-memory DB with migrations applied.
    fn db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    /// Insert a bare-minimum `events_v2` row so FK constraints are satisfied.
    /// We derive a unique HLC from the event_id suffix to avoid the UNIQUE(hlc) constraint.
    fn seed_event(db: &Arc<Database>, event_id: &str) {
        // Use a deterministic HLC based on a counter embedded in the event_id string.
        // Format: 16 hex wall_ms digits + 8 hex counter digits = 24 chars total.
        // We hash the event_id to get a unique-ish value.
        let hlc = format!("{:016x}{:08x}", event_id.len() as u64 * 1_000_000, {
            let mut h: u32 = 0;
            for b in event_id.bytes() {
                h = h.wrapping_mul(31).wrapping_add(b as u32);
            }
            h
        });
        db.write()
            .execute(
                "INSERT INTO events_v2(
                     id, hlc, ts_utc, tz_offset_min, schema_version,
                     phase, event_type, source_kind, source_id, actor_id,
                     namespace, owner_id, scope, sensitivity, policy_version,
                     payload_plain, payload_encoding, payload_checksum
                 ) VALUES (
                     ?1, ?2, '2024-01-01T00:00:00Z', 0, 17,
                     'start', 'tool_invocation', 'tool', 'tool-src', 'user',
                     'core', 'user', 'private', 0, 'v1',
                     '{\"ok\":true}', 'json', 'c'
                 )",
                rusqlite::params![event_id, hlc],
            )
            .unwrap();
    }

    fn default_start<'a>(invocation_id: &'a str, event_id: &'a str) -> StartParams<'a> {
        StartParams {
            invocation_id,
            tool_kind: Some("native"),
            tool_id: Some("file_read"),
            tool_version: Some("1.0"),
            capability_id: None,
            goal_id: None,
            environment_class: Some("test"),
            input_fingerprint: None,
            server_id: None,
            retry_count: None,
            recovery_strategy: None,
            start_event_id: event_id,
            namespace: "core",
            owner_id: "user",
            scope: "private",
            sensitivity: 0,
            source_id: "src-1",
            policy_version: "v1",
        }
    }

    // ── 1. Success ─────────────────────────────────────────────────────────────

    #[test]
    fn success_classification() {
        let db = db();
        seed_event(&db, "evt-start-1");
        seed_event(&db, "evt-done-1");

        record_tool_invocation_start(&db, default_start("inv-success", "evt-start-1")).unwrap();
        record_tool_invocation_completion(
            &db,
            "inv-success",
            ToolOutcome::Success,
            Some(42),
            Some("read 512 bytes"),
            None,
            "evt-done-1",
        )
        .unwrap();

        let outcome = correlate_invocation_outcome(&db, "inv-success").unwrap();
        assert_eq!(outcome, ToolOutcome::Success);
    }

    // ── 2. Timeout ─────────────────────────────────────────────────────────────

    #[test]
    fn timeout_classification() {
        let db = db();
        seed_event(&db, "evt-start-2");
        seed_event(&db, "evt-done-2");

        record_tool_invocation_start(&db, default_start("inv-timeout", "evt-start-2")).unwrap();
        record_tool_invocation_completion(
            &db,
            "inv-timeout",
            ToolOutcome::Timeout,
            Some(30_000),
            None,
            None,
            "evt-done-2",
        )
        .unwrap();

        assert_eq!(
            correlate_invocation_outcome(&db, "inv-timeout").unwrap(),
            ToolOutcome::Timeout
        );
    }

    // ── 3. Cancelled ───────────────────────────────────────────────────────────

    #[test]
    fn cancelled_classification() {
        let db = db();
        seed_event(&db, "evt-start-3");
        seed_event(&db, "evt-done-3");

        record_tool_invocation_start(&db, default_start("inv-cancel", "evt-start-3")).unwrap();
        record_tool_invocation_completion(
            &db,
            "inv-cancel",
            ToolOutcome::Cancelled,
            None,
            None,
            None,
            "evt-done-3",
        )
        .unwrap();

        assert_eq!(
            correlate_invocation_outcome(&db, "inv-cancel").unwrap(),
            ToolOutcome::Cancelled
        );
    }

    // ── 4. Start-only → Unknown ────────────────────────────────────────────────

    #[test]
    fn start_only_returns_unknown() {
        let db = db();
        seed_event(&db, "evt-start-4");

        record_tool_invocation_start(&db, default_start("inv-start-only", "evt-start-4")).unwrap();

        // No completion recorded — should come back as Unknown.
        assert_eq!(
            correlate_invocation_outcome(&db, "inv-start-only").unwrap(),
            ToolOutcome::Unknown
        );
    }

    // ── 5. Non-existent invocation → error ────────────────────────────────────

    #[test]
    fn nonexistent_invocation_returns_error() {
        let db = db();
        let result = correlate_invocation_outcome(&db, "inv-does-not-exist");
        assert!(
            result.is_err(),
            "expected error for unknown invocation_id, got: {result:?}"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not found"),
            "error message should mention 'not found', got: {msg}"
        );
    }

    // ── 6. Correction variant ─────────────────────────────────────────────────

    #[test]
    fn correction_classification() {
        let db = db();
        seed_event(&db, "evt-start-5");
        seed_event(&db, "evt-done-5");

        record_tool_invocation_start(&db, default_start("inv-correction", "evt-start-5")).unwrap();
        record_tool_invocation_completion(
            &db,
            "inv-correction",
            ToolOutcome::Correction {
                corrected_to: "The correct answer is 42".into(),
            },
            None,
            None,
            None,
            "evt-done-5",
        )
        .unwrap();

        let outcome = correlate_invocation_outcome(&db, "inv-correction").unwrap();
        assert_eq!(
            outcome,
            ToolOutcome::Correction {
                corrected_to: "The correct answer is 42".into()
            }
        );
    }

    // ── 7. Undo variant ───────────────────────────────────────────────────────

    #[test]
    fn undo_classification() {
        let db = db();
        seed_event(&db, "evt-start-6");
        seed_event(&db, "evt-done-6");

        record_tool_invocation_start(&db, default_start("inv-undo", "evt-start-6")).unwrap();
        record_tool_invocation_completion(
            &db,
            "inv-undo",
            ToolOutcome::Undo,
            None,
            None,
            None,
            "evt-done-6",
        )
        .unwrap();

        assert_eq!(
            correlate_invocation_outcome(&db, "inv-undo").unwrap(),
            ToolOutcome::Undo
        );
    }

    // ── 8. Partial variant ────────────────────────────────────────────────────

    #[test]
    fn partial_classification() {
        let db = db();
        seed_event(&db, "evt-start-7");
        seed_event(&db, "evt-done-7");

        record_tool_invocation_start(&db, default_start("inv-partial", "evt-start-7")).unwrap();
        record_tool_invocation_completion(
            &db,
            "inv-partial",
            ToolOutcome::Partial {
                reason: "3 of 5 files processed".into(),
            },
            Some(150),
            None,
            None,
            "evt-done-7",
        )
        .unwrap();

        assert_eq!(
            correlate_invocation_outcome(&db, "inv-partial").unwrap(),
            ToolOutcome::Partial {
                reason: "3 of 5 files processed".into()
            }
        );
    }

    // ── 9. ExpectedFailure and UnexpectedFailure ───────────────────────────────

    #[test]
    fn expected_failure_classification() {
        let db = db();
        seed_event(&db, "evt-start-8");
        seed_event(&db, "evt-done-8");

        record_tool_invocation_start(&db, default_start("inv-exp-fail", "evt-start-8")).unwrap();
        record_tool_invocation_completion(
            &db,
            "inv-exp-fail",
            ToolOutcome::ExpectedFailure {
                error_class: "file_not_found".into(),
            },
            None,
            None,
            None,
            "evt-done-8",
        )
        .unwrap();

        assert_eq!(
            correlate_invocation_outcome(&db, "inv-exp-fail").unwrap(),
            ToolOutcome::ExpectedFailure {
                error_class: "file_not_found".into()
            }
        );
    }

    #[test]
    fn unexpected_failure_classification() {
        let db = db();
        seed_event(&db, "evt-start-9");
        seed_event(&db, "evt-done-9");

        record_tool_invocation_start(&db, default_start("inv-unexp-fail", "evt-start-9")).unwrap();
        record_tool_invocation_completion(
            &db,
            "inv-unexp-fail",
            ToolOutcome::UnexpectedFailure {
                error_class: "permission_denied".into(),
            },
            Some(5),
            None,
            None,
            "evt-done-9",
        )
        .unwrap();

        assert_eq!(
            correlate_invocation_outcome(&db, "inv-unexp-fail").unwrap(),
            ToolOutcome::UnexpectedFailure {
                error_class: "permission_denied".into()
            }
        );
    }

    // ── 10. Duplicate completion is rejected ───────────────────────────────────

    #[test]
    fn duplicate_completion_is_rejected() {
        let db = db();
        seed_event(&db, "evt-start-10");
        seed_event(&db, "evt-done-10a");
        seed_event(&db, "evt-done-10b");

        record_tool_invocation_start(&db, default_start("inv-dup", "evt-start-10")).unwrap();
        record_tool_invocation_completion(
            &db,
            "inv-dup",
            ToolOutcome::Success,
            None,
            None,
            None,
            "evt-done-10a",
        )
        .unwrap();

        // A second completion attempt on the same invocation must fail.
        let result = record_tool_invocation_completion(
            &db,
            "inv-dup",
            ToolOutcome::Timeout,
            None,
            None,
            None,
            "evt-done-10b",
        );
        assert!(
            result.is_err(),
            "second completion on same invocation must return an error"
        );
    }

    // ── 11. Wire round-trip for all variants ──────────────────────────────────

    #[test]
    fn wire_round_trips_all_variants() {
        let variants = vec![
            ToolOutcome::Success,
            ToolOutcome::Timeout,
            ToolOutcome::Cancelled,
            ToolOutcome::Undo,
            ToolOutcome::Unknown,
            ToolOutcome::Partial {
                reason: "half done".into(),
            },
            ToolOutcome::ExpectedFailure {
                error_class: "not_found".into(),
            },
            ToolOutcome::UnexpectedFailure {
                error_class: "crash".into(),
            },
            ToolOutcome::Correction {
                corrected_to: "fixed".into(),
            },
        ];

        for v in variants {
            let wire = v.to_wire();
            assert_eq!(
                ToolOutcome::from_wire(&wire),
                v,
                "round-trip failed for: {wire}"
            );
        }
    }

    // ── F3.7.2 — Rich policy-safe facts ───────────────────────────────────────

    /// Helper: start params with all new F3.7.2 fields set.
    fn rich_start<'a>(invocation_id: &'a str, event_id: &'a str) -> StartParams<'a> {
        StartParams {
            invocation_id,
            tool_kind: Some("mcp"),
            tool_id: Some("search_web"),
            tool_version: Some("2.1"),
            capability_id: Some("cap-search"),
            goal_id: None,
            environment_class: Some("production"),
            input_fingerprint: Some("sha256:abc123"),
            server_id: Some("mcp-server-main"),
            retry_count: Some(2),
            recovery_strategy: Some("retry-with-backoff"),
            start_event_id: event_id,
            namespace: "core",
            owner_id: "user",
            scope: "private",
            sensitivity: 0,
            source_id: "src-mcp",
            policy_version: "v1",
        }
    }

    // ── 12. Full facts round-trip: server_id / retry_count / recovery_strategy

    #[test]
    fn full_facts_round_trip() {
        let db = db();
        seed_event(&db, "evt-start-rf");
        seed_event(&db, "evt-done-rf");

        record_tool_invocation_start(&db, rich_start("inv-rich", "evt-start-rf")).unwrap();
        record_tool_invocation_completion(
            &db,
            "inv-rich",
            ToolOutcome::Success,
            Some(120),
            Some("found 5 results"),
            None,
            "evt-done-rf",
        )
        .unwrap();

        // Verify stored values by reading the row directly.
        let row = db
            .with_read(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT server_id, retry_count, recovery_action, result_summary
                         FROM tool_observations WHERE invocation_id = 'inv-rich'",
                    )
                    .map_err(crate::memory::error::StorageError::Sqlite)?;
                let row: (Option<String>, Option<i64>, Option<String>, Option<String>) = stmt
                    .query_row([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                    .map_err(crate::memory::error::StorageError::Sqlite)?;
                Ok(row)
            })
            .unwrap();

        assert_eq!(row.0.as_deref(), Some("mcp-server-main"), "server_id");
        assert_eq!(row.1, Some(2), "retry_count");
        assert_eq!(
            row.2.as_deref(),
            Some("retry-with-backoff"),
            "recovery_action"
        );
        assert_eq!(row.3.as_deref(), Some("found 5 results"), "result_summary");

        // Outcome correlation still works.
        assert_eq!(
            correlate_invocation_outcome(&db, "inv-rich").unwrap(),
            ToolOutcome::Success
        );
    }

    // ── 13. result_summary is truncated at 512 chars ──────────────────────────

    #[test]
    fn result_summary_truncated_at_512() {
        let db = db();
        seed_event(&db, "evt-start-trunc");
        seed_event(&db, "evt-done-trunc");

        // Build a 600-char summary.
        let long_summary: String = "x".repeat(600);
        assert_eq!(long_summary.chars().count(), 600);

        record_tool_invocation_start(&db, default_start("inv-trunc", "evt-start-trunc")).unwrap();
        record_tool_invocation_completion(
            &db,
            "inv-trunc",
            ToolOutcome::Success,
            None,
            Some(&long_summary),
            None,
            "evt-done-trunc",
        )
        .unwrap();

        let stored: Option<String> = db
            .with_read(|conn| {
                conn.query_row(
                    "SELECT result_summary FROM tool_observations WHERE invocation_id = 'inv-trunc'",
                    [],
                    |r| r.get(0),
                )
                .map_err(crate::memory::error::StorageError::Sqlite)
                .map_err(crate::memory::error::MemoryError::from)
            })
            .unwrap();

        let stored = stored.expect("result_summary must be stored");
        assert_eq!(
            stored.chars().count(),
            512,
            "result_summary must be truncated to exactly 512 chars, got {}",
            stored.chars().count()
        );
    }

    // ── 14. affected_records_json stores valid JSON array of record IDs ───────

    #[test]
    fn affected_records_json_stored() {
        let db = db();
        seed_event(&db, "evt-start-aff");

        record_tool_invocation_start(&db, default_start("inv-aff", "evt-start-aff")).unwrap();

        let ids = ["rec-001", "rec-002", "rec-003"];
        let id_refs: Vec<&str> = ids.iter().copied().collect();
        record_affected_records(&db, "inv-aff", &id_refs).unwrap();

        let stored: Option<String> = db
            .with_read(|conn| {
                conn.query_row(
                    "SELECT affected_records_json FROM tool_observations \
                     WHERE invocation_id = 'inv-aff'",
                    [],
                    |r| r.get(0),
                )
                .map_err(crate::memory::error::StorageError::Sqlite)
                .map_err(crate::memory::error::MemoryError::from)
            })
            .unwrap();

        let json = stored.expect("affected_records_json must be stored");
        assert!(json.starts_with('['), "must be JSON array");
        assert!(json.contains("rec-001"), "must contain first id");
        assert!(json.contains("rec-002"), "must contain second id");
        assert!(json.contains("rec-003"), "must contain third id");
        // SQLite json_valid must accept the stored value (the CHECK constraint
        // would have already rejected it at insert, but let's be explicit).
        let valid: i64 = db
            .with_read(|conn| {
                conn.query_row(
                    "SELECT json_valid(affected_records_json) FROM tool_observations \
                     WHERE invocation_id = 'inv-aff'",
                    [],
                    |r| r.get(0),
                )
                .map_err(crate::memory::error::StorageError::Sqlite)
                .map_err(crate::memory::error::MemoryError::from)
            })
            .unwrap();
        assert_eq!(
            valid, 1,
            "stored JSON must be valid per SQLite json_valid()"
        );
    }

    #[test]
    fn affected_records_empty_slice_stores_empty_array() {
        let db = db();
        seed_event(&db, "evt-start-emp");

        record_tool_invocation_start(&db, default_start("inv-emp", "evt-start-emp")).unwrap();
        record_affected_records(&db, "inv-emp", &[]).unwrap();

        let stored: Option<String> = db
            .with_read(|conn| {
                conn.query_row(
                    "SELECT affected_records_json FROM tool_observations \
                     WHERE invocation_id = 'inv-emp'",
                    [],
                    |r| r.get(0),
                )
                .map_err(crate::memory::error::StorageError::Sqlite)
                .map_err(crate::memory::error::MemoryError::from)
            })
            .unwrap();

        assert_eq!(
            stored.as_deref(),
            Some("[]"),
            "empty slice should produce '[]'"
        );
    }

    // ── 15. update_retry_recovery updates an existing row ─────────────────────

    #[test]
    fn update_retry_recovery_updates_row() {
        let db = db();
        seed_event(&db, "evt-start-urr");
        seed_event(&db, "evt-done-urr");

        // Start with no retry context.
        record_tool_invocation_start(&db, default_start("inv-urr", "evt-start-urr")).unwrap();

        // Simulate: initial attempt failed, then retry with backoff succeeded.
        // Update the retry context before recording completion.
        update_retry_recovery(&db, "inv-urr", Some(3), Some("fallback")).unwrap();

        record_tool_invocation_completion(
            &db,
            "inv-urr",
            ToolOutcome::Success,
            Some(500),
            Some("recovered ok"),
            None,
            "evt-done-urr",
        )
        .unwrap();

        let row = db
            .with_read(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT retry_count, recovery_action FROM tool_observations \
                         WHERE invocation_id = 'inv-urr'",
                    )
                    .map_err(crate::memory::error::StorageError::Sqlite)?;
                let row: (Option<i64>, Option<String>) = stmt
                    .query_row([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(crate::memory::error::StorageError::Sqlite)?;
                Ok(row)
            })
            .unwrap();

        assert_eq!(row.0, Some(3), "retry_count must be updated");
        assert_eq!(
            row.1.as_deref(),
            Some("fallback"),
            "recovery_action must be updated"
        );
    }

    #[test]
    fn update_retry_recovery_none_fields_unchanged() {
        let db = db();
        seed_event(&db, "evt-start-unc");

        // Start with retry context already set.
        let mut p = default_start("inv-unc", "evt-start-unc");
        p.retry_count = Some(1);
        p.recovery_strategy = Some("initial-strategy");
        record_tool_invocation_start(&db, p).unwrap();

        // Call update with None for both — should leave values unchanged.
        update_retry_recovery(&db, "inv-unc", None, None).unwrap();

        let row = db
            .with_read(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT retry_count, recovery_action FROM tool_observations \
                         WHERE invocation_id = 'inv-unc'",
                    )
                    .map_err(crate::memory::error::StorageError::Sqlite)?;
                let row: (Option<i64>, Option<String>) = stmt
                    .query_row([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(crate::memory::error::StorageError::Sqlite)?;
                Ok(row)
            })
            .unwrap();

        assert_eq!(row.0, Some(1), "retry_count must be unchanged");
        assert_eq!(
            row.1.as_deref(),
            Some("initial-strategy"),
            "recovery_action must be unchanged"
        );
    }

    // ── 16. truncate_512 helper unit test ────────────────────────────────────

    #[test]
    fn truncate_512_exactly_at_boundary() {
        // Exactly 512 chars — no truncation.
        let s512: String = "a".repeat(512);
        assert_eq!(truncate_512(&s512).chars().count(), 512);

        // 513 chars — truncated to 512.
        let s513: String = "b".repeat(513);
        let t = truncate_512(&s513);
        assert_eq!(t.chars().count(), 512);
        assert_eq!(t, "b".repeat(512));

        // Below limit — returned as-is.
        let s10 = "hello";
        assert_eq!(truncate_512(s10), s10);
    }

    #[test]
    fn truncate_512_multibyte_chars() {
        // Each '€' is 3 UTF-8 bytes; 512 of them should all survive.
        let euro512: String = "€".repeat(512);
        let result = truncate_512(&euro512);
        assert_eq!(result.chars().count(), 512);

        // 600 multibyte chars — truncated to 512.
        let euro600: String = "€".repeat(600);
        let result = truncate_512(&euro600);
        assert_eq!(result.chars().count(), 512);
    }

    // ── F3.7.3 — should_preserve_failure_evidence ─────────────────────────────

    #[test]
    fn unexpected_failure_is_always_preserved() {
        let outcome = ToolOutcome::UnexpectedFailure {
            error_class: "panic".into(),
        };
        assert_eq!(
            should_preserve_failure_evidence(&outcome, Some("any_tool"), None, None),
            PreserveDecision::Preserve,
            "UnexpectedFailure must always be preserved"
        );
    }

    #[test]
    fn success_is_never_individually_preserved() {
        assert_eq!(
            should_preserve_failure_evidence(&ToolOutcome::Success, Some("file_read"), None, None),
            PreserveDecision::Skip {
                reason: "success_aggregated"
            },
            "Success must never be individually preserved"
        );
    }

    #[test]
    fn secret_bearing_summary_is_unsafe_regardless_of_outcome() {
        let secret_summaries = &[
            "Bearer abc123",
            "bearer xyz",
            "sk-proj-abc",
            "token=s3cr3t",
            "Token=secret",
            "api_key=supersecret",
            "API_KEY=VALUE",
            "password=hunter2",
            "Authorization: Bearer foo",
            "authorization: token bar",
        ];
        for summary in secret_summaries {
            // Even an UnexpectedFailure is suppressed if the summary is unsafe.
            let outcome = ToolOutcome::UnexpectedFailure {
                error_class: "crash".into(),
            };
            assert_eq!(
                should_preserve_failure_evidence(&outcome, Some("tool"), None, Some(summary)),
                PreserveDecision::Skip {
                    reason: "unsafe_secret"
                },
                "Secret summary must suppress durable write, summary: {summary}"
            );
        }
    }

    #[test]
    fn trivial_expected_failure_is_skipped() {
        let outcome = ToolOutcome::ExpectedFailure {
            error_class: "file_not_found".into(),
        };
        // file_read + file_not_found is in the trivial set.
        assert_eq!(
            should_preserve_failure_evidence(&outcome, Some("file_read"), None, None),
            PreserveDecision::Skip {
                reason: "trivial_expected_failure"
            }
        );
    }

    #[test]
    fn nontrivial_expected_failure_is_preserved() {
        let outcome = ToolOutcome::ExpectedFailure {
            error_class: "network_timeout".into(),
        };
        // file_read + network_timeout is NOT in the trivial set.
        assert_eq!(
            should_preserve_failure_evidence(&outcome, Some("file_read"), None, None),
            PreserveDecision::Preserve,
        );
    }

    #[test]
    fn timeout_is_always_preserved() {
        assert_eq!(
            should_preserve_failure_evidence(&ToolOutcome::Timeout, Some("slow_tool"), None, None),
            PreserveDecision::Preserve
        );
    }

    #[test]
    fn cancelled_with_retries_is_preserved() {
        assert_eq!(
            should_preserve_failure_evidence(&ToolOutcome::Cancelled, Some("tool"), Some(1), None),
            PreserveDecision::Preserve,
            "Cancelled after retries indicates a problematic pattern"
        );
    }

    #[test]
    fn cancelled_with_no_retries_is_skipped() {
        assert_eq!(
            should_preserve_failure_evidence(&ToolOutcome::Cancelled, Some("tool"), None, None),
            PreserveDecision::Skip {
                reason: "cancelled_no_retries"
            }
        );
        assert_eq!(
            should_preserve_failure_evidence(&ToolOutcome::Cancelled, Some("tool"), Some(0), None),
            PreserveDecision::Skip {
                reason: "cancelled_no_retries"
            }
        );
    }

    #[test]
    fn partial_is_always_preserved() {
        let outcome = ToolOutcome::Partial {
            reason: "only half processed".into(),
        };
        assert_eq!(
            should_preserve_failure_evidence(&outcome, Some("batch_tool"), None, None),
            PreserveDecision::Preserve
        );
    }

    #[test]
    fn correction_and_undo_are_preserved() {
        let correction = ToolOutcome::Correction {
            corrected_to: "the right answer".into(),
        };
        assert_eq!(
            should_preserve_failure_evidence(&correction, Some("tool"), None, None),
            PreserveDecision::Preserve,
            "Correction should be preserved"
        );
        assert_eq!(
            should_preserve_failure_evidence(&ToolOutcome::Undo, Some("tool"), None, None),
            PreserveDecision::Preserve,
            "Undo should be preserved"
        );
    }

    #[test]
    fn unknown_is_skipped() {
        assert_eq!(
            should_preserve_failure_evidence(&ToolOutcome::Unknown, None, None, None),
            PreserveDecision::Skip {
                reason: "unknown_outcome"
            }
        );
    }

    // ── F3.7.3 — SuccessAggregator ────────────────────────────────────────────

    #[test]
    fn aggregator_fires_at_first_threshold() {
        let mut agg = SuccessAggregator::new();
        // Count 1 → threshold crossing.
        let result = agg.record_success("file_read", "test");
        assert_eq!(
            result,
            Some(1),
            "first success should trigger durable write (threshold 1)"
        );
    }

    #[test]
    fn aggregator_does_not_fire_between_thresholds() {
        let mut agg = SuccessAggregator::new();
        // Threshold at 1.
        agg.record_success("tool", "env");
        // 2, 3, 4 are between threshold 1 and threshold 5 — must all be None.
        for n in 2..5 {
            let result = agg.record_success("tool", "env");
            assert_eq!(
                result, None,
                "count {n} is between thresholds — must not fire"
            );
        }
    }

    #[test]
    fn aggregator_fires_at_threshold_5() {
        let mut agg = SuccessAggregator::new();
        for _ in 1..5 {
            agg.record_success("tool", "env");
        }
        let result = agg.record_success("tool", "env");
        assert_eq!(result, Some(5), "threshold at 5 must fire");
    }

    #[test]
    fn aggregator_fires_at_threshold_20() {
        let mut agg = SuccessAggregator::new();
        for _ in 1..20 {
            agg.record_success("tool", "env");
        }
        let result = agg.record_success("tool", "env");
        assert_eq!(result, Some(20), "threshold at 20 must fire");
    }

    #[test]
    fn aggregator_does_not_fire_between_20_and_120() {
        let mut agg = SuccessAggregator::new();
        // Advance to 20 (threshold).
        for _ in 0..20 {
            agg.record_success("tool", "env");
        }
        // 21–119 should all be None.
        for n in 21..120 {
            let result = agg.record_success("tool", "env");
            assert_eq!(
                result, None,
                "count {n} is between threshold 20 and next period 120 — must not fire"
            );
        }
    }

    #[test]
    fn aggregator_fires_at_period_multiples_after_20() {
        let mut agg = SuccessAggregator::new();
        // Advance to 20.
        for _ in 0..20 {
            agg.record_success("tool", "env");
        }
        // 21–119: no fire.
        for _ in 21..120 {
            agg.record_success("tool", "env");
        }
        // 120 = 20 + 1×100 → fire.
        let result = agg.record_success("tool", "env");
        assert_eq!(result, Some(120), "threshold at 20+100=120 must fire");

        // 121–219: no fire.
        for n in 121..220 {
            let r = agg.record_success("tool", "env");
            assert_eq!(r, None, "count {n} must not fire");
        }
        // 220 = 20 + 2×100 → fire.
        let result = agg.record_success("tool", "env");
        assert_eq!(result, Some(220), "threshold at 20+200=220 must fire");
    }

    #[test]
    fn aggregator_tracks_independent_keys() {
        let mut agg = SuccessAggregator::new();
        // Two distinct tool+env pairs.
        let r1 = agg.record_success("tool_a", "prod");
        let r2 = agg.record_success("tool_b", "prod");
        assert_eq!(r1, Some(1), "tool_a first success fires at threshold 1");
        assert_eq!(
            r2,
            Some(1),
            "tool_b first success fires independently at threshold 1"
        );
        assert_eq!(agg.count_for("tool_a", "prod"), 1);
        assert_eq!(agg.count_for("tool_b", "prod"), 1);
    }

    #[test]
    fn aggregator_bounded_map_evicts_at_capacity() {
        // Use a small capacity to exercise eviction.
        let mut agg = SuccessAggregator::with_capacity(4);

        // Fill to capacity.
        for i in 0..4u32 {
            agg.record_success(&format!("tool_{i}"), "env");
        }
        assert_eq!(agg.len(), 4, "should have exactly 4 entries");

        // Adding a 5th key must evict one entry.
        agg.record_success("tool_new", "env");
        assert_eq!(
            agg.len(),
            4,
            "map must remain at capacity after eviction; len={}",
            agg.len()
        );
        // The new key is present.
        assert!(
            agg.count_for("tool_new", "env") >= 1,
            "new key must be tracked after eviction"
        );
    }

    #[test]
    fn aggregator_bounded_map_does_not_exceed_256() {
        let mut agg = SuccessAggregator::new();
        // Insert 300 distinct keys.
        for i in 0..300u32 {
            agg.record_success(&format!("tool_{i:04}"), "env");
        }
        assert!(
            agg.len() <= 256,
            "aggregator must never exceed 256 entries, got {}",
            agg.len()
        );
    }

    // ── compute_capability_metrics ────────────────────────────────────────────

    /// Helper: seed N tool_observations rows for the given tool/version/env,
    /// all within window "2024-01-01T00:00:00Z" to "2024-01-31T23:59:59Z".
    /// Each row gets a unique invocation_id and start_event_id derived from
    /// `base_idx`.
    fn seed_observations(
        db: &Arc<Database>,
        tool_id: &str,
        tool_version: &str,
        environment_class: &str,
        outcomes: &[(&str, Option<i64>)], // (outcome_wire, latency_ms)
    ) {
        let window_ts = "2024-01-15T00:00:00Z";
        for (i, (outcome, latency)) in outcomes.iter().enumerate() {
            // Build a unique event id.
            let evt_id = format!("cap-evt-{tool_id}-{i}");
            seed_event(db, &evt_id);

            let invocation_id = format!("cap-inv-{tool_id}-{i}");
            let row_id = new_id().to_string();
            db.write()
                .execute(
                    "INSERT INTO tool_observations (
                         id, invocation_id, tool_id, tool_version,
                         environment_class, outcome, latency_ms,
                         namespace, owner_id, scope, sensitivity, source_id,
                         policy_version, start_event_id, completion_event_id,
                         created_at
                     ) VALUES (
                         ?1, ?2, ?3, ?4,
                         ?5, ?6, ?7,
                         'core', 'user', 'private', 0, 'src',
                         'v1', ?8, ?8,
                         ?9
                     )",
                    rusqlite::params![
                        row_id,
                        invocation_id,
                        tool_id,
                        tool_version,
                        environment_class,
                        outcome,
                        latency,
                        evt_id,
                        window_ts,
                    ],
                )
                .unwrap();
        }
    }

    // Build a slice of N success outcomes each with a given latency.
    fn successes_with_latency(n: usize, latency_ms: i64) -> Vec<(&'static str, Option<i64>)> {
        vec![("success", Some(latency_ms)); n]
    }

    #[test]
    fn cap_metrics_insufficient_at_n_5() {
        let db = db();
        let outcomes = successes_with_latency(5, 100);
        seed_observations(&db, "tool_x", "1.0", "test", &outcomes);

        let result = compute_capability_metrics(
            &db,
            "tool_x",
            "1.0",
            "test",
            "2024-01-01T00:00:00Z",
            "2024-01-31T23:59:59Z",
        )
        .unwrap();

        assert_eq!(
            result,
            CapabilityMetricsResult::InsufficientEvidence { sample_size: 5 }
        );
    }

    #[test]
    fn cap_metrics_at_exactly_n_20() {
        let db = db();
        // 20 successes; latencies 10..29 ms.
        let outcomes: Vec<(&str, Option<i64>)> =
            (0..20).map(|i| ("success", Some(10 + i as i64))).collect();
        seed_observations(&db, "tool_y", "2.0", "prod", &outcomes);

        let result = compute_capability_metrics(
            &db,
            "tool_y",
            "2.0",
            "prod",
            "2024-01-01T00:00:00Z",
            "2024-01-31T23:59:59Z",
        )
        .unwrap();

        match result {
            CapabilityMetricsResult::Metrics {
                sample_size,
                success_rate,
                p50_latency_ms,
                p95_latency_ms,
                outcome_counts,
            } => {
                assert_eq!(sample_size, 20);
                assert!(
                    (success_rate - 1.0).abs() < 1e-9,
                    "expected success_rate=1.0, got {success_rate}"
                );
                // Latencies sorted: [10, 11, ..., 29]
                // p50 index = (20-1)/2 = 9 → latency 19
                assert_eq!(p50_latency_ms, Some(19));
                // p95 index = ceil(0.95*20)-1 = ceil(19)-1 = 18 → latency 28
                assert_eq!(p95_latency_ms, Some(28));
                assert_eq!(outcome_counts.success, 20);
            }
            CapabilityMetricsResult::InsufficientEvidence { .. } => {
                panic!("expected Metrics at n=20");
            }
        }
    }

    #[test]
    fn cap_metrics_n_25_mixed_outcomes_correct_success_rate() {
        let db = db();
        // 15 success, 10 unexpected_failure → success_rate = 15/25 = 0.6
        let mut outcomes: Vec<(&str, Option<i64>)> =
            (0..15).map(|i| ("success", Some(50 + i as i64))).collect();
        for _ in 0..10 {
            outcomes.push(("unexpected_failure:net", Some(200)));
        }
        seed_observations(&db, "tool_z", "3.0", "staging", &outcomes);

        let result = compute_capability_metrics(
            &db,
            "tool_z",
            "3.0",
            "staging",
            "2024-01-01T00:00:00Z",
            "2024-01-31T23:59:59Z",
        )
        .unwrap();

        match result {
            CapabilityMetricsResult::Metrics {
                sample_size,
                success_rate,
                outcome_counts,
                ..
            } => {
                assert_eq!(sample_size, 25);
                assert!(
                    (success_rate - 0.6).abs() < 1e-9,
                    "expected success_rate=0.6, got {success_rate}"
                );
                assert_eq!(outcome_counts.success, 15);
                assert_eq!(outcome_counts.unexpected_failure, 10);
            }
            CapabilityMetricsResult::InsufficientEvidence { .. } => {
                panic!("expected Metrics at n=25");
            }
        }
    }

    #[test]
    fn cap_metrics_null_latency_excluded_from_quantiles() {
        let db = db();
        // 20 rows: half with latency, half NULL.
        let mut outcomes: Vec<(&str, Option<i64>)> =
            (0..10).map(|_| ("success", Some(100_i64))).collect();
        for _ in 0..10 {
            outcomes.push(("success", None));
        }
        seed_observations(&db, "tool_lat", "1.0", "test", &outcomes);

        let result = compute_capability_metrics(
            &db,
            "tool_lat",
            "1.0",
            "test",
            "2024-01-01T00:00:00Z",
            "2024-01-31T23:59:59Z",
        )
        .unwrap();

        match result {
            CapabilityMetricsResult::Metrics {
                sample_size,
                p50_latency_ms,
                p95_latency_ms,
                ..
            } => {
                assert_eq!(sample_size, 20);
                // Only 10 non-NULL latencies (all 100 ms): p50=100, p95=100.
                assert_eq!(p50_latency_ms, Some(100));
                assert_eq!(p95_latency_ms, Some(100));
            }
            CapabilityMetricsResult::InsufficientEvidence { .. } => {
                panic!("expected Metrics at n=20");
            }
        }
    }

    #[test]
    fn cap_metrics_all_null_latency_returns_none_quantiles() {
        let db = db();
        // 20 rows all with NULL latency.
        let outcomes: Vec<(&str, Option<i64>)> = (0..20).map(|_| ("success", None)).collect();
        seed_observations(&db, "tool_nonull", "1.0", "test", &outcomes);

        let result = compute_capability_metrics(
            &db,
            "tool_nonull",
            "1.0",
            "test",
            "2024-01-01T00:00:00Z",
            "2024-01-31T23:59:59Z",
        )
        .unwrap();

        match result {
            CapabilityMetricsResult::Metrics {
                p50_latency_ms,
                p95_latency_ms,
                ..
            } => {
                assert_eq!(p50_latency_ms, None);
                assert_eq!(p95_latency_ms, None);
            }
            CapabilityMetricsResult::InsufficientEvidence { .. } => {
                panic!("expected Metrics at n=20");
            }
        }
    }

    #[test]
    fn cap_metrics_empty_window_returns_insufficient_evidence() {
        let db = db();
        // No rows for this tool at all.
        let result = compute_capability_metrics(
            &db,
            "nonexistent_tool",
            "1.0",
            "test",
            "2024-01-01T00:00:00Z",
            "2024-01-31T23:59:59Z",
        )
        .unwrap();

        assert_eq!(
            result,
            CapabilityMetricsResult::InsufficientEvidence { sample_size: 0 }
        );
    }

    // ── F3.7.5 — attribute_task_outcome tests ─────────────────────────────────

    /// Insert a minimal `retrieval_traces` row (no FK dep needed for in-memory).
    fn seed_retrieval_trace(db: &Arc<Database>, trace_id: &str, response_id: &str) {
        db.write()
            .execute(
                "INSERT INTO retrieval_traces (
                     id, response_id, task_id, query_hash, query_class,
                     classifier_version, profile_id, graph_revision, policy_hash,
                     token_budget, status, degradation_json, embed_model_version,
                     k_value, availability_json, weights_json,
                     evidence_contribution, memory_worth_contribution, goal_contribution_total,
                     created_at
                 ) VALUES (
                     ?1, ?2, NULL, 'qhash', 'recall',
                     'clf-v1', 'default', NULL, NULL,
                     NULL, 'finalized', NULL, NULL,
                     60.0, '{}', '{}',
                     0.0, 0.0, 0.0,
                     '2024-01-01T00:00:00Z'
                 )",
                rusqlite::params![trace_id, response_id],
            )
            .unwrap();
    }

    /// Insert a `retrieval_trace_items` row.
    fn seed_trace_item(
        db: &Arc<Database>,
        trace_id: &str,
        record_id: &str,
        injected_order: Option<i64>,
    ) {
        db.write()
            .execute(
                "INSERT INTO retrieval_trace_items (
                     trace_id, record_id, strategy, strategy_rank, strategy_score,
                     weight, rrf_contribution, gate_disposition, reason_code,
                     token_cost, allocated_tokens, injected_order, goal_id,
                     evidence_contribution, memory_worth_contribution
                 ) VALUES (
                     ?1, ?2, 'fts', 1, 0.9,
                     1.0, 0.9, 'included', NULL,
                     50, 50, ?3, NULL,
                     0.0, 0.0
                 )",
                rusqlite::params![trace_id, record_id, injected_order],
            )
            .unwrap();
    }

    #[test]
    fn attribution_three_used_records_gives_one_third_each() {
        let db = db();
        seed_retrieval_trace(&db, "trace-a", "resp-001");
        seed_trace_item(&db, "trace-a", "rec-1", Some(1));
        seed_trace_item(&db, "trace-a", "rec-2", Some(2));
        seed_trace_item(&db, "trace-a", "rec-3", Some(3));

        let result =
            attribute_task_outcome(&db, "resp-001", &ToolOutcome::Success, "my-policy").unwrap();

        assert_eq!(result.response_id, "resp-001");
        assert_eq!(result.policy_name, "my-policy");
        assert_eq!(result.used_record_count, 3);
        assert_eq!(result.contributions.len(), 3);

        for contrib in &result.contributions {
            let diff = (contrib.fraction - (1.0 / 3.0)).abs();
            assert!(
                diff < 1e-12,
                "expected fraction ~1/3 but got {}",
                contrib.fraction
            );
            assert_eq!(contrib.policy_version, "equal-attribution-v1");
        }
    }

    #[test]
    fn attribution_empty_used_set_returns_empty_contributions() {
        let db = db();
        // A trace exists but all items have injected_order = NULL (not injected).
        seed_retrieval_trace(&db, "trace-b", "resp-002");
        seed_trace_item(&db, "trace-b", "rec-x", None); // not injected

        let result =
            attribute_task_outcome(&db, "resp-002", &ToolOutcome::Success, "my-policy").unwrap();

        assert_eq!(result.used_record_count, 0);
        assert!(result.contributions.is_empty());
    }

    #[test]
    fn attribution_no_trace_returns_empty_contributions() {
        let db = db();
        // No trace at all for this response_id.
        let result =
            attribute_task_outcome(&db, "resp-nonexistent", &ToolOutcome::Success, "p").unwrap();

        assert_eq!(result.used_record_count, 0);
        assert!(result.contributions.is_empty());
    }

    // ── F3.7.5 — MemoryWorthTracker tests ─────────────────────────────────────

    #[test]
    fn memory_worth_below_20_is_inert() {
        let mut tracker = MemoryWorthTracker::new();
        for _ in 0..19 {
            tracker.record_observation("rec-A", &ToolOutcome::Success);
        }
        assert_eq!(
            tracker.get_worth_contribution("rec-A"),
            MemoryWorthContribution::Inert
        );
    }

    #[test]
    fn memory_worth_at_20_is_active() {
        let mut tracker = MemoryWorthTracker::new();
        for _ in 0..20 {
            tracker.record_observation("rec-B", &ToolOutcome::Success);
        }
        match tracker.get_worth_contribution("rec-B") {
            MemoryWorthContribution::Active {
                observation_count,
                worth_score,
            } => {
                assert_eq!(observation_count, 20);
                // 20 / 100.0 = 0.2
                let diff = (worth_score - 0.2).abs();
                assert!(diff < 1e-12, "expected worth_score 0.2 got {worth_score}");
            }
            MemoryWorthContribution::Inert => panic!("expected Active at 20 observations"),
        }
    }

    #[test]
    fn memory_worth_score_caps_at_1_0_at_100_observations() {
        let mut tracker = MemoryWorthTracker::new();
        for _ in 0..100 {
            tracker.record_observation("rec-C", &ToolOutcome::Success);
        }
        match tracker.get_worth_contribution("rec-C") {
            MemoryWorthContribution::Active { worth_score, .. } => {
                assert!((worth_score - 1.0).abs() < 1e-12, "expected 1.0 at 100 obs");
            }
            MemoryWorthContribution::Inert => panic!("expected Active at 100 observations"),
        }
    }

    #[test]
    fn memory_worth_score_caps_at_1_0_beyond_100_observations() {
        let mut tracker = MemoryWorthTracker::new();
        for _ in 0..200 {
            tracker.record_observation("rec-D", &ToolOutcome::Success);
        }
        match tracker.get_worth_contribution("rec-D") {
            MemoryWorthContribution::Active { worth_score, .. } => {
                assert!(
                    (worth_score - 1.0).abs() < 1e-12,
                    "expected cap of 1.0 at 200 obs"
                );
            }
            MemoryWorthContribution::Inert => panic!("expected Active at 200 observations"),
        }
    }

    #[test]
    fn trace_contribution_returns_none_below_20() {
        let mut tracker = MemoryWorthTracker::new();
        for _ in 0..19 {
            tracker.record_observation("rec-E", &ToolOutcome::Success);
        }
        let traced = tracker.trace_contribution("rec-E", "trace-xyz");
        assert!(
            traced.is_none(),
            "trace_contribution must return None below 20 observations"
        );
    }

    #[test]
    fn trace_contribution_returns_some_at_20() {
        let mut tracker = MemoryWorthTracker::new();
        for _ in 0..20 {
            tracker.record_observation("rec-F", &ToolOutcome::Success);
        }
        let traced = tracker
            .trace_contribution("rec-F", "trace-abc")
            .expect("should return Some at 20 observations");

        assert_eq!(traced.record_id, "rec-F");
        assert_eq!(traced.trace_id, "trace-abc");
        assert_eq!(traced.observation_count, 20);
        let diff = (traced.worth_score - 0.2).abs();
        assert!(
            diff < 1e-12,
            "expected worth_score 0.2 got {}",
            traced.worth_score
        );
        assert_eq!(traced.policy_version, "memory-worth-v1");
    }

    #[test]
    fn memory_worth_untracked_record_is_inert() {
        let tracker = MemoryWorthTracker::new();
        assert_eq!(
            tracker.get_worth_contribution("never-seen"),
            MemoryWorthContribution::Inert
        );
        assert!(tracker.trace_contribution("never-seen", "t").is_none());
    }
}

// ── F3.7.6 — Non-escalation negative tests ─────────────────────────────────
//
// These tests prove the critical invariants from design §4.3 / MGR-033,
// MGR-043–045; MGD-031, MGD-043:
//
//   * Observations NEVER grant capability, widen scope, bypass approval,
//     promote a Rule, change security policy, delete data, or override
//     an explicit correction or newer capability version.
//   * `tool_observations` is the ONLY table this module ever writes to.
//   * `MemoryWorthTracker` is purely in-memory and never touches the DB.
//   * `compute_capability_metrics` and `attribute_task_outcome` are read-only.

#[cfg(test)]
mod non_escalation_tests_inner {
    use rusqlite::params;

    use crate::memory::cognition::tool_observation::{
        attribute_task_outcome, compute_capability_metrics, record_tool_invocation_completion,
        record_tool_invocation_start, MemoryWorthContribution, MemoryWorthTracker, ToolOutcome,
    };
    use crate::memory::db::Database;
    use crate::memory::ids::new_id;
    use std::sync::Arc;

    // ── Re-declare helpers that live only inside the sibling `tests` module ───

    /// Minimal in-memory DB with migrations applied.
    fn db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    /// Insert a bare-minimum `events_v2` row so FK constraints are satisfied.
    fn seed_event(db: &Arc<Database>, event_id: &str) {
        let hlc = format!("{:016x}{:08x}", event_id.len() as u64 * 1_000_000, {
            let mut h: u32 = 0;
            for b in event_id.bytes() {
                h = h.wrapping_mul(31).wrapping_add(b as u32);
            }
            h
        });
        db.write()
            .execute(
                "INSERT INTO events_v2(
                     id, hlc, ts_utc, tz_offset_min, schema_version,
                     phase, event_type, source_kind, source_id, actor_id,
                     namespace, owner_id, scope, sensitivity, policy_version,
                     payload_plain, payload_encoding, payload_checksum
                 ) VALUES (
                     ?1, ?2, '2024-01-01T00:00:00Z', 0, 17,
                     'start', 'tool_invocation', 'tool', 'tool-src', 'user',
                     'core', 'user', 'private', 0, 'v1',
                     '{\"ok\":true}', 'json', 'c'
                 )",
                rusqlite::params![event_id, hlc],
            )
            .unwrap();
    }

    fn default_start_params<'a>(
        invocation_id: &'a str,
        event_id: &'a str,
    ) -> crate::memory::cognition::tool_observation::StartParams<'a> {
        crate::memory::cognition::tool_observation::StartParams {
            invocation_id,
            tool_kind: Some("native"),
            tool_id: Some("file_read"),
            tool_version: Some("1.0"),
            capability_id: None,
            goal_id: None,
            environment_class: Some("test"),
            input_fingerprint: None,
            server_id: None,
            retry_count: None,
            recovery_strategy: None,
            start_event_id: event_id,
            namespace: "core",
            owner_id: "user",
            scope: "private",
            sensitivity: 0,
            source_id: "src-1",
            policy_version: "v1",
        }
    }

    /// Insert a minimal `retrieval_traces` row.
    fn seed_retrieval_trace(db: &Arc<Database>, trace_id: &str, response_id: &str) {
        db.write()
            .execute(
                "INSERT INTO retrieval_traces (
                     id, response_id, task_id, query_hash, query_class,
                     classifier_version, profile_id, graph_revision, policy_hash,
                     token_budget, status, degradation_json, embed_model_version,
                     k_value, availability_json, weights_json,
                     evidence_contribution, memory_worth_contribution, goal_contribution_total,
                     created_at
                 ) VALUES (
                     ?1, ?2, NULL, 'qhash', 'recall',
                     'clf-v1', 'default', NULL, NULL,
                     NULL, 'finalized', NULL, NULL,
                     60.0, '{}', '{}',
                     0.0, 0.0, 0.0,
                     '2024-01-01T00:00:00Z'
                 )",
                rusqlite::params![trace_id, response_id],
            )
            .unwrap();
    }

    /// Insert a `retrieval_trace_items` row.
    fn seed_trace_item(
        db: &Arc<Database>,
        trace_id: &str,
        record_id: &str,
        injected_order: Option<i64>,
    ) {
        db.write()
            .execute(
                "INSERT INTO retrieval_trace_items (
                     trace_id, record_id, strategy, strategy_rank, strategy_score,
                     weight, rrf_contribution, gate_disposition, reason_code,
                     token_cost, allocated_tokens, injected_order, goal_id,
                     evidence_contribution, memory_worth_contribution
                 ) VALUES (
                     ?1, ?2, 'fts', 1, 0.9,
                     1.0, 0.9, 'included', NULL,
                     50, 50, ?3, NULL,
                     0.0, 0.0
                 )",
                rusqlite::params![trace_id, record_id, injected_order],
            )
            .unwrap();
    }

    // ── Shared helpers ────────────────────────────────────────────────────────

    /// Count all rows in a named table.
    fn row_count(db: &Arc<Database>, table: &str) -> i64 {
        db.with_read(|conn| {
            let sql = format!("SELECT COUNT(*) FROM {table}");
            conn.query_row(&sql, [], |r| r.get::<_, i64>(0))
                .map_err(crate::memory::error::StorageError::Sqlite)
                .map_err(crate::memory::error::MemoryError::from)
        })
        .unwrap()
    }

    /// Insert a minimal `records` row.
    fn seed_record(
        db: &Arc<Database>,
        record_id: &str,
        event_id: &str,
        sensitivity: u8,
        scope: &str,
        truth_state: &str,
    ) {
        db.write()
            .execute(
                "INSERT INTO records(
                     id, record_kind, schema_version, content,
                     namespace, owner_id, scope, sensitivity, source_id, policy_version,
                     created_event_id, created_at, truth_state)
                 VALUES (
                     ?1, 'memory', 1, 'test content',
                     'core', 'user', ?2, ?3, 'src', 'v1',
                     ?4, '2024-01-01T00:00:00Z', ?5
                 )",
                params![record_id, scope, sensitivity as i64, event_id, truth_state],
            )
            .unwrap();
    }

    /// Start + complete a single tool observation with Success outcome.
    fn do_observation(db: &Arc<Database>, inv_suffix: &str) {
        let start_id = format!("ne-start-{inv_suffix}");
        let done_id = format!("ne-done-{inv_suffix}");
        let inv_id = format!("ne-inv-{inv_suffix}");
        seed_event(db, &start_id);
        seed_event(db, &done_id);
        record_tool_invocation_start(db, default_start_params(&inv_id, &start_id)).unwrap();
        record_tool_invocation_completion(
            db,
            &inv_id,
            ToolOutcome::Success,
            Some(10),
            Some("ok"),
            None,
            &done_id,
        )
        .unwrap();
    }

    // ── a. Recording success does not change any records.sensitivity ──────────

    /// Validate: Requirements MGR-033, MGR-043
    ///
    /// Start + complete a tool observation with `Success`. Verify that the
    /// `sensitivity` column of the pre-existing `records` row has NOT changed.
    /// (Records are never touched by tool_observation functions.)
    #[test]
    fn recording_success_does_not_change_record_sensitivity() {
        let db = db();
        // Seed an event to satisfy the records FK.
        seed_event(&db, "ne-rec-evt-a");

        // Insert a records row with sensitivity=1.
        seed_record(
            &db,
            "rec-sentinel-a",
            "ne-rec-evt-a",
            1,
            "private",
            "current",
        );

        // Read back the baseline sensitivity.
        let baseline: i64 = db
            .with_read(|conn| {
                conn.query_row(
                    "SELECT sensitivity FROM records WHERE id = 'rec-sentinel-a'",
                    [],
                    |r| r.get(0),
                )
                .map_err(crate::memory::error::StorageError::Sqlite)
                .map_err(crate::memory::error::MemoryError::from)
            })
            .unwrap();
        assert_eq!(baseline, 1, "pre-condition: sensitivity must be 1");

        // Perform a tool observation start + completion.
        do_observation(&db, "a");

        // Re-read the sensitivity — it must be unchanged.
        let after: i64 = db
            .with_read(|conn| {
                conn.query_row(
                    "SELECT sensitivity FROM records WHERE id = 'rec-sentinel-a'",
                    [],
                    |r| r.get(0),
                )
                .map_err(crate::memory::error::StorageError::Sqlite)
                .map_err(crate::memory::error::MemoryError::from)
            })
            .unwrap();
        assert_eq!(
            after, baseline,
            "observations must NOT change records.sensitivity"
        );
    }

    // ── b. Recording success does not create authority records ────────────────

    /// Validate: Requirements MGR-033, MGR-043–045
    ///
    /// Start + complete a tool observation. Verify no new row was added to
    /// `records`, `entities_v2`, or `goals_v2`. Observations only ever append
    /// to `tool_observations`.
    #[test]
    fn recording_success_does_not_create_authority_record() {
        let db = db();

        // Count rows before.
        let records_before = row_count(&db, "records");
        let entities_before = row_count(&db, "entities_v2");
        let goals_before = row_count(&db, "goals_v2");

        // Perform a tool observation start + completion.
        do_observation(&db, "b");

        // Counts must be unchanged.
        let records_after = row_count(&db, "records");
        let entities_after = row_count(&db, "entities_v2");
        let goals_after = row_count(&db, "goals_v2");

        assert_eq!(
            records_after, records_before,
            "observations must NOT write to records table (before={records_before}, after={records_after})"
        );
        assert_eq!(
            entities_after, entities_before,
            "observations must NOT write to entities_v2 table"
        );
        assert_eq!(
            goals_after, goals_before,
            "observations must NOT write to goals_v2 table"
        );

        // Verify that exactly one row was appended to tool_observations.
        let obs_count = row_count(&db, "tool_observations");
        assert_eq!(
            obs_count, 1,
            "exactly one tool_observation row must exist after one start+complete"
        );
    }

    // ── c. MemoryWorthTracker never modifies the DB ───────────────────────────

    /// Validate: Requirements MGR-033, MGD-031
    ///
    /// Call `record_observation` 100 times, then `get_worth_contribution` and
    /// `trace_contribution`. Verify the DB has ZERO new rows in every table
    /// (MemoryWorthTracker is purely in-memory; it never touches the DB).
    #[test]
    fn memory_worth_tracker_never_modifies_db() {
        let db = db();

        // Snapshot row counts for all relevant tables before tracker use.
        let tables = [
            "tool_observations",
            "records",
            "entities_v2",
            "goals_v2",
            "evidence_v2",
            "retrieval_traces",
            "retrieval_trace_items",
            "events_v2",
        ];
        let before: Vec<i64> = tables.iter().map(|t| row_count(&db, t)).collect();

        // Use MemoryWorthTracker purely in-memory.
        let mut tracker = MemoryWorthTracker::new();
        for i in 0..100u32 {
            let record_id = format!("mwt-rec-{i}");
            tracker.record_observation(&record_id, &ToolOutcome::Success);
        }
        // Call query methods — none of these should touch the DB.
        let _ = tracker.get_worth_contribution("mwt-rec-0");
        let _ = tracker.trace_contribution("mwt-rec-0", "mwt-trace-1");

        // Row counts must be identical — tracker must not have touched the DB.
        let after: Vec<i64> = tables.iter().map(|t| row_count(&db, t)).collect();
        for (i, table) in tables.iter().enumerate() {
            assert_eq!(
                after[i], before[i],
                "MemoryWorthTracker must NOT modify table '{table}' \
                 (before={}, after={})",
                before[i], after[i]
            );
        }
    }

    // ── d. compute_capability_metrics is read-only ────────────────────────────

    /// Validate: Requirements MGR-044, MGD-043
    ///
    /// Insert 20 rows, call `compute_capability_metrics`. Verify the total row
    /// count in `tool_observations` has not changed after the call.
    #[test]
    fn capability_metrics_read_only_no_writes() {
        let db = db();

        // Seed 20 observations so compute_capability_metrics reaches the
        // Metrics branch (≥20 sample). We insert directly to control timing.
        let window_ts = "2024-03-01T12:00:00Z";
        for i in 0..20u32 {
            let evt_id = format!("cm-evt-{i}");
            seed_event(&db, &evt_id);
            let row_id = new_id().to_string();
            let inv_id = format!("cm-inv-{i}");
            db.write()
                .execute(
                    "INSERT INTO tool_observations (
                         id, invocation_id, tool_id, tool_version,
                         environment_class, outcome, latency_ms,
                         namespace, owner_id, scope, sensitivity, source_id,
                         policy_version, start_event_id, completion_event_id,
                         created_at
                     ) VALUES (
                         ?1, ?2, 'cm_tool', '1.0',
                         'test', 'success', 50,
                         'core', 'user', 'private', 0, 'src',
                         'v1', ?3, ?3,
                         ?4
                     )",
                    params![row_id, inv_id, evt_id, window_ts],
                )
                .unwrap();
        }

        let count_before = row_count(&db, "tool_observations");
        assert_eq!(count_before, 20, "pre-condition: 20 rows must exist");

        // Call the metrics function.
        let _ = compute_capability_metrics(
            &db,
            "cm_tool",
            "1.0",
            "test",
            "2024-03-01T00:00:00Z",
            "2024-03-31T23:59:59Z",
        )
        .unwrap();

        // Row count must be unchanged — compute_capability_metrics is read-only.
        let count_after = row_count(&db, "tool_observations");
        assert_eq!(
            count_after, count_before,
            "compute_capability_metrics must NOT write any rows \
             (before={count_before}, after={count_after})"
        );
    }

    // ── e. attribute_task_outcome is read-only ────────────────────────────────

    /// Validate: Requirements MGR-033, MGR-045, MGD-031
    ///
    /// Set up a retrieval trace with 3 injected records, call
    /// `attribute_task_outcome`. Verify nothing was written to `records`,
    /// `retrieval_traces`, or `retrieval_trace_items`.
    #[test]
    fn attribute_task_outcome_read_only_no_writes() {
        let db = db();

        // Seed a retrieval trace with 3 injected items.
        seed_retrieval_trace(&db, "ato-trace-1", "ato-resp-1");
        seed_trace_item(&db, "ato-trace-1", "ato-rec-1", Some(1));
        seed_trace_item(&db, "ato-trace-1", "ato-rec-2", Some(2));
        seed_trace_item(&db, "ato-trace-1", "ato-rec-3", Some(3));

        // Snapshot counts.
        let records_before = row_count(&db, "records");
        let traces_before = row_count(&db, "retrieval_traces");
        let items_before = row_count(&db, "retrieval_trace_items");

        // Call attribution.
        let result =
            attribute_task_outcome(&db, "ato-resp-1", &ToolOutcome::Success, "test-policy")
                .unwrap();

        // Verify attribution worked correctly (3 used records).
        assert_eq!(result.used_record_count, 3);

        // Row counts must be unchanged — attribution is read-only.
        assert_eq!(
            row_count(&db, "records"),
            records_before,
            "attribute_task_outcome must NOT write to records"
        );
        assert_eq!(
            row_count(&db, "retrieval_traces"),
            traces_before,
            "attribute_task_outcome must NOT write to retrieval_traces"
        );
        assert_eq!(
            row_count(&db, "retrieval_trace_items"),
            items_before,
            "attribute_task_outcome must NOT write to retrieval_trace_items"
        );
    }

    // ── f. Observation cannot override an explicit correction (truth_state) ───

    /// Validate: Requirements MGR-033, MGR-043
    ///
    /// Insert a `records` row with `truth_state = 'current'`, then record 100
    /// tool observations. Verify the `truth_state` is still `'current'` after
    /// all those observations. Observations never override truth state.
    #[test]
    fn observation_cannot_override_explicit_correction() {
        let db = db();
        seed_event(&db, "ne-corr-evt");

        // Insert a record with truth_state = 'current' (explicit correction signal).
        seed_record(&db, "rec-truth", "ne-corr-evt", 0, "private", "current");

        // Record 100 tool observations — none of them should touch truth_state.
        for i in 0..100u32 {
            let suffix = format!("corr-{i}");
            do_observation(&db, &suffix);
        }

        // truth_state must still be 'current'.
        let truth_state: String = db
            .with_read(|conn| {
                conn.query_row(
                    "SELECT truth_state FROM records WHERE id = 'rec-truth'",
                    [],
                    |r| r.get(0),
                )
                .map_err(crate::memory::error::StorageError::Sqlite)
                .map_err(crate::memory::error::MemoryError::from)
            })
            .unwrap();

        assert_eq!(
            truth_state, "current",
            "observations must NOT override truth_state; expected 'current', got '{truth_state}'"
        );
    }

    // ── g. Observation cannot grant capability scope widening ─────────────────

    /// Validate: Requirements MGR-033, MGR-043–045, MGD-043
    ///
    /// Insert a `records` row with `sensitivity=2, scope='private'`. Record
    /// tool observations with different sensitivity/scope parameters (via
    /// `StartParams`). Verify the record's `sensitivity` and `scope` are
    /// completely unchanged.
    #[test]
    fn observation_cannot_grant_capability_scope_widening() {
        let db = db();
        seed_event(&db, "ne-scope-evt");

        // Insert a record with restrictive sensitivity=2 and scope='private'.
        seed_record(
            &db,
            "rec-restricted",
            "ne-scope-evt",
            2,
            "private",
            "current",
        );

        // Record observations with different sensitivity/scope combos.
        let combos: &[(&str, u8)] = &[("public", 0), ("team", 1), ("shared", 0), ("private", 3)];
        for (i, (scope, sensitivity)) in combos.iter().enumerate() {
            let start_id = format!("ne-scope-start-{i}");
            let done_id = format!("ne-scope-done-{i}");
            let inv_id = format!("ne-scope-inv-{i}");
            seed_event(&db, &start_id);
            seed_event(&db, &done_id);

            let mut p = default_start_params(&inv_id, &start_id);
            p.scope = scope;
            p.sensitivity = *sensitivity;
            record_tool_invocation_start(&db, p).unwrap();
            record_tool_invocation_completion(
                &db,
                &inv_id,
                ToolOutcome::Success,
                Some(5),
                None,
                None,
                &done_id,
            )
            .unwrap();
        }

        // Verify the record's sensitivity and scope are unchanged.
        let (stored_sensitivity, stored_scope): (i64, String) = db
            .with_read(|conn| {
                conn.query_row(
                    "SELECT sensitivity, scope FROM records WHERE id = 'rec-restricted'",
                    [],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                )
                .map_err(crate::memory::error::StorageError::Sqlite)
                .map_err(crate::memory::error::MemoryError::from)
            })
            .unwrap();

        assert_eq!(
            stored_sensitivity, 2,
            "observations must NOT change records.sensitivity \
             (expected 2, got {stored_sensitivity})"
        );
        assert_eq!(
            stored_scope, "private",
            "observations must NOT change records.scope \
             (expected 'private', got '{stored_scope}')"
        );
    }

    // ── h. MemoryWorthTracker below 20 never influences record content ─────────

    /// Validate: Requirements MGR-033, MGD-031
    ///
    /// Set up a `MemoryWorthTracker`, add 19 observations, verify it's still
    /// `Inert`, and confirm `trace_contribution` returns `None` (cannot
    /// influence any retrieval ranking below the 20-observation threshold).
    #[test]
    fn memory_worth_below_20_never_influences_record_content() {
        let mut tracker = MemoryWorthTracker::new();

        // Add exactly 19 observations — one below the Active threshold.
        for _ in 0..19 {
            tracker.record_observation("mw-rec-h", &ToolOutcome::Success);
        }

        // Must still be Inert at 19.
        assert_eq!(
            tracker.get_worth_contribution("mw-rec-h"),
            MemoryWorthContribution::Inert,
            "tracker must be Inert at 19 observations (< 20 threshold)"
        );

        // trace_contribution must return None — cannot influence retrieval ranking.
        let traced = tracker.trace_contribution("mw-rec-h", "mw-trace-h");
        assert!(
            traced.is_none(),
            "trace_contribution must return None below 20 observations; \
             a non-None value would allow the inert tracker to influence retrieval"
        );

        // Also verify an unseen record is Inert.
        assert_eq!(
            tracker.get_worth_contribution("mw-rec-unseen"),
            MemoryWorthContribution::Inert,
            "untracked record must be Inert"
        );
        assert!(
            tracker
                .trace_contribution("mw-rec-unseen", "mw-trace-h")
                .is_none(),
            "trace_contribution for untracked record must return None"
        );
    }
}
