//! Observability: explain + health report + metrics (memory-upgrade design §28, L6).
//!
//! Every memory is explainable (L6): `explain_memory` reconstructs a memory's
//! provenance chain, contradictions, Memory Worth, and access history;
//! `memory_health_report` summarizes the bank for the "what KRIA believes about
//! you" surface. Read-only.
//!
//! ## Scheduler Telemetry (F3.8 / task 3.8.6)
//!
//! [`SchedulerMetrics`] aggregates scheduler counters with **zero heap allocation
//! and zero locking in hot paths**.  All increments use `Relaxed` atomics;
//! [`SchedulerMetrics::snapshot`] reads all fields with `Relaxed` as well —
//! sufficient for telemetry that needs only eventual visibility, never a fenced
//! consistent read.
//!
//! **Privacy invariant**: no user content, no invocation IDs, no record IDs.
//! Only aggregate counts (§39 / MGR-039).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::db::Database;
use crate::error::{MemoryResult, StorageError};

// ---------------------------------------------------------------------------
// F3.8 / task 3.8.6 — Scheduler telemetry (aggregate, redacted, lock-free)
// ---------------------------------------------------------------------------

/// Aggregate scheduler telemetry counters.
///
/// All fields are `AtomicU64` so hot-path recording requires **no allocation
/// and no locking**.  The struct is safe to share via `Arc<SchedulerMetrics>`.
///
/// **Privacy**: only aggregate counts — no user content, no invocation IDs,
/// no record IDs (§39 / MGR-039).
///
/// **Overhead budget**: each `record_*` call is a single `fetch_add` with
/// `Relaxed` ordering — estimated ≤ 2 ns/call on modern hardware, well within
/// the ≤1 % CPU and ≤1 % interactive-latency overhead budget (MGR-009,
/// MGR-022, MGR-028, MGR-039, MGR-042, MGR-045; MGD-015).
pub struct SchedulerMetrics {
    /// Total jobs successfully dispatched to a worker.
    pub jobs_dispatched: AtomicU64,
    /// Jobs dropped because the bounded queue was at capacity.
    pub jobs_dropped_cap: AtomicU64,
    /// Jobs silently coalesced (duplicate coalescing key already queued).
    pub jobs_coalesced: AtomicU64,
    /// Times a background job was preempted by a higher-priority foreground
    /// arrival or by the 100 ms time-slice budget.
    pub jobs_preempted: AtomicU64,
    /// Arrivals of P0 (foreground) work — used to gauge preemption pressure.
    pub p0_arrivals: AtomicU64,
    /// Last-known queue depth (backlog).  Written on every push/pop, never
    /// fenced; the telemetry consumer reads an eventually-consistent value.
    pub backlog_size: AtomicU64,
    /// P3/P4 suspension events due to resource pressure (battery / memory /
    /// thermal / model), i.e., degradation events.
    pub degradation_events: AtomicU64,
}

impl SchedulerMetrics {
    /// Create a zero-initialised metrics set.  Stack-allocatable; wrap in
    /// `Arc` when sharing across threads.
    pub fn new() -> Self {
        Self {
            jobs_dispatched: AtomicU64::new(0),
            jobs_dropped_cap: AtomicU64::new(0),
            jobs_coalesced: AtomicU64::new(0),
            jobs_preempted: AtomicU64::new(0),
            p0_arrivals: AtomicU64::new(0),
            backlog_size: AtomicU64::new(0),
            degradation_events: AtomicU64::new(0),
        }
    }

    // ── record_* helpers — one `fetch_add` each, hot-path safe ─────────────

    /// Record a successfully dispatched job.
    #[inline]
    pub fn record_dispatched(&self) {
        self.jobs_dispatched.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a job dropped because the queue was at its configured cap.
    #[inline]
    pub fn record_dropped_cap(&self) {
        self.jobs_dropped_cap.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a job that was coalesced (duplicate key already queued).
    #[inline]
    pub fn record_coalesced(&self) {
        self.jobs_coalesced.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a preemption event (foreground arrival or time-slice expiry).
    #[inline]
    pub fn record_preempted(&self) {
        self.jobs_preempted.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a P0 foreground arrival.
    #[inline]
    pub fn record_p0_arrival(&self) {
        self.p0_arrivals.fetch_add(1, Ordering::Relaxed);
    }

    /// Update the last-known backlog depth (current queue length).
    ///
    /// This is a `store`, not an `add`, because the value represents a gauge
    /// rather than a monotonic counter.
    #[inline]
    pub fn update_backlog_size(&self, depth: u64) {
        self.backlog_size.store(depth, Ordering::Relaxed);
    }

    /// Record a degradation event (P3/P4 suspended due to resource pressure).
    #[inline]
    pub fn record_degradation_event(&self) {
        self.degradation_events.fetch_add(1, Ordering::Relaxed);
    }

    /// Read all counters atomically for telemetry reporting.
    ///
    /// Uses `Relaxed` ordering throughout — appropriate for telemetry that
    /// only needs eventual visibility and never drives correctness decisions.
    /// No fence is inserted; the caller must not rely on this as a
    /// synchronisation point.
    pub fn snapshot(&self) -> SchedulerMetricsSnapshot {
        SchedulerMetricsSnapshot {
            jobs_dispatched: self.jobs_dispatched.load(Ordering::Relaxed),
            jobs_dropped_cap: self.jobs_dropped_cap.load(Ordering::Relaxed),
            jobs_coalesced: self.jobs_coalesced.load(Ordering::Relaxed),
            jobs_preempted: self.jobs_preempted.load(Ordering::Relaxed),
            p0_arrivals: self.p0_arrivals.load(Ordering::Relaxed),
            backlog_size: self.backlog_size.load(Ordering::Relaxed),
            degradation_events: self.degradation_events.load(Ordering::Relaxed),
        }
    }
}

impl Default for SchedulerMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// A point-in-time snapshot of [`SchedulerMetrics`] — plain `u64` fields
/// suitable for serialisation, logging, or health-report inclusion.
///
/// **Privacy**: only aggregate counts — no user content, no invocation IDs,
/// no record IDs (§39 / MGR-039).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SchedulerMetricsSnapshot {
    /// Total jobs successfully dispatched to a worker.
    pub jobs_dispatched: u64,
    /// Jobs dropped because the bounded queue was at capacity.
    pub jobs_dropped_cap: u64,
    /// Jobs silently coalesced (duplicate coalescing key already queued).
    pub jobs_coalesced: u64,
    /// Times a background job was preempted.
    pub jobs_preempted: u64,
    /// Arrivals of P0 (foreground) work.
    pub p0_arrivals: u64,
    /// Last-known queue depth (backlog) at snapshot time.
    pub backlog_size: u64,
    /// P3/P4 suspension events due to resource pressure.
    pub degradation_events: u64,
}

/// Provenance + status explanation for a single memory (L6).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemoryExplanation {
    pub id: Uuid,
    pub content: String,
    pub memory_type: String,
    pub state: String,
    pub confidence: f32,
    pub importance: f32,
    pub source_event_tag: Option<String>,
    pub derived_from: Vec<Uuid>,
    pub contradicts: Vec<Uuid>,
    pub worth_success: u32,
    pub worth_failure: u32,
    pub worth_samples: u32,
    pub access_count: u64,
    pub staleness_class: String,
    pub superseded_by: Option<Uuid>,
}

/// Aggregate health / "what KRIA believes about you" report (design §28).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemoryHealthReport {
    pub total_active: i64,
    pub total_archived: i64,
    pub total_superseded: i64,
    pub total_forgotten: i64,
    pub by_type: Vec<(String, i64)>,
    pub by_staleness: Vec<(String, i64)>,
    pub avg_confidence: f64,
    pub unresolved_contradictions: i64,
    pub knowledge_gaps: i64,
    pub enrichment_backlog: i64,
    pub outbox_pending: i64,
    /// Application-level cryptographic shredding capability (MGR-041 / design §5.4).
    /// Always `"unavailable — payload encryption not yet implemented; reliance on
    /// host OS disk encryption only"` until real encryption + key-destruction
    /// evidence exists.  Surfaced here so the "What KRIA believes about you"
    /// surface never falsely claims cryptographic erasure.
    pub crypto_shred_capability: String,
}

pub struct Observability {
    db: Arc<Database>,
}

impl Observability {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Explain a memory: provenance chain + status (L6). `None` if not found.
    pub fn explain_memory(&self, id: Uuid) -> MemoryResult<Option<MemoryExplanation>> {
        self.db.with_read(|conn| {
            let base: Option<(
                String,
                String,
                String,
                f64,
                f64,
                String,
                u32,
                u32,
                u32,
                i64,
                String,
                Option<String>,
                String,
            )> = conn
                .query_row(
                    "SELECT m.content, m.memory_type, m.state, m.confidence, m.importance, \
                     e.source, m.memory_worth_success, m.memory_worth_failure, \
                     m.memory_worth_samples, m.access_count, m.staleness_class, m.superseded_by, \
                     m.source_event_id \
                     FROM memories m JOIN events e ON m.source_event_id = e.id WHERE m.id = ?1",
                    params![id.to_string()],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get::<_, i64>(6)? as u32,
                            r.get::<_, i64>(7)? as u32,
                            r.get::<_, i64>(8)? as u32,
                            r.get::<_, i64>(9)?,
                            r.get(10)?,
                            r.get::<_, Option<String>>(11)?,
                            r.get(12)?,
                        ))
                    },
                )
                .optional()
                .map_err(StorageError::Sqlite)?;

            let Some((
                content,
                mtype,
                state,
                conf,
                imp,
                src,
                ws,
                wf,
                wsm,
                ac,
                stale,
                superseded,
                _sev,
            )) = base
            else {
                return Ok(None);
            };

            let derived_from = collect_ids(
                conn,
                "SELECT child_id FROM memory_derived_from WHERE parent_id = ?1",
                &id,
            )?;
            let contradicts = collect_ids(
                conn,
                "SELECT b_id FROM memory_contradicts WHERE a_id = ?1",
                &id,
            )?;

            Ok(Some(MemoryExplanation {
                id,
                content,
                memory_type: mtype,
                state,
                confidence: conf as f32,
                importance: imp as f32,
                source_event_tag: Some(src),
                derived_from,
                contradicts,
                worth_success: ws,
                worth_failure: wf,
                worth_samples: wsm,
                access_count: ac.max(0) as u64,
                staleness_class: stale,
                superseded_by: superseded.and_then(|s| Uuid::parse_str(&s).ok()),
            }))
        })
    }

    /// Build the aggregate health report (design §28).
    pub fn health_report(&self) -> MemoryResult<MemoryHealthReport> {
        self.db.with_read(|conn| {
            let count = |sql: &str| -> Result<i64, StorageError> {
                conn.query_row(sql, [], |r| r.get(0)).map_err(StorageError::Sqlite)
            };
            let mut report = MemoryHealthReport {
                total_active: count("SELECT COUNT(*) FROM memories WHERE state='active'")?,
                total_archived: count("SELECT COUNT(*) FROM memories WHERE state='archived'")?,
                total_superseded: count("SELECT COUNT(*) FROM memories WHERE state='superseded'")?,
                total_forgotten: count("SELECT COUNT(*) FROM memories WHERE state='forgotten'")?,
                unresolved_contradictions: count("SELECT COUNT(*) FROM memory_contradicts")?,
                knowledge_gaps: count("SELECT COUNT(*) FROM knowledge_gaps WHERE resolved=0")?,
                enrichment_backlog: count("SELECT COUNT(*) FROM enrichment_deadletter")?,
                outbox_pending: count("SELECT COUNT(*) FROM embedding_outbox WHERE status='pending'")?,
                avg_confidence: conn
                    .query_row(
                        "SELECT COALESCE(AVG(confidence),0) FROM memories WHERE state='active'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(StorageError::Sqlite)?,
                by_type: Vec::new(),
                by_staleness: Vec::new(),
                // Populated below after the struct literal.
                crypto_shred_capability: String::new(),
            };

            report.by_type = group_counts(
                conn,
                "SELECT memory_type, COUNT(*) FROM memories WHERE state='active' GROUP BY memory_type",
            )?;
            report.by_staleness = group_counts(
                conn,
                "SELECT staleness_class, COUNT(*) FROM memories WHERE state='active' GROUP BY staleness_class",
            )?;
            // MGR-041 / design §5.4: always "unavailable" — content is plaintext,
            // no payload encryption exists.  shred_keys.status='destroyed' is a
            // hard-delete flag only.  Never claim application-level unreadability.
            report.crypto_shred_capability =
                crate::api::CRYPTO_SHRED_CAPABILITY.to_owned();
            Ok(report)
        })
    }
}

fn collect_ids(
    conn: &rusqlite::Connection,
    sql: &str,
    key: &Uuid,
) -> Result<Vec<Uuid>, StorageError> {
    let mut stmt = conn.prepare(sql).map_err(StorageError::Sqlite)?;
    let rows = stmt
        .query_map(params![key.to_string()], |r| r.get::<_, String>(0))
        .map_err(StorageError::Sqlite)?;
    let mut out = Vec::new();
    for r in rows {
        if let Ok(u) = Uuid::parse_str(&r.map_err(StorageError::Sqlite)?) {
            out.push(u);
        }
    }
    Ok(out)
}

fn group_counts(
    conn: &rusqlite::Connection,
    sql: &str,
) -> Result<Vec<(String, i64)>, StorageError> {
    let mut stmt = conn.prepare(sql).map_err(StorageError::Sqlite)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(StorageError::Sqlite)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(StorageError::Sqlite)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stores::ports::{EventStore, RelationalStore};
    use crate::stores::{SqliteEventStore, SqliteRelationalStore};
    use crate::types::{
        Event, EventType, Memory, MemoryState, MemoryType, MemoryWorth, Modality, Scope,
        Sensitivity, Source, StalenessClass,
    };

    fn seed(db: &Arc<Database>) -> Uuid {
        let events = SqliteEventStore::new(db.clone());
        let rel = SqliteRelationalStore::new(db.clone());
        let ev = Event {
            id: crate::ids::new_id(),
            hlc: crate::ids::HlcGenerator::new().now(),
            ts_utc: chrono::Utc::now(),
            tz_offset_min: 0,
            event_type: EventType::UserMessage,
            source: Source::User,
            session_id: None,
            parent_event_id: None,
            shred_key_id: None,
            payload: serde_json::json!({}),
            encrypted: false,
            checksum: "c".into(),
        };
        let now = chrono::Utc::now();
        let m = Memory {
            id: crate::ids::new_id(),
            content: "the user prefers dark mode".into(),
            memory_type: MemoryType::Semantic,
            compression_level: 0,
            source_event_id: ev.id,
            namespace: "core".into(),
            owner_id: "user".into(),
            device_id: "d".into(),
            scope: Scope::Global,
            confidence: 0.9,
            importance: 6.0,
            access_count: 3,
            decay_score: 1.0,
            staleness_class: StalenessClass::Slow,
            sensitivity: Sensitivity::Private,
            state: MemoryState::Active,
            created_at: now,
            last_accessed: Some(now),
            valid_from: now,
            valid_until: None,
            embedding_id: None,
            embedding_model_version: None,
            estimated_tokens: 3,
            content_hash: "h".into(),
            shred_key_id: None,
            verify_against: None,
            superseded_by: None,
            episode_id: None,
            goal_context_id: None,
            worth: MemoryWorth {
                success: 5,
                failure: 1,
                samples: 6,
            },
            modality: Modality::Text,
            preference_pair_id: None,
            training_eligible: false,
        };
        let mut tx = db.begin().unwrap();
        events.append(&mut tx, &ev).unwrap();
        rel.upsert_memory(&mut tx, &m).unwrap();
        tx.commit().unwrap();
        m.id
    }

    #[test]
    fn explain_memory_returns_provenance() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let id = seed(&db);
        let obs = Observability::new(db.clone());
        let e = obs.explain_memory(id).unwrap().unwrap();
        assert_eq!(e.memory_type, "semantic");
        assert_eq!(e.state, "active");
        assert_eq!(e.worth_samples, 6);
        assert_eq!(e.source_event_tag.as_deref(), Some("user"));
        assert!(obs.explain_memory(Uuid::now_v7()).unwrap().is_none());
    }

    #[test]
    fn health_report_aggregates() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        seed(&db);
        let obs = Observability::new(db.clone());
        let r = obs.health_report().unwrap();
        assert_eq!(r.total_active, 1);
        assert!(r.avg_confidence > 0.8);
        assert_eq!(r.by_type, vec![("semantic".to_string(), 1)]);
        assert_eq!(r.knowledge_gaps, 0);
    }

    /// Validates: MGR-041 — the health report must explicitly disclose that
    /// application-level cryptographic shredding is unavailable until payload
    /// encryption, key destruction, and zero-plaintext evidence are all
    /// implemented.  It must NOT claim "Crypto-Shredded" or imply unreadability.
    #[test]
    fn health_report_crypto_shred_capability_is_unavailable() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        seed(&db);
        let obs = Observability::new(db.clone());
        let r = obs.health_report().unwrap();
        // Must contain "unavailable" so the UI/caller cannot misread it as
        // available or complete.
        assert!(
            r.crypto_shred_capability.contains("unavailable"),
            "crypto_shred_capability must contain 'unavailable', got: {:?}",
            r.crypto_shred_capability
        );
        // Must not claim cryptographic erasure is complete.
        let lower = r.crypto_shred_capability.to_lowercase();
        assert!(
            !lower.contains("complete") && !lower.contains("available\u{201c}"),
            "crypto_shred_capability must not claim completion, got: {:?}",
            r.crypto_shred_capability
        );
    }

    // -----------------------------------------------------------------------
    // SchedulerMetrics — F3.8 / task 3.8.6
    // -----------------------------------------------------------------------

    /// Validates: MGR-009, MGR-022, MGR-028, MGR-039, MGR-042, MGR-045; MGD-015.
    ///
    /// `jobs_dispatched` counter starts at zero and increments by 1 per call.
    #[test]
    fn scheduler_metrics_jobs_dispatched_increments() {
        let m = SchedulerMetrics::new();
        assert_eq!(m.snapshot().jobs_dispatched, 0);
        m.record_dispatched();
        m.record_dispatched();
        m.record_dispatched();
        assert_eq!(m.snapshot().jobs_dispatched, 3);
    }

    /// Validates: MGR-009, MGR-022, MGR-039; MGD-015.
    ///
    /// `jobs_dropped_cap` counter starts at zero and increments by 1 per call.
    #[test]
    fn scheduler_metrics_jobs_dropped_cap_increments() {
        let m = SchedulerMetrics::new();
        assert_eq!(m.snapshot().jobs_dropped_cap, 0);
        m.record_dropped_cap();
        m.record_dropped_cap();
        assert_eq!(m.snapshot().jobs_dropped_cap, 2);
    }

    /// Validates: MGR-009, MGR-022, MGR-039; MGD-015.
    ///
    /// `snapshot()` reads all seven fields and returns the correct values.
    #[test]
    fn scheduler_metrics_snapshot_captures_all_fields() {
        let m = SchedulerMetrics::new();
        m.record_dispatched();
        m.record_dropped_cap();
        m.record_coalesced();
        m.record_preempted();
        m.record_p0_arrival();
        m.update_backlog_size(42);
        m.record_degradation_event();

        let s = m.snapshot();
        assert_eq!(s.jobs_dispatched, 1, "jobs_dispatched");
        assert_eq!(s.jobs_dropped_cap, 1, "jobs_dropped_cap");
        assert_eq!(s.jobs_coalesced, 1, "jobs_coalesced");
        assert_eq!(s.jobs_preempted, 1, "jobs_preempted");
        assert_eq!(s.p0_arrivals, 1, "p0_arrivals");
        assert_eq!(s.backlog_size, 42, "backlog_size");
        assert_eq!(s.degradation_events, 1, "degradation_events");
    }

    /// Validates: MGR-009, MGR-022, MGR-039; MGD-015.
    ///
    /// Concurrent increments from multiple Tokio tasks produce the exact
    /// expected sum, confirming the `AtomicU64` implementation is data-race-free.
    #[tokio::test]
    async fn scheduler_metrics_concurrent_increments_produce_correct_sum() {
        let metrics = Arc::new(SchedulerMetrics::new());
        let tasks = 8usize;
        let per_task = 1_000usize;

        let handles: Vec<_> = (0..tasks)
            .map(|_| {
                let m = Arc::clone(&metrics);
                tokio::spawn(async move {
                    for _ in 0..per_task {
                        m.record_dispatched();
                    }
                })
            })
            .collect();

        for h in handles {
            h.await.expect("task panicked");
        }

        let expected = (tasks * per_task) as u64;
        assert_eq!(
            metrics.snapshot().jobs_dispatched,
            expected,
            "expected {} dispatched after concurrent increments",
            expected
        );
    }

    /// `SchedulerMetrics::new()` zero-initialises all counters.
    #[test]
    fn scheduler_metrics_new_zero_initialised() {
        let s = SchedulerMetrics::new().snapshot();
        assert_eq!(s, SchedulerMetricsSnapshot::default());
    }

    /// `Arc<SchedulerMetrics>` is `Send + Sync` — usable across threads.
    #[test]
    fn scheduler_metrics_arc_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Arc<SchedulerMetrics>>();
    }
}
