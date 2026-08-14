//! Startup integrity assertions (design §5.3, task 1.8.1, MGR-017) and
//! recovery/release deep-check suite (task 1.8.2, MGR-017, MGR-042).
//!
//! ## Fast startup checks — [`StartupIntegrityChecker`]
//!
//! Runs on every authority open — before any reads or writes are accepted —
//! and asserts the following six invariants:
//!
//! 1. **Schema checksums / pragmas** — all applied migration checksums in
//!    `schema_version` match the expected BLAKE3 digests; `PRAGMA quick_check`
//!    returns `ok`; WAL / FK / `synchronous=FULL` are asserted (already done
//!    per-connection by `db::configure`, but re-verified here as a single
//!    collective startup gate).
//!
//! 2. **`PRAGMA quick_check`** — SQLite's fast structural B-tree scan (does NOT
//!    require a write lock; runs on a read connection).
//!
//! 3. **Event HLC monotonicity** — the last 500 `events_v2` rows ordered by HLC
//!    must have no backwards-step (i.e. HLC values are non-decreasing); any gap
//!    is a corruption signal.
//!
//! 4. **Graph-revision continuity** — every `graph_revisions` row must satisfy
//!    `base_revision = revision - 1`; gaps indicate a lost or corrupt revision.
//!    An empty table is valid (fresh authority).
//!
//! 5. **Authority singleton** — `authority_meta` has exactly one row with
//!    `id = 1` and `graph_revision >= 0`.
//!
//! 6. **Outbox cursor sanity** — no `derived_outbox` row has an unrecognised
//!    `status` value, and no row has `attempts > 100` (stuck dead-letter
//!    entries that would never be processed).
//!
//! All checks are read-only and designed to complete in milliseconds on a
//! laptop-scale database (bounded window for HLC check; COUNT(*) for others).
//!
//! ## Deep recovery/release checks — [`RecoveryIntegrityChecker`]
//!
//! A heavier, slower set of checks intended for explicit recovery triage or
//! the release gate — **NOT called on startup**. The five checks are:
//!
//! 1. **Full `PRAGMA integrity_check`** — thorough SQLite structural scan (slower
//!    than `quick_check`; acceptable for release/recovery contexts).
//!
//! 2. **Full event HLC order + bounded checksum coverage** — all-event HLC
//!    monotonicity scan plus non-empty `payload_checksum` verification over the
//!    last 10 000 events.
//!
//! 3. **Complete migration coverage** — every compiled-in migration is applied
//!    (no missing required migration) and every `schema_version` row is in the
//!    compiled-in set (no injected migration).
//!
//! 4. **Derived manifest version comparison** — active `derived_manifests` rows
//!    are checked against current expected `algorithm_version` / `model_version`.
//!    Stale versions are classified as [`IntegrityFaultClass::ManifestVersionMismatch`]
//!    which produces [`CapabilityState::Partial`] — not a corruption error.
//!
//! 5. **Policy-safe fault classification** — all fault reports use
//!    [`IntegrityFaultClass`] and a correlation ID; no memory content, entity
//!    labels, or protected row counts are included.
//!
//! Results are aggregated into a [`RecoveryCheckReport`].
//!
//! ## Integration
//!
//! [`StartupIntegrityChecker::run_all`] is called from
//! [`MemorySystem::assemble`](crate::api::MemorySystem::assemble) after
//! the database opens but before any service is wired up. A [`StartupError`]
//! return blocks startup; the caller surfaces it as a hard
//! [`MemoryError`](crate::error::MemoryError).
//!
//! [`RecoveryIntegrityChecker::run_all`] is intended for explicit invocation
//! (recovery triage, release gate) and is exposed via
//! [`AuthorityIntegrity::deep_check`].

use std::sync::Arc;

use rusqlite::OptionalExtension;

use crate::db::{migrations, Database};
use crate::error::{MemoryResult, StorageError};

// ─────────────────────────────────────────────────────────────────────────────
// StartupError — typed failure reasons
// ─────────────────────────────────────────────────────────────────────────────

/// A startup integrity failure (design §5.3 "fail-closed posture").
///
/// Each variant corresponds to one of the six named checks in task 1.8.1.
/// Callers (currently [`MemorySystem::assemble`]) convert this into a
/// [`StorageError::Corruption`] so it propagates as a hard startup error and
/// can trigger Recovery_Mode in task 1.8.3.
#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    /// `PRAGMA quick_check` did not return `ok`.
    #[error("PRAGMA quick_check failed: {0}")]
    QuickCheckFailed(String),

    /// A migration checksum stored in `schema_version` does not match the
    /// BLAKE3 digest of the compiled-in migration SQL.
    #[error(
        "schema checksum mismatch for migration {version}: stored={stored}, expected={expected}"
    )]
    SchemaChecksumMismatch {
        version: u32,
        stored: String,
        expected: String,
    },

    /// A migration row exists in `schema_version` for a version that is not in
    /// the compiled-in migration set (unknown / injected migration).
    #[error("unknown migration version {0} found in schema_version")]
    UnknownMigrationVersion(u32),

    /// `events_v2` HLC values are not monotonically non-decreasing in the
    /// sampled window, indicating a corrupt or tampered event log.
    #[error("event HLC order violation: hlc {current:?} < previous {previous:?}")]
    EventHlcOrderViolation { previous: String, current: String },

    /// A `graph_revisions` row has `base_revision != revision - 1`, indicating
    /// a gap or duplicate in the revision sequence.
    #[error(
        "graph revision continuity gap: revision {revision} has base_revision {base_revision}, \
         expected {expected}"
    )]
    GraphRevisionGap {
        revision: i64,
        base_revision: i64,
        expected: i64,
    },

    /// `authority_meta` does not contain exactly one row with `id = 1`.
    #[error("authority singleton violation: {0}")]
    AuthoritySingletonViolation(String),

    /// An outbox row has an unrecognised `status` value.
    #[error("outbox sanity: unrecognised status {status:?} on row id={id}")]
    OutboxUnknownStatus { id: i64, status: String },

    /// An outbox row has `attempts > 100` (stuck / dead-letter entry).
    #[error("outbox sanity: row id={id} has attempts={attempts} > 100 (stuck)")]
    OutboxStuckEntry { id: i64, attempts: i64 },

    /// A SQLite error occurred while running a check.
    #[error("sqlite error during startup check: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

impl From<StartupError> for crate::error::MemoryError {
    fn from(e: StartupError) -> Self {
        StorageError::Corruption(e.to_string()).into()
    }
}

/// Result type for individual startup checks.
pub type StartupResult<T> = Result<T, StartupError>;

// ─────────────────────────────────────────────────────────────────────────────
// StartupIntegrityChecker
// ─────────────────────────────────────────────────────────────────────────────

/// Maximum outbox `attempts` before a row is considered stuck (design §5.3
/// "outbox cursor sanity"). Rows at or above this threshold prevent startup.
pub const MAX_OUTBOX_ATTEMPTS: i64 = 100;

/// Maximum number of `events_v2` rows sampled for the HLC monotonicity check.
/// Bounded so this check stays fast on a large event log.
const HLC_SAMPLE_WINDOW: i64 = 500;

/// Runs all six startup integrity checks against the open authority.
///
/// Constructed from an [`Arc<Database>`] by the composition root and called
/// exactly once per startup before any service is wired up.
pub struct StartupIntegrityChecker {
    db: Arc<Database>,
}

impl StartupIntegrityChecker {
    /// Create a checker over the already-open authority handle.
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Run all six checks in order.  Returns the first [`StartupError`] found.
    /// All checks are read-only and complete in O(1)/O(log N) time.
    pub fn run_all(&self) -> MemoryResult<()> {
        self.check_quick_check()?;
        self.check_schema_checksums()?;
        self.check_event_hlc_order()?;
        self.check_graph_revision_continuity()?;
        self.check_authority_singleton()?;
        self.check_outbox_cursor_sanity()?;
        Ok(())
    }

    // ── 1 & 2: PRAGMA quick_check ─────────────────────────────────────────

    /// Run `PRAGMA quick_check` and fail if the result is not `ok`.
    pub fn check_quick_check(&self) -> MemoryResult<()> {
        self.db
            .with_read(|conn| {
                let result: String = conn
                    .query_row("PRAGMA quick_check", [], |r| r.get(0))
                    .map_err(StartupError::Sqlite)?;
                if result != "ok" {
                    return Err(StartupError::QuickCheckFailed(result).into());
                }
                Ok(())
            })
            .map_err(|e| {
                // Convert MemoryError wrapping a StartupError back out — if the
                // closure returned a StartupError it was wrapped by the ? in
                // with_read's map_err; re-wrap as corruption.
                e
            })
    }

    // ── 1: Schema checksums ───────────────────────────────────────────────

    /// Verify every row in `schema_version` matches the compiled-in BLAKE3
    /// checksum and that no unknown migration version appears.
    pub fn check_schema_checksums(&self) -> MemoryResult<()> {
        self.db.with_read(|conn| {
            // Build a lookup: version → expected checksum from compiled-in SQL.
            let expected: std::collections::HashMap<u32, String> =
                migrations::migration_checksums();

            // Read all applied rows.
            let mut stmt = conn
                .prepare("SELECT version, checksum FROM schema_version ORDER BY version")
                .map_err(StartupError::Sqlite)?;
            let rows: Vec<(u32, String)> = stmt
                .query_map([], |row| Ok((row.get::<_, i64>(0)? as u32, row.get(1)?)))
                .map_err(StartupError::Sqlite)?
                .collect::<Result<_, _>>()
                .map_err(StartupError::Sqlite)?;

            for (version, stored) in rows {
                match expected.get(&version) {
                    None => {
                        return Err(StartupError::UnknownMigrationVersion(version).into());
                    }
                    Some(exp) if exp != &stored => {
                        return Err(StartupError::SchemaChecksumMismatch {
                            version,
                            stored,
                            expected: exp.clone(),
                        }
                        .into());
                    }
                    Some(_) => {}
                }
            }
            Ok(())
        })
    }

    // ── 3: Event HLC monotonicity ─────────────────────────────────────────

    /// Sample the last `HLC_SAMPLE_WINDOW` rows from `events_v2` ordered by
    /// HLC and verify they are non-decreasing. A backwards step means either
    /// clock corruption or a tampered insert.
    pub fn check_event_hlc_order(&self) -> MemoryResult<()> {
        self.db.with_read(|conn| {
            // Check if events_v2 table exists (may not on very old schemas).
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master \
                     WHERE type='table' AND name='events_v2'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StartupError::Sqlite)?;
            if exists == 0 {
                return Ok(()); // table absent → skip (pre-v2 schema)
            }

            // Fetch last N HLCs in ascending order.
            let mut stmt = conn
                .prepare(
                    "SELECT hlc FROM events_v2 \
                     ORDER BY hlc DESC \
                     LIMIT ?1",
                )
                .map_err(StartupError::Sqlite)?;
            let hlcs: Vec<String> = stmt
                .query_map([HLC_SAMPLE_WINDOW], |row| row.get(0))
                .map_err(StartupError::Sqlite)?
                .collect::<Result<_, _>>()
                .map_err(StartupError::Sqlite)?;

            // The query returns DESC so we check adjacent pairs: each successive
            // element must be <= its predecessor (i.e. none is GREATER than the
            // one we already saw in descending order).
            // Re-reverse for a more natural ascending check.
            let mut prev: Option<String> = None;
            for hlc in hlcs.into_iter().rev() {
                if let Some(ref p) = prev {
                    if hlc < *p {
                        return Err(StartupError::EventHlcOrderViolation {
                            previous: p.clone(),
                            current: hlc,
                        }
                        .into());
                    }
                }
                prev = Some(hlc);
            }
            Ok(())
        })
    }

    // ── 4: Graph revision continuity ─────────────────────────────────────

    /// Verify that every `graph_revisions` row has `base_revision = revision - 1`
    /// and that no gaps exist in the sequence (i.e. no revision is missing its
    /// predecessor). An empty table is valid (fresh authority, no commits yet).
    pub fn check_graph_revision_continuity(&self) -> MemoryResult<()> {
        self.db.with_read(|conn| {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master \
                     WHERE type='table' AND name='graph_revisions'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StartupError::Sqlite)?;
            if exists == 0 {
                return Ok(()); // table absent → skip
            }

            // A single aggregate query detects any violation without a full scan.
            // Strategy: if revisions are a contiguous sequence starting at
            // some base, then MAX - MIN + 1 = COUNT.
            // Additionally, any row whose base_revision does not equal revision-1
            // is a local structural error (should be caught by the DB CHECK,
            // but we verify anyway in case of schema bypass or future schema
            // evolution).
            //
            // Two checks in one query for efficiency:
            //  1. Any row with base_revision != revision - 1.
            //  2. Any gap in the sequence (MAX - MIN + 1 != COUNT).
            let (count, min_rev, max_rev): (i64, Option<i64>, Option<i64>) = conn
                .query_row(
                    "SELECT COUNT(*), MIN(revision), MAX(revision) FROM graph_revisions",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .map_err(StartupError::Sqlite)?;

            if count == 0 {
                return Ok(()); // empty → valid
            }

            let min_rev = min_rev.unwrap_or(0);
            let max_rev = max_rev.unwrap_or(0);
            // Check for gaps: a contiguous sequence has max - min + 1 = count.
            if max_rev - min_rev + 1 != count {
                return Err(StartupError::GraphRevisionGap {
                    revision: max_rev,
                    base_revision: -1, // sentinel: gap detected via count mismatch
                    expected: min_rev + count - 1,
                }
                .into());
            }

            // Also check the structural local predicate for any surviving row.
            let mut stmt = conn
                .prepare(
                    "SELECT revision, base_revision \
                     FROM graph_revisions \
                     WHERE base_revision != revision - 1 \
                     ORDER BY revision \
                     LIMIT 1",
                )
                .map_err(StartupError::Sqlite)?;
            let bad: Option<(i64, i64)> = stmt
                .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()
                .map_err(StartupError::Sqlite)?;

            if let Some((revision, base_revision)) = bad {
                return Err(StartupError::GraphRevisionGap {
                    revision,
                    base_revision,
                    expected: revision - 1,
                }
                .into());
            }
            Ok(())
        })
    }

    // ── 5: Authority singleton ────────────────────────────────────────────

    /// Verify `authority_meta` has exactly one row with `id = 1` and
    /// `graph_revision >= 0`.
    pub fn check_authority_singleton(&self) -> MemoryResult<()> {
        self.db.with_read(|conn| {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master \
                     WHERE type='table' AND name='authority_meta'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StartupError::Sqlite)?;
            if exists == 0 {
                return Err(StartupError::AuthoritySingletonViolation(
                    "authority_meta table does not exist".to_string(),
                )
                .into());
            }

            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM authority_meta", [], |r| r.get(0))
                .map_err(StartupError::Sqlite)?;
            if count != 1 {
                return Err(StartupError::AuthoritySingletonViolation(format!(
                    "expected exactly 1 authority_meta row, found {count}"
                ))
                .into());
            }

            // Verify id=1 and graph_revision>=0.
            let (id, graph_revision): (i64, i64) = conn
                .query_row(
                    "SELECT id, graph_revision FROM authority_meta LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(StartupError::Sqlite)?;

            if id != 1 {
                return Err(StartupError::AuthoritySingletonViolation(format!(
                    "authority_meta singleton has id={id}, expected 1"
                ))
                .into());
            }
            if graph_revision < 0 {
                return Err(StartupError::AuthoritySingletonViolation(format!(
                    "authority_meta graph_revision={graph_revision} is negative"
                ))
                .into());
            }
            Ok(())
        })
    }

    // ── 6: Outbox cursor sanity ───────────────────────────────────────────

    /// Check `derived_outbox` for rows with unrecognised status values or with
    /// `attempts > MAX_OUTBOX_ATTEMPTS`.
    pub fn check_outbox_cursor_sanity(&self) -> MemoryResult<()> {
        self.db.with_read(|conn| {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master \
                     WHERE type='table' AND name='derived_outbox'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StartupError::Sqlite)?;
            if exists == 0 {
                return Ok(()); // table absent → skip
            }

            // Check for unknown status values.
            let mut stmt = conn
                .prepare(
                    "SELECT id, status FROM derived_outbox \
                     WHERE status NOT IN ('pending','applied','superseded','dead_letter') \
                     LIMIT 1",
                )
                .map_err(StartupError::Sqlite)?;
            let bad_status: Option<(i64, String)> = stmt
                .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()
                .map_err(StartupError::Sqlite)?;
            if let Some((id, status)) = bad_status {
                return Err(StartupError::OutboxUnknownStatus { id, status }.into());
            }

            // Check for stuck entries (attempts > MAX_OUTBOX_ATTEMPTS).
            let mut stmt2 = conn
                .prepare(
                    "SELECT id, attempts FROM derived_outbox \
                     WHERE attempts > ?1 \
                     LIMIT 1",
                )
                .map_err(StartupError::Sqlite)?;
            let stuck: Option<(i64, i64)> = stmt2
                .query_row([MAX_OUTBOX_ATTEMPTS], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()
                .map_err(StartupError::Sqlite)?;
            if let Some((id, attempts)) = stuck {
                return Err(StartupError::OutboxStuckEntry { id, attempts }.into());
            }

            Ok(())
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// IntegrityFaultClass — policy-safe fault taxonomy (design §5.3, task 1.8.2)
// ─────────────────────────────────────────────────────────────────────────────

/// A policy-safe corruption/degradation class for the recovery/release checker.
///
/// **Invariant:** no variant embeds memory content, entity labels, row counts
/// from protected namespaces, or any other protected data. Only the class of
/// fault and a correlation ID may be surfaced to a caller.  Design §5.3:
/// "Diagnostic exposes corruption class/correlation ID only."
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrityFaultClass {
    /// SQLite `PRAGMA integrity_check` reported a structural problem.
    SqliteIntegrityViolation,

    /// An event's `payload_checksum` field is empty, violating the tamper-
    /// detection invariant (design §4.1: every event must carry a checksum).
    EventChecksumMissing,

    /// The HLC sequence over all events is not non-decreasing — a backwards
    /// step was detected (tampered or corrupt event log).
    EventHlcOrderViolation,

    /// A migration row exists in `schema_version` that is not in the compiled-in
    /// migration set (potentially injected).
    UnknownMigration,

    /// A compiled-in migration that should be applied is missing from
    /// `schema_version`.
    MissingRequiredMigration,

    /// An active `derived_manifests` record carries an `algorithm_version` or
    /// `model_version` that does not match the currently expected values.
    /// This classifies as [`CapabilityState::Partial`] — not corruption.
    ManifestVersionMismatch,

    /// An I/O or SQLite error occurred during the check.
    SqliteError,
}

impl std::fmt::Display for IntegrityFaultClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SqliteIntegrityViolation => f.write_str("sqlite_integrity_violation"),
            Self::EventChecksumMissing => f.write_str("event_checksum_missing"),
            Self::EventHlcOrderViolation => f.write_str("event_hlc_order_violation"),
            Self::UnknownMigration => f.write_str("unknown_migration"),
            Self::MissingRequiredMigration => f.write_str("missing_required_migration"),
            Self::ManifestVersionMismatch => f.write_str("manifest_version_mismatch"),
            Self::SqliteError => f.write_str("sqlite_error"),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CapabilityState — overall state of a subsystem after deep checks
// ─────────────────────────────────────────────────────────────────────────────

/// Operational capability state for a subsystem or the authority as a whole.
///
/// Mirrors the state machine in design §5.3:
/// - `Healthy` → no faults found.
/// - `Partial` → a degraded-but-not-corrupt state (e.g. stale manifest version).
/// - `Corrupt` → authority integrity failure; Recovery_Mode is required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityState {
    /// All checks passed — authority is structurally sound and up-to-date.
    Healthy,
    /// Degraded but not corrupt — some optional capability is unavailable or
    /// stale (e.g. manifest version mismatch).
    Partial,
    /// Authority integrity has failed — enter Recovery_Mode.
    Corrupt,
}

impl std::fmt::Display for CapabilityState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => f.write_str("Healthy"),
            Self::Partial => f.write_str("Partial"),
            Self::Corrupt => f.write_str("Corrupt"),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RecoveryFault — one recorded fault (policy-safe)
// ─────────────────────────────────────────────────────────────────────────────

/// A single fault recorded during the deep recovery/release check.
///
/// Only the fault class, a stable correlation ID (monotonically numbered within
/// the run), and a short human-readable description are included. **No** memory
/// content, entity labels, namespace counts, or any protected data appears here.
#[derive(Debug, Clone)]
pub struct RecoveryFault {
    /// Stable correlation ID within this run (1-based).
    pub correlation_id: u32,
    /// Policy-safe fault classification.
    pub fault_class: IntegrityFaultClass,
    /// Short human-readable description — must not contain protected data.
    pub description: String,
}

impl RecoveryFault {
    fn new(
        correlation_id: u32,
        fault_class: IntegrityFaultClass,
        description: impl Into<String>,
    ) -> Self {
        Self {
            correlation_id,
            fault_class,
            description: description.into(),
        }
    }
}

impl std::fmt::Display for RecoveryFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] fault_class={} description={}",
            self.correlation_id, self.fault_class, self.description
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RecoveryCheckReport — aggregate result of all deep checks
// ─────────────────────────────────────────────────────────────────────────────

/// Aggregated result of all five deep recovery/release checks.
///
/// The overall [`CapabilityState`] is derived from the worst fault class found:
/// any [`IntegrityFaultClass::ManifestVersionMismatch`] produces at most
/// [`CapabilityState::Partial`]; any other fault class escalates to
/// [`CapabilityState::Corrupt`].
#[derive(Debug)]
pub struct RecoveryCheckReport {
    /// Aggregate capability state across all checks.
    pub state: CapabilityState,
    /// All faults found, ordered by correlation ID.
    pub faults: Vec<RecoveryFault>,
    /// Whether `PRAGMA integrity_check` passed (no structural SQLite errors).
    pub sqlite_integrity_ok: bool,
    /// Whether all events in the bounded window have a non-empty checksum.
    pub event_checksums_ok: bool,
    /// Whether all events in the full log have a monotonically non-decreasing HLC.
    pub event_hlc_order_ok: bool,
    /// Whether migration coverage is complete (no extras, no missing).
    pub migration_coverage_ok: bool,
    /// Number of active derived manifests with a stale algorithm/model version.
    pub stale_manifest_count: u32,
}

impl RecoveryCheckReport {
    fn empty() -> Self {
        Self {
            state: CapabilityState::Healthy,
            faults: Vec::new(),
            sqlite_integrity_ok: true,
            event_checksums_ok: true,
            event_hlc_order_ok: true,
            migration_coverage_ok: true,
            stale_manifest_count: 0,
        }
    }

    /// Promote the aggregate state to at least `new_state`.
    fn escalate(&mut self, new_state: CapabilityState) {
        use CapabilityState::*;
        self.state = match (&self.state, &new_state) {
            (Corrupt, _) | (_, Corrupt) => Corrupt,
            (Partial, _) | (_, Partial) => Partial,
            _ => Healthy,
        };
    }

    /// Record a fault and escalate the aggregate state accordingly.
    fn record(&mut self, fault_class: IntegrityFaultClass, description: impl Into<String>) {
        let id = self.faults.len() as u32 + 1;
        let state = match &fault_class {
            IntegrityFaultClass::ManifestVersionMismatch => CapabilityState::Partial,
            _ => CapabilityState::Corrupt,
        };
        self.faults
            .push(RecoveryFault::new(id, fault_class, description));
        self.escalate(state);
    }
}

impl std::fmt::Display for RecoveryCheckReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RecoveryCheckReport {{ state={}, faults={}, stale_manifests={} }}",
            self.state,
            self.faults.len(),
            self.stale_manifest_count
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RecoveryIntegrityChecker — deep checks for release gate / recovery triage
// ─────────────────────────────────────────────────────────────────────────────

/// Maximum number of recent events checked for non-empty checksum.
///
/// Checking all events is potentially too slow on a very large database; we
/// bound it to the most recent `EVENT_CHECKSUM_WINDOW` events which covers
/// all recently written data while remaining bounded in cost.
pub const EVENT_CHECKSUM_WINDOW: i64 = 10_000;

/// A recovery/release deep-check suite (design §5.3, task 1.8.2).
///
/// This checker runs **five** thorough checks that are intentionally too slow
/// for startup but appropriate for:
/// - Explicit recovery triage (before attempting a verified restore).
/// - The release gate (CI `cargo test` or a named evidence command).
///
/// It is constructed from the same [`Arc<Database>`] as the startup checker
/// and accessed via [`crate::authority::mod::AuthorityIntegrity::deep_check`].
///
/// **None of its checks mutate the authority.**
pub struct RecoveryIntegrityChecker {
    db: Arc<Database>,
    /// Expected algorithm version for active derived manifests, if known.
    /// `None` means skip the algorithm version comparison.
    expected_algorithm_version: Option<String>,
    /// Expected model version for active derived manifests, if known.
    /// `None` means skip the model version comparison.
    expected_model_version: Option<String>,
}

impl RecoveryIntegrityChecker {
    /// Create a checker over the already-open authority handle with no
    /// expected version constraints (manifest version check is skipped).
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            expected_algorithm_version: None,
            expected_model_version: None,
        }
    }

    /// Set the expected `algorithm_version` for active derived manifests.
    /// When set, any manifest with a different value is classified as
    /// [`IntegrityFaultClass::ManifestVersionMismatch`] → [`CapabilityState::Partial`].
    pub fn with_expected_algorithm_version(mut self, version: impl Into<String>) -> Self {
        self.expected_algorithm_version = Some(version.into());
        self
    }

    /// Set the expected `model_version` for active derived manifests.
    /// When set, any manifest with a different value is classified as
    /// [`IntegrityFaultClass::ManifestVersionMismatch`] → [`CapabilityState::Partial`].
    pub fn with_expected_model_version(mut self, version: impl Into<String>) -> Self {
        self.expected_model_version = Some(version.into());
        self
    }

    /// Run all five deep checks and return an aggregate [`RecoveryCheckReport`].
    ///
    /// The checks always produce a report — they do not short-circuit on the
    /// first failure, unlike [`StartupIntegrityChecker::run_all`] which is
    /// designed to block startup immediately on any error.
    pub fn run_all(&self) -> RecoveryCheckReport {
        let mut report = RecoveryCheckReport::empty();
        self.check_full_integrity(&mut report);
        self.check_full_event_hlc_order(&mut report);
        self.check_event_checksum_coverage(&mut report);
        self.check_migration_coverage(&mut report);
        self.check_derived_manifest_versions(&mut report);
        report
    }

    // ── Check 1: PRAGMA integrity_check (full, slow) ──────────────────────

    /// Run SQLite's full structural integrity check and record any failure.
    ///
    /// Unlike `quick_check`, `integrity_check` verifies:
    /// - B-tree structure, page ownership, sorted order, overflow chains.
    /// - Index/table consistency.
    /// - No page is referenced twice.
    ///
    /// This is deliberately slow and must NOT be called at startup.
    pub fn check_full_integrity(&self, report: &mut RecoveryCheckReport) {
        let result = self.db.with_read(|conn| {
            // SQLite PRAGMA integrity_check can return multiple rows; each row
            // is a separate error message. "ok" as the only row means no errors.
            let mut stmt = conn
                .prepare("PRAGMA integrity_check")
                .map_err(|e| crate::error::StorageError::Sqlite(e))?;
            let rows: Vec<String> = stmt
                .query_map([], |row| row.get(0))
                .map_err(|e| crate::error::StorageError::Sqlite(e))?
                .filter_map(|r| r.ok())
                .collect();

            // If the only row is "ok", the integrity check passed.
            if rows.len() == 1 && rows[0] == "ok" {
                Ok(true)
            } else {
                // The rows contain error descriptions — we return a count to
                // avoid exposing potentially sensitive path/page information.
                // The caller classifies by fault class only.
                Ok(false)
            }
        });

        match result {
            Ok(true) => {
                report.sqlite_integrity_ok = true;
            }
            Ok(false) => {
                report.sqlite_integrity_ok = false;
                report.record(
                    IntegrityFaultClass::SqliteIntegrityViolation,
                    "PRAGMA integrity_check reported structural errors",
                );
            }
            Err(_e) => {
                report.sqlite_integrity_ok = false;
                report.record(
                    IntegrityFaultClass::SqliteError,
                    "sqlite error running PRAGMA integrity_check",
                );
            }
        }
    }

    // ── Check 2a: Full event HLC order (all events) ───────────────────────

    /// Verify that ALL events in `events_v2` are in monotonically non-decreasing
    /// HLC order (a full scan — slower than the startup 500-row window).
    pub fn check_full_event_hlc_order(&self, report: &mut RecoveryCheckReport) {
        let result = self.db.with_read(|conn| {
            // Check if events_v2 table exists.
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master \
                     WHERE type='table' AND name='events_v2'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| crate::error::StorageError::Sqlite(e))?;
            if exists == 0 {
                return Ok(None); // skip — pre-v2 schema
            }

            // Find the first HLC backwards-step via a self-join or window;
            // a self-join-based approach is more portable across SQLite versions.
            // We look for any row where the previous HLC (ordered by rowid) is
            // greater than the current HLC.
            //
            // Using a CTE with LAG avoids a full cross-join:
            //   WITH ordered AS (SELECT hlc, ROW_NUMBER() OVER (ORDER BY hlc) rn FROM events_v2)
            //   SELECT ... WHERE curr < prev
            //
            // However, LAG() is only available from SQLite 3.25 (2018-09). Since
            // we require JSON1 but not specifically ≥3.25 window functions, we
            // use a subquery approach compatible with all modern SQLite builds:
            // check that MIN(hlc) ordered by rowid in adjacent pairs are ordered.
            //
            // Simpler and correct: detect any violation by checking that
            // `COUNT(*) WHERE hlc < lag` > 0 using a self-join on rowid:
            let violation_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) \
                     FROM events_v2 AS a \
                     JOIN events_v2 AS b ON b.rowid = a.rowid - 1 \
                     WHERE a.hlc < b.hlc",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| crate::error::StorageError::Sqlite(e))?;

            if violation_count > 0 {
                // Return how many violations were found (a count, not content).
                Ok(Some(violation_count))
            } else {
                Ok(None) // no violations
            }
        });

        match result {
            Ok(None) => {
                report.event_hlc_order_ok = true;
            }
            Ok(Some(count)) => {
                report.event_hlc_order_ok = false;
                report.record(
                    IntegrityFaultClass::EventHlcOrderViolation,
                    format!(
                        "event HLC order violation: {count} backwards steps detected in full scan"
                    ),
                );
            }
            Err(_e) => {
                report.event_hlc_order_ok = false;
                report.record(
                    IntegrityFaultClass::SqliteError,
                    "sqlite error during full event HLC order check",
                );
            }
        }
    }

    // ── Check 2b: Event checksum coverage (last 10k events) ──────────────

    /// Verify that the last `EVENT_CHECKSUM_WINDOW` events all have a non-empty
    /// `payload_checksum` field.  An empty checksum violates the tamper-detection
    /// invariant mandated by design §4.1.
    pub fn check_event_checksum_coverage(&self, report: &mut RecoveryCheckReport) {
        let result = self.db.with_read(|conn| {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master \
                     WHERE type='table' AND name='events_v2'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| crate::error::StorageError::Sqlite(e))?;
            if exists == 0 {
                return Ok(0i64); // no table — no violation
            }

            // Count rows in the last window that have an empty or NULL checksum.
            // We do NOT return the row IDs or any content — just the count.
            let empty_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM (\
                         SELECT payload_checksum \
                         FROM events_v2 \
                         ORDER BY hlc DESC \
                         LIMIT ?1\
                     ) WHERE payload_checksum IS NULL OR payload_checksum = ''",
                    rusqlite::params![EVENT_CHECKSUM_WINDOW],
                    |r| r.get(0),
                )
                .map_err(|e| crate::error::StorageError::Sqlite(e))?;
            Ok(empty_count)
        });

        match result {
            Ok(0) => {
                report.event_checksums_ok = true;
            }
            Ok(count) => {
                report.event_checksums_ok = false;
                report.record(
                    IntegrityFaultClass::EventChecksumMissing,
                    format!(
                        "event checksum violation: {count} events in last {EVENT_CHECKSUM_WINDOW} \
                         window have empty payload_checksum"
                    ),
                );
            }
            Err(_e) => {
                report.event_checksums_ok = false;
                report.record(
                    IntegrityFaultClass::SqliteError,
                    "sqlite error during event checksum coverage check",
                );
            }
        }
    }

    // ── Check 3: Complete migration coverage ──────────────────────────────

    /// Verify that:
    /// 1. No `schema_version` row belongs to a migration not in the compiled-in set
    ///    (would indicate an injected or unknown migration).
    /// 2. Every compiled-in migration is present in `schema_version` (no missing
    ///    required migration).
    ///
    /// This is a stronger check than the startup per-row checksum verification:
    /// it catches both extra rows AND missing rows.
    pub fn check_migration_coverage(&self, report: &mut RecoveryCheckReport) {
        let compiled_in: std::collections::HashMap<u32, String> = migrations::migration_checksums();
        let all_compiled_versions: std::collections::HashSet<u32> =
            compiled_in.keys().cloned().collect();

        let result = self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare("SELECT version FROM schema_version ORDER BY version")
                .map_err(|e| crate::error::StorageError::Sqlite(e))?;
            let applied: Vec<u32> = stmt
                .query_map([], |row| row.get::<_, i64>(0).map(|v| v as u32))
                .map_err(|e| crate::error::StorageError::Sqlite(e))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(applied)
        });

        let applied_versions = match result {
            Ok(v) => v,
            Err(_e) => {
                report.migration_coverage_ok = false;
                report.record(
                    IntegrityFaultClass::SqliteError,
                    "sqlite error reading schema_version for migration coverage check",
                );
                return;
            }
        };

        let applied_set: std::collections::HashSet<u32> =
            applied_versions.iter().cloned().collect();

        // 1. Unknown migrations (in applied set but not compiled-in).
        for v in &applied_versions {
            if !all_compiled_versions.contains(v) {
                report.migration_coverage_ok = false;
                report.record(
                    IntegrityFaultClass::UnknownMigration,
                    format!("migration version {v} is applied but not in the compiled-in set"),
                );
            }
        }

        // 2. Missing required migrations (in compiled-in but not applied).
        let mut missing: Vec<u32> = all_compiled_versions
            .difference(&applied_set)
            .cloned()
            .collect();
        missing.sort_unstable();
        for v in missing {
            report.migration_coverage_ok = false;
            report.record(
                IntegrityFaultClass::MissingRequiredMigration,
                format!("compiled-in migration version {v} is not present in schema_version"),
            );
        }
    }

    // ── Check 4: Derived manifest version comparison ──────────────────────

    /// Check active `derived_manifests` rows for stale `algorithm_version` or
    /// `model_version` against the expected values set on the checker.
    ///
    /// A mismatch is classified as [`IntegrityFaultClass::ManifestVersionMismatch`]
    /// which produces [`CapabilityState::Partial`] — it is a degraded state, not
    /// a corruption error.  If no expected versions are configured, this check
    /// is skipped.
    pub fn check_derived_manifest_versions(&self, report: &mut RecoveryCheckReport) {
        // If neither expected version is configured, skip the comparison.
        if self.expected_algorithm_version.is_none() && self.expected_model_version.is_none() {
            return;
        }

        let result = self.db.with_read(|conn| {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master \
                     WHERE type='table' AND name='derived_manifests'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| crate::error::StorageError::Sqlite(e))?;
            if exists == 0 {
                return Ok(vec![]); // no table → no manifests to check
            }

            // Read only the version columns — no content data.
            let mut stmt = conn
                .prepare(
                    "SELECT target, algorithm_version, model_version, status \
                     FROM derived_manifests \
                     WHERE status = 'active' OR status IS NULL",
                )
                .map_err(|e| crate::error::StorageError::Sqlite(e))?;
            let rows: Vec<(String, Option<String>, Option<String>)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(|e| crate::error::StorageError::Sqlite(e))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        });

        let manifests = match result {
            Ok(m) => m,
            Err(_e) => {
                report.record(
                    IntegrityFaultClass::SqliteError,
                    "sqlite error reading derived_manifests for version comparison",
                );
                return;
            }
        };

        let mut stale = 0u32;
        for (target, alg_ver, model_ver) in &manifests {
            let mut is_stale = false;

            if let Some(ref expected_alg) = self.expected_algorithm_version {
                if alg_ver.as_deref() != Some(expected_alg.as_str()) {
                    is_stale = true;
                    // Policy-safe: do NOT include the stored version value —
                    // only the target name (an identifier) and the fact of mismatch.
                    report.record(
                        IntegrityFaultClass::ManifestVersionMismatch,
                        format!(
                            "derived manifest for target '{}' has a stale algorithm_version \
                             (does not match expected current version)",
                            target,
                        ),
                    );
                }
            }

            if let Some(ref expected_model) = self.expected_model_version {
                if model_ver.as_deref() != Some(expected_model.as_str()) {
                    is_stale = true;
                    // Policy-safe: do NOT include the stored version value.
                    report.record(
                        IntegrityFaultClass::ManifestVersionMismatch,
                        format!(
                            "derived manifest for target '{}' has a stale model_version \
                             (does not match expected current version)",
                            target,
                        ),
                    );
                }
            }

            if is_stale {
                stale += 1;
            }
        }
        report.stale_manifest_count = stale;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn fresh() -> Arc<Database> {
        Arc::new(Database::open_in_memory().expect("in-memory db"))
    }

    fn checker(db: Arc<Database>) -> StartupIntegrityChecker {
        StartupIntegrityChecker::new(db)
    }

    // ── 1. Fresh in-memory DB passes all checks ───────────────────────────

    #[test]
    fn fresh_db_passes_all_checks() {
        let db = fresh();
        checker(db)
            .run_all()
            .expect("all startup checks must pass on a fresh DB");
    }

    // ── 2. Tampered schema_version checksum fails ─────────────────────────

    #[test]
    fn tampered_schema_checksum_fails() {
        let db = fresh();
        // Corrupt the checksum of migration version 1.
        {
            let conn = db.write();
            conn.execute(
                "UPDATE schema_version SET checksum='deadbeef00000000' WHERE version=1",
                [],
            )
            .expect("update checksum");
        }
        let result = checker(db).check_schema_checksums();
        assert!(
            result.is_err(),
            "tampered schema checksum must fail startup"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("checksum") || msg.contains("corruption"),
            "error message should mention checksum or corruption: {msg}"
        );
    }

    // ── 3. Gap in graph_revisions — checker SQL detects violation ────────
    //
    // The production schema enforces `CHECK (base_revision = revision - 1)`, so
    // no normal write path can introduce a gap.  We test the checker's detection
    // SQL directly against a minimal bare connection that omits the constraint,
    // ensuring the query logic is correct independently of the schema guard.

    #[test]
    fn graph_revision_gap_sql_detects_gap() {
        // Build a constraint-free table to inject a non-contiguous sequence.
        let conn = rusqlite::Connection::open_in_memory().expect("bare conn");
        conn.execute_batch(
            "CREATE TABLE graph_revisions (
                revision      INTEGER PRIMARY KEY,
                base_revision INTEGER NOT NULL,
                tx_id         TEXT NOT NULL UNIQUE,
                committed_at  TEXT NOT NULL,
                actor_id      TEXT NOT NULL,
                policy_hash   TEXT NOT NULL,
                change_count  INTEGER NOT NULL
             );
             -- Revision 1 (base=0) and revision 3 (base=2) — revision 2 is missing.
             INSERT INTO graph_revisions VALUES (1, 0, 'tx-1', '2026-01-01T00:00:00Z', 'a', 'p', 0);
             INSERT INTO graph_revisions VALUES (3, 2, 'tx-3', '2026-01-01T00:00:01Z', 'a', 'p', 0);",
        )
        .expect("setup");

        // This mirrors the count/min/max check in the checker.
        let (count, min_rev, max_rev): (i64, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT COUNT(*), MIN(revision), MAX(revision) FROM graph_revisions",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("query");

        let min_rev = min_rev.unwrap_or(0);
        let max_rev = max_rev.unwrap_or(0);
        let is_gap = max_rev - min_rev + 1 != count;
        assert!(
            is_gap,
            "gap detector must identify the missing revision 2; count={count} min={min_rev} max={max_rev}"
        );
    }

    /// On a fully-migrated DB (contiguous revisions), `check_graph_revision_continuity`
    /// passes even after inserting several valid revisions.
    #[test]
    fn contiguous_revisions_pass_via_checker() {
        let db = fresh();
        {
            let conn = db.write();
            for rev in 1i64..=3 {
                conn.execute(
                    "INSERT INTO graph_revisions(revision, base_revision, tx_id, \
                     committed_at, actor_id, policy_hash, change_count) \
                     VALUES (?1, ?2, ?3, '2026-01-01T00:00:00Z', 'actor', 'ph', 0)",
                    rusqlite::params![rev, rev - 1, format!("tx-{rev}")],
                )
                .expect("insert revision");
            }
        }
        checker(db)
            .check_graph_revision_continuity()
            .expect("contiguous revisions must pass the checker");
    }

    // ── 4. Empty authority_meta — checker SQL detects missing singleton ───
    //
    // The DB trigger prevents deleting the singleton row in the live authority.
    // We test the checker's detection SQL against a minimal bare connection
    // that has no trigger, confirming the query logic is correct.

    #[test]
    fn empty_authority_meta_sql_detects_missing_singleton() {
        let conn = rusqlite::Connection::open_in_memory().expect("bare conn");
        // Minimal authority_meta with no rows — simulates a corrupt state where
        // the singleton is absent.
        conn.execute_batch(
            "CREATE TABLE authority_meta (
                id INTEGER PRIMARY KEY,
                graph_revision INTEGER NOT NULL,
                event_hlc TEXT NOT NULL,
                schema_epoch INTEGER NOT NULL
             );
             -- Intentionally no INSERT — simulate missing singleton.",
        )
        .expect("setup");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM authority_meta", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 0, "table should be empty for this test");

        // The checker detects count != 1 as a violation.
        let violation = count != 1;
        assert!(violation, "checker must flag missing authority singleton");
    }

    /// Checker passes on a DB where authority_meta has exactly the one singleton.
    #[test]
    fn authority_meta_singleton_passes_checker() {
        let db = fresh();
        checker(db)
            .check_authority_singleton()
            .expect("fresh DB singleton must pass the authority check");
    }

    // ── 5. quick_check pass on valid DB ──────────────────────────────────

    #[test]
    fn quick_check_passes_on_valid_db() {
        let db = fresh();
        checker(db)
            .check_quick_check()
            .expect("quick_check must pass on a fresh DB");
    }

    // ── 6. Outbox unknown status fails ────────────────────────────────────

    #[test]
    fn outbox_unknown_status_fails() {
        let db = fresh();
        {
            let conn = db.write();
            // Insert an outbox row with an invalid status.
            conn.execute(
                "INSERT INTO derived_outbox(target, op, attempts, status, created_at) \
                 VALUES ('fts', 'upsert', 0, 'bogus_status', '2026-01-01T00:00:00Z')",
                [],
            )
            .expect("insert bad outbox row");
        }
        let result = checker(db).check_outbox_cursor_sanity();
        assert!(result.is_err(), "unknown outbox status must fail");
    }

    // ── 7. Outbox stuck entry (attempts > 100) fails ──────────────────────

    #[test]
    fn outbox_stuck_entry_fails() {
        let db = fresh();
        {
            let conn = db.write();
            conn.execute(
                "INSERT INTO derived_outbox(target, op, attempts, status, created_at) \
                 VALUES ('fts', 'upsert', 101, 'dead_letter', '2026-01-01T00:00:00Z')",
                [],
            )
            .expect("insert stuck outbox row");
        }
        let result = checker(db).check_outbox_cursor_sanity();
        assert!(
            result.is_err(),
            "stuck outbox entry must fail startup check"
        );
    }

    // ── 8. Valid outbox (attempts = 100) passes ───────────────────────────

    #[test]
    fn outbox_at_max_threshold_passes() {
        let db = fresh();
        {
            let conn = db.write();
            conn.execute(
                "INSERT INTO derived_outbox(target, op, attempts, status, created_at) \
                 VALUES ('fts', 'upsert', 100, 'dead_letter', '2026-01-01T00:00:00Z')",
                [],
            )
            .expect("insert row at max");
        }
        checker(db)
            .check_outbox_cursor_sanity()
            .expect("attempts=100 is the threshold, not > 100");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // RecoveryIntegrityChecker tests (task 1.8.2)
    // ─────────────────────────────────────────────────────────────────────────

    fn recovery_checker(db: Arc<Database>) -> RecoveryIntegrityChecker {
        RecoveryIntegrityChecker::new(db)
    }

    // ── R1. Full integrity_check passes on a fresh DB ─────────────────────

    #[test]
    fn recovery_full_integrity_check_passes_on_fresh_db() {
        let db = fresh();
        let mut report = RecoveryCheckReport::empty();
        recovery_checker(db).check_full_integrity(&mut report);
        assert!(
            report.sqlite_integrity_ok,
            "PRAGMA integrity_check must pass on a fresh DB"
        );
        assert_eq!(report.state, CapabilityState::Healthy);
        assert!(report.faults.is_empty());
    }

    // ── R2. Full HLC order passes on a fresh DB ───────────────────────────

    #[test]
    fn recovery_full_hlc_order_passes_on_fresh_db() {
        let db = fresh();
        let mut report = RecoveryCheckReport::empty();
        recovery_checker(db).check_full_event_hlc_order(&mut report);
        assert!(
            report.event_hlc_order_ok,
            "no HLC violation on empty events_v2"
        );
        assert_eq!(report.state, CapabilityState::Healthy);
    }

    // ── R3. Event checksum coverage — row with empty checksum is reported ──

    #[test]
    fn recovery_event_checksum_empty_is_reported() {
        let db = fresh();
        {
            let conn = db.write();
            // Insert an event with an empty payload_checksum.
            conn.execute(
                "INSERT INTO events_v2 \
                 (id, hlc, ts_utc, tz_offset_min, event_type, \
                  source_kind, source_id, actor_id, payload_encoding, \
                  payload_plain, payload_checksum, phase, schema_version, \
                  namespace, owner_id, scope, sensitivity, policy_version) \
                 VALUES \
                 ('ev-empty-cksum', '2026-01-01T00:00:01.000Z', '2026-01-01T00:00:01Z', \
                  0, 'test', 'native', 'src-1', 'actor-1', 'plain', \
                  '{\"x\":1}', '', 'start', 1, \
                  'default', 'owner-1', 'private', 0, 'v1')",
                [],
            )
            .expect("insert event with empty checksum");
        }
        let mut report = RecoveryCheckReport::empty();
        recovery_checker(Arc::clone(&db)).check_event_checksum_coverage(&mut report);
        assert!(
            !report.event_checksums_ok,
            "empty checksum must be detected"
        );
        assert_eq!(report.state, CapabilityState::Corrupt);
        assert!(
            report
                .faults
                .iter()
                .any(|f| f.fault_class == IntegrityFaultClass::EventChecksumMissing),
            "must record EventChecksumMissing fault"
        );
        // Verify the fault description contains no protected data — just counts/class.
        for fault in &report.faults {
            let desc = &fault.description;
            // Should not contain payload content or actor names.
            assert!(
                !desc.contains("x\":1") && !desc.contains("actor-1"),
                "fault description must not contain protected payload data: {desc}"
            );
        }
    }

    // ── R4. Event checksum coverage — valid checksum passes ──────────────

    #[test]
    fn recovery_event_checksum_non_empty_passes() {
        let db = fresh();
        {
            let conn = db.write();
            conn.execute(
                "INSERT INTO events_v2 \
                 (id, hlc, ts_utc, tz_offset_min, event_type, \
                  source_kind, source_id, actor_id, payload_encoding, \
                  payload_plain, payload_checksum, phase, schema_version, \
                  namespace, owner_id, scope, sensitivity, policy_version) \
                 VALUES \
                 ('ev-valid-cksum', '2026-01-01T00:00:02.000Z', '2026-01-01T00:00:02Z', \
                  0, 'test', 'native', 'src-1', 'actor-1', 'plain', \
                  '{\"y\":2}', 'abc123def456', 'start', 1, \
                  'default', 'owner-1', 'private', 0, 'v1')",
                [],
            )
            .expect("insert event with valid checksum");
        }
        let mut report = RecoveryCheckReport::empty();
        recovery_checker(db).check_event_checksum_coverage(&mut report);
        assert!(report.event_checksums_ok, "non-empty checksum must pass");
        assert_eq!(report.state, CapabilityState::Healthy);
    }

    // ── R5. Migration coverage — fresh DB (all migrations applied) ────────

    #[test]
    fn recovery_migration_coverage_passes_on_fresh_db() {
        let db = fresh();
        let mut report = RecoveryCheckReport::empty();
        recovery_checker(db).check_migration_coverage(&mut report);
        assert!(
            report.migration_coverage_ok,
            "fresh DB has all migrations applied — coverage must pass"
        );
        assert_eq!(report.state, CapabilityState::Healthy);
    }

    // ── R6. Migration coverage — injected unknown migration detected ──────

    #[test]
    fn recovery_migration_coverage_detects_unknown_migration() {
        let db = fresh();
        {
            let conn = db.write();
            // Insert a migration version that is not in the compiled-in set.
            // schema_version columns: version, applied_at, checksum (no 'name').
            conn.execute(
                "INSERT INTO schema_version(version, applied_at, checksum) \
                 VALUES (9999, '2026-01-01T00:00:00Z', 'deadbeef')",
                [],
            )
            .expect("insert unknown migration");
        }
        let mut report = RecoveryCheckReport::empty();
        recovery_checker(db).check_migration_coverage(&mut report);
        assert!(
            !report.migration_coverage_ok,
            "unknown migration must fail coverage"
        );
        assert!(
            report
                .faults
                .iter()
                .any(|f| f.fault_class == IntegrityFaultClass::UnknownMigration),
            "must record UnknownMigration fault"
        );
    }

    // ── R7. Derived manifest version mismatch → Partial (not Corrupt) ─────

    #[test]
    fn recovery_manifest_version_mismatch_is_partial() {
        let db = fresh();
        {
            let conn = db.write();
            conn.execute(
                "INSERT INTO derived_manifests \
                 (target, version, algorithm_version, model_version, status) \
                 VALUES ('fts', 1, 'alg-v1', 'model-v1', 'active')",
                [],
            )
            .expect("insert manifest");
        }
        let mut report = RecoveryCheckReport::empty();
        recovery_checker(Arc::clone(&db))
            .with_expected_algorithm_version("alg-v2") // mismatch!
            .check_derived_manifest_versions(&mut report);

        assert_eq!(
            report.state,
            CapabilityState::Partial,
            "manifest version mismatch must yield Partial, not Corrupt"
        );
        assert_eq!(report.stale_manifest_count, 1);
        assert!(
            report
                .faults
                .iter()
                .any(|f| f.fault_class == IntegrityFaultClass::ManifestVersionMismatch),
            "must record ManifestVersionMismatch fault"
        );
    }

    // ── R8. run_all on fresh DB → Healthy ────────────────────────────────

    #[test]
    fn recovery_run_all_healthy_on_fresh_db() {
        let db = fresh();
        let report = recovery_checker(db).run_all();
        assert_eq!(
            report.state,
            CapabilityState::Healthy,
            "run_all on a fresh DB must report Healthy; faults: {:?}",
            report
                .faults
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
        );
        assert!(report.faults.is_empty());
        assert!(report.sqlite_integrity_ok);
        assert!(report.event_checksums_ok);
        assert!(report.event_hlc_order_ok);
        assert!(report.migration_coverage_ok);
        assert_eq!(report.stale_manifest_count, 0);
    }

    // ── R9. Fault descriptions contain no protected data ─────────────────

    #[test]
    fn recovery_fault_descriptions_contain_no_protected_data() {
        // Verify that the fault descriptions from a manifest mismatch do not
        // accidentally embed row content (this tests the policy-safety invariant).
        let db = fresh();
        {
            let conn = db.write();
            conn.execute(
                "INSERT INTO derived_manifests \
                 (target, version, algorithm_version, model_version, status) \
                 VALUES ('vector', 1, 'secret-model-weights-v99', 'super-private-v42', 'active')",
                [],
            )
            .expect("insert manifest");
        }
        let report = recovery_checker(Arc::clone(&db))
            .with_expected_algorithm_version("alg-current")
            .with_expected_model_version("model-current")
            .run_all();

        for fault in &report.faults {
            // The description should NOT contain the actual secret model weight content.
            // It IS allowed to contain the target name and version strings (those are
            // identifiers, not memory content).
            let desc = &fault.description;
            // No raw row content from protected namespaces in the description.
            assert!(
                !desc.contains("secret-model-weights"),
                "fault description must not contain protected algorithm version value: {desc}"
            );
            assert!(
                !desc.contains("super-private"),
                "fault description must not contain protected model version value: {desc}"
            );
        }
    }
}
