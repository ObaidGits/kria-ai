//! Workflow Telemetry Persistence — SQLite-backed Workflow Traces.
//!
//! Persists workflow telemetry for:
//! - Debugging failed workflows after the fact
//! - Replay and eval analysis
//! - Crash recovery (resume interrupted workflows)
//! - Production forensics
//!
//! # Design
//!
//! - Append-only event model (never mutates past events)
//! - Monotonic sequence preservation
//! - Bounded storage (auto-prunes old workflows)
//! - Efficient timeline reconstruction

use crate::agent::workflow_types::*;

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Persistence Store
// ═══════════════════════════════════════════════════════════════════════════════

/// SQLite-backed workflow telemetry store.
pub struct WorkflowTelemetryStore {
    conn: rusqlite::Connection,
    max_workflows: usize,
}

impl WorkflowTelemetryStore {
    /// Create a new store at the given path.
    pub fn new(db_path: std::path::PathBuf) -> Result<Self, String> {
        let conn = rusqlite::Connection::open(&db_path)
            .map_err(|e| format!("Failed to open workflow DB: {}", e))?;
        let store = Self { conn, max_workflows: 100 };
        store.initialize_schema()?;
        Ok(store)
    }

    /// Create an in-memory store (for testing).
    pub fn in_memory() -> Result<Self, String> {
        let conn = rusqlite::Connection::open_in_memory()
            .map_err(|e| format!("Failed to open in-memory DB: {}", e))?;
        let store = Self { conn, max_workflows: 50 };
        store.initialize_schema()?;
        Ok(store)
    }

    fn initialize_schema(&self) -> Result<(), String> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS workflow_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                workflow_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                event_json TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                source TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_workflow_events_wf_id
                ON workflow_events (workflow_id, seq);
            CREATE INDEX IF NOT EXISTS idx_workflow_events_created
                ON workflow_events (created_at);

            CREATE TABLE IF NOT EXISTS workflow_summaries (
                workflow_id TEXT PRIMARY KEY,
                user_text TEXT,
                verdict TEXT,
                duration_ms INTEGER,
                step_count INTEGER,
                source TEXT,
                capabilities_json TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_workflow_summaries_created
                ON workflow_summaries (created_at DESC);",
        )
        .map_err(|e| format!("Failed to initialize workflow schema: {}", e))
    }

    /// Persist a telemetry event.
    pub fn persist_event(&self, envelope: &TelemetryEnvelope) -> Result<(), String> {
        let conn = &self.conn;
        let workflow_id = extract_workflow_id_from_event(&envelope.event);
        let event_type = event_type_name(&envelope.event);
        let event_json = serde_json::to_string(&envelope.event)
            .map_err(|e| format!("Serialize error: {}", e))?;

        conn.execute(
            "INSERT INTO workflow_events (workflow_id, seq, event_type, event_json, timestamp_ms, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                workflow_id,
                envelope.seq,
                event_type,
                event_json,
                envelope.timestamp_ms,
                format!("{:?}", envelope.source),
            ],
        )
        .map_err(|e| format!("Insert error: {}", e))?;

        Ok(())
    }

    /// Persist a workflow summary (called on completion).
    pub fn persist_summary(
        &self,
        workflow_id: &str,
        user_text: &str,
        verdict: &WorkflowVerdict,
        duration_ms: u64,
        step_count: u32,
        source: WorkflowSource,
        capabilities: Option<&CapabilitySet>,
    ) -> Result<(), String> {
        let conn = &self.conn;
        let verdict_str = format!("{:?}", verdict);
        let caps_json = capabilities
            .map(|c| serde_json::to_string(c).unwrap_or_default())
            .unwrap_or_default();

        conn.execute(
            "INSERT OR REPLACE INTO workflow_summaries
             (workflow_id, user_text, verdict, duration_ms, step_count, source, capabilities_json, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
            rusqlite::params![
                workflow_id,
                user_text,
                verdict_str,
                duration_ms as i64,
                step_count as i64,
                format!("{:?}", source),
                caps_json,
            ],
        )
        .map_err(|e| format!("Summary insert error: {}", e))?;

        self.prune_old_workflows()?;
        Ok(())
    }

    /// Load the telemetry timeline for a workflow.
    pub fn load_workflow_timeline(&self, workflow_id: &str) -> Result<Vec<TelemetryEnvelope>, String> {
        let conn = &self.conn;
        let mut stmt = conn
            .prepare(
                "SELECT seq, event_json, timestamp_ms, source
                 FROM workflow_events
                 WHERE workflow_id = ?1
                 ORDER BY seq ASC",
            )
            .map_err(|e| format!("Prepare error: {}", e))?;

        let events = stmt
            .query_map(rusqlite::params![workflow_id], |row| {
                let seq: u64 = row.get(0)?;
                let event_json: String = row.get(1)?;
                let timestamp_ms: u64 = row.get(2)?;
                let source_str: String = row.get(3)?;
                Ok((seq, event_json, timestamp_ms, source_str))
            })
            .map_err(|e| format!("Query error: {}", e))?
            .filter_map(|r| r.ok())
            .filter_map(|(seq, json, ts, src)| {
                let event: WorkflowTelemetry = serde_json::from_str(&json).ok()?;
                let source = match src.as_str() {
                    "SubstrateRouter" => WorkflowSource::SubstrateRouter,
                    "LegacyShim" => WorkflowSource::LegacyShim,
                    _ => WorkflowSource::ReactLoop,
                };
                Some(TelemetryEnvelope {
                    version: TELEMETRY_VERSION,
                    seq,
                    event,
                    timestamp_ms: ts,
                    source,
                })
            })
            .collect();

        Ok(events)
    }

    /// Load recent workflow summaries.
    pub fn load_recent_summaries(&self, limit: usize) -> Result<Vec<WorkflowSummaryRecord>, String> {
        let conn = &self.conn;
        let mut stmt = conn
            .prepare(
                "SELECT workflow_id, user_text, verdict, duration_ms, step_count, source, created_at
                 FROM workflow_summaries
                 ORDER BY created_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| format!("Prepare error: {}", e))?;

        let summaries = stmt
            .query_map(rusqlite::params![limit as i64], |row| {
                Ok(WorkflowSummaryRecord {
                    workflow_id: row.get(0)?,
                    user_text: row.get(1)?,
                    verdict: row.get(2)?,
                    duration_ms: row.get::<_, i64>(3)? as u64,
                    step_count: row.get::<_, i64>(4)? as u32,
                    source: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(|e| format!("Query error: {}", e))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(summaries)
    }

    /// Prune old workflows to stay within storage bounds.
    fn prune_old_workflows(&self) -> Result<(), String> {
        self.conn.execute(
            "DELETE FROM workflow_events WHERE workflow_id NOT IN (
                SELECT workflow_id FROM workflow_summaries
                ORDER BY created_at DESC LIMIT ?1
            )",
            rusqlite::params![self.max_workflows as i64],
        )
        .map_err(|e| format!("Prune error: {}", e))?;

        self.conn.execute(
            "DELETE FROM workflow_summaries WHERE workflow_id NOT IN (
                SELECT workflow_id FROM workflow_summaries
                ORDER BY created_at DESC LIMIT ?1
            )",
            rusqlite::params![self.max_workflows as i64],
        )
        .map_err(|e| format!("Prune summaries error: {}", e))?;

        Ok(())
    }
}

/// A persisted workflow summary record.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkflowSummaryRecord {
    pub workflow_id: String,
    pub user_text: String,
    pub verdict: String,
    pub duration_ms: u64,
    pub step_count: u32,
    pub source: String,
    pub created_at: String,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Helpers
// ═══════════════════════════════════════════════════════════════════════════════

fn extract_workflow_id_from_event(event: &WorkflowTelemetry) -> String {
    match event {
        WorkflowTelemetry::Started { workflow_id, .. }
        | WorkflowTelemetry::PlanPreview { workflow_id, .. }
        | WorkflowTelemetry::StepStarted { workflow_id, .. }
        | WorkflowTelemetry::StepCompleted { workflow_id, .. }
        | WorkflowTelemetry::HitlRequired { workflow_id, .. }
        | WorkflowTelemetry::Completed { workflow_id, .. }
        | WorkflowTelemetry::Cancelled { workflow_id, .. } => workflow_id.clone(),
    }
}

fn event_type_name(event: &WorkflowTelemetry) -> &'static str {
    match event {
        WorkflowTelemetry::Started { .. } => "started",
        WorkflowTelemetry::PlanPreview { .. } => "plan_preview",
        WorkflowTelemetry::StepStarted { .. } => "step_started",
        WorkflowTelemetry::StepCompleted { .. } => "step_completed",
        WorkflowTelemetry::HitlRequired { .. } => "hitl_required",
        WorkflowTelemetry::Completed { .. } => "completed",
        WorkflowTelemetry::Cancelled { .. } => "cancelled",
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_creates_and_persists_events() {
        let store = WorkflowTelemetryStore::in_memory().unwrap();

        let envelope = TelemetryEnvelope {
            version: 1,
            seq: 1,
            event: WorkflowTelemetry::Started {
                workflow_id: "test-wf-1".into(),
                title: "Test".into(),
                steps: vec![],
                execution_mode: ExecutionMode::Structural,
                estimated_duration_ms: Some(5000),
            },
            timestamp_ms: 0,
            source: WorkflowSource::SubstrateRouter,
        };

        store.persist_event(&envelope).unwrap();

        let timeline = store.load_workflow_timeline("test-wf-1").unwrap();
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].seq, 1);
    }

    #[test]
    fn store_persists_and_loads_summaries() {
        let store = WorkflowTelemetryStore::in_memory().unwrap();

        store.persist_summary(
            "wf-1",
            "open firefox",
            &WorkflowVerdict::Complete,
            1500,
            2,
            WorkflowSource::SubstrateRouter,
            None,
        ).unwrap();

        let summaries = store.load_recent_summaries(10).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].workflow_id, "wf-1");
        assert_eq!(summaries[0].step_count, 2);
    }

    #[test]
    fn store_prunes_old_workflows() {
        let mut store = WorkflowTelemetryStore::in_memory().unwrap();
        store.max_workflows = 3;

        for i in 0..5 {
            store.persist_summary(
                &format!("wf-{}", i),
                "test",
                &WorkflowVerdict::Complete,
                100,
                1,
                WorkflowSource::SubstrateRouter,
                None,
            ).unwrap();
        }

        let summaries = store.load_recent_summaries(10).unwrap();
        assert!(summaries.len() <= 3, "Should prune to max_workflows");
    }

    #[test]
    fn timeline_preserves_ordering() {
        let store = WorkflowTelemetryStore::in_memory().unwrap();

        for seq in 1..=5 {
            let envelope = TelemetryEnvelope {
                version: 1,
                seq,
                event: WorkflowTelemetry::StepStarted {
                    workflow_id: "wf-order".into(),
                    step_index: seq as u32,
                    description: format!("Step {}", seq),
                    step_type: StepType::CommandExecution,
                },
                timestamp_ms: seq * 100,
                source: WorkflowSource::SubstrateRouter,
            };
            store.persist_event(&envelope).unwrap();
        }

        let timeline = store.load_workflow_timeline("wf-order").unwrap();
        assert_eq!(timeline.len(), 5);
        for (i, event) in timeline.iter().enumerate() {
            assert_eq!(event.seq, (i + 1) as u64);
        }
    }
}
