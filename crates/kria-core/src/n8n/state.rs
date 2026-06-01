use super::types::{N8nCallbackEnvelope, N8nRunStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

const SEEN_EVENT_TTL_MS: u64 = 600_000;
const MAX_SEEN_EVENTS: usize = 10_000;
const MAX_DEAD_LETTERS: usize = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct N8nWorkflowRunState {
    pub correlation_id: String,
    pub workflow_id: String,
    pub workflow_version: String,
    pub n8n_run_id: String,
    pub last_sequence_number: u64,
    pub status: N8nRunStatus,
    pub evidence_log: Vec<serde_json::Value>,
    pub side_effects: Vec<String>,
    pub terminal: bool,
}

impl N8nWorkflowRunState {
    pub fn new(
        correlation_id: impl Into<String>,
        workflow_id: impl Into<String>,
        workflow_version: impl Into<String>,
        n8n_run_id: impl Into<String>,
        status: N8nRunStatus,
        evidence: Vec<serde_json::Value>,
    ) -> Self {
        let terminal = status.is_terminal();
        Self {
            correlation_id: correlation_id.into(),
            workflow_id: workflow_id.into(),
            workflow_version: workflow_version.into(),
            n8n_run_id: n8n_run_id.into(),
            last_sequence_number: 0,
            status,
            evidence_log: evidence,
            side_effects: Vec::new(),
            terminal,
        }
    }

    fn from_envelope(envelope: &N8nCallbackEnvelope) -> Self {
        Self {
            correlation_id: envelope.correlation_id.clone(),
            workflow_id: envelope.workflow_id.clone(),
            workflow_version: envelope.workflow_version.clone(),
            n8n_run_id: envelope.n8n_run_id.clone(),
            last_sequence_number: envelope.sequence_number,
            status: envelope.status.clone(),
            evidence_log: vec![envelope.evidence.clone()],
            side_effects: envelope.side_effects.clone(),
            terminal: envelope.status.is_terminal(),
        }
    }

    fn apply(&mut self, envelope: &N8nCallbackEnvelope) {
        self.n8n_run_id = envelope.n8n_run_id.clone();
        self.last_sequence_number = envelope.sequence_number;
        self.status = envelope.status.clone();
        self.evidence_log.push(envelope.evidence.clone());
        self.side_effects.extend(envelope.side_effects.clone());
        self.terminal = envelope.status.is_terminal();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nIngestDecision {
    Accepted,
    Duplicate,
    OutOfOrder,
    TerminalAlreadyReached,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct N8nDeadLetter {
    pub reason: N8nIngestDecision,
    pub correlation_id: String,
    pub event_id: String,
    pub workflow_id: String,
    pub sequence_number: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct N8nInboxRecord {
    pub received_at_ms: u128,
    pub decision: N8nIngestDecision,
    pub envelope: N8nCallbackEnvelope,
}

#[derive(Default)]
pub struct N8nWorkflowStateStore {
    runs: Mutex<HashMap<String, N8nWorkflowRunState>>,
    seen_events: Mutex<HashMap<String, u64>>,
    dead_letters: Mutex<Vec<N8nDeadLetter>>,
    /// Maps correlation_id → session_id for chat-turn correlation.
    /// When a callback arrives, this tells us which chat session to inject into.
    session_map: Mutex<HashMap<String, String>>,
}

impl N8nWorkflowStateStore {
    pub fn ingest(&self, envelope: N8nCallbackEnvelope) -> N8nIngestDecision {
        {
            let mut seen_events = self.seen_events.lock().expect("n8n seen_events poisoned");
            prune_seen_events(&mut seen_events, state_now_ms());
            if seen_events.contains_key(&envelope.event_id) {
                self.record_dead_letter(&envelope, N8nIngestDecision::Duplicate);
                return N8nIngestDecision::Duplicate;
            }
            seen_events.insert(envelope.event_id.clone(), state_now_ms());
        }

        let mut runs = self.runs.lock().expect("n8n runs poisoned");
        match runs.get_mut(&envelope.correlation_id) {
            None => {
                runs.insert(
                    envelope.correlation_id.clone(),
                    N8nWorkflowRunState::from_envelope(&envelope),
                );
                N8nIngestDecision::Accepted
            }
            Some(run) if run.terminal => {
                self.record_dead_letter(&envelope, N8nIngestDecision::TerminalAlreadyReached);
                N8nIngestDecision::TerminalAlreadyReached
            }
            Some(run) if envelope.sequence_number <= run.last_sequence_number => {
                self.record_dead_letter(&envelope, N8nIngestDecision::OutOfOrder);
                N8nIngestDecision::OutOfOrder
            }
            Some(run) => {
                run.apply(&envelope);
                N8nIngestDecision::Accepted
            }
        }
    }

    pub fn get(&self, correlation_id: &str) -> Option<N8nWorkflowRunState> {
        self.runs
            .lock()
            .expect("n8n runs poisoned")
            .get(correlation_id)
            .cloned()
    }

    pub fn upsert_run(&self, mut next: N8nWorkflowRunState) -> N8nWorkflowRunState {
        next.terminal = next.status.is_terminal();
        let mut runs = self.runs.lock().expect("n8n runs poisoned");
        if let Some(existing) = runs.get_mut(&next.correlation_id) {
            if existing.terminal {
                return existing.clone();
            }
            if next.n8n_run_id.trim().is_empty() {
                next.n8n_run_id = existing.n8n_run_id.clone();
            }
            if next.last_sequence_number == 0 {
                next.last_sequence_number = existing.last_sequence_number.saturating_add(1);
            }
            *existing = next.clone();
            next
        } else {
            if next.last_sequence_number == 0 {
                next.last_sequence_number = 1;
            }
            runs.insert(next.correlation_id.clone(), next.clone());
            next
        }
    }

    pub fn runs(&self) -> Vec<N8nWorkflowRunState> {
        let mut runs = self
            .runs
            .lock()
            .expect("n8n runs poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        runs.sort_by(|a, b| a.correlation_id.cmp(&b.correlation_id));
        runs
    }

    pub fn dead_letters(&self) -> Vec<N8nDeadLetter> {
        self.dead_letters
            .lock()
            .expect("n8n dead_letters poisoned")
            .clone()
    }

    fn record_dead_letter(&self, envelope: &N8nCallbackEnvelope, reason: N8nIngestDecision) {
        let mut dead_letters = self.dead_letters.lock().expect("n8n dead_letters poisoned");
        dead_letters.push(N8nDeadLetter {
            reason,
            correlation_id: envelope.correlation_id.clone(),
            event_id: envelope.event_id.clone(),
            workflow_id: envelope.workflow_id.clone(),
            sequence_number: envelope.sequence_number,
        });
        let overflow = dead_letters.len().saturating_sub(MAX_DEAD_LETTERS);
        if overflow > 0 {
            dead_letters.drain(0..overflow);
        }
    }

    /// Register a correlation_id → session_id mapping.
    /// Called when a workflow is invoked from a chat turn.
    pub fn register_session(&self, correlation_id: &str, session_id: &str) {
        self.session_map
            .lock()
            .expect("n8n session_map poisoned")
            .insert(correlation_id.to_string(), session_id.to_string());
    }

    /// Look up which session_id a correlation_id belongs to.
    /// Used by callback handler to inject results into the correct chat.
    pub fn get_session(&self, correlation_id: &str) -> Option<String> {
        self.session_map
            .lock()
            .expect("n8n session_map poisoned")
            .get(correlation_id)
            .cloned()
    }

    /// Check for runs that have been non-terminal for longer than `deadline_ms`.
    /// Returns correlation_ids of runs that timed out so callers can emit governance.
    pub fn check_timeouts(&self, deadline_ms: u64) -> Vec<N8nWorkflowRunState> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut timed_out = Vec::new();
        let mut runs = self.runs.lock().expect("n8n runs poisoned");

        for run in runs.values_mut() {
            if run.terminal {
                continue;
            }
            // Use the last evidence timestamp as "last activity"
            // If no activity for deadline_ms, mark as timed out
            let last_activity_ms = run
                .evidence_log
                .last()
                .and_then(|e| e.get("occurred_at_ms").and_then(|v| v.as_u64()))
                .unwrap_or(0);

            let elapsed = now_ms.saturating_sub(last_activity_ms);
            if last_activity_ms > 0 && elapsed > deadline_ms {
                run.status = super::types::N8nRunStatus::TimedOut;
                run.terminal = true;
                timed_out.push(run.clone());
            }
        }

        timed_out
    }

    /// Evict completed/terminal runs older than `max_age_ms` from memory.
    /// Returns number of evicted runs.
    pub fn evict_old_runs(&self, max_age_ms: u64) -> usize {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut runs = self.runs.lock().expect("n8n runs poisoned");
        let before = runs.len();

        runs.retain(|_, run| {
            if !run.terminal {
                return true; // Keep non-terminal runs
            }
            let last_activity = run
                .evidence_log
                .last()
                .and_then(|e| e.get("occurred_at_ms").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let elapsed = now_ms.saturating_sub(last_activity);
            elapsed < max_age_ms // Keep if younger than max_age
        });

        before - runs.len()
    }

    pub fn seen_event_count(&self) -> usize {
        self.seen_events
            .lock()
            .expect("n8n seen_events poisoned")
            .len()
    }
}

fn state_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn prune_seen_events(seen_events: &mut HashMap<String, u64>, now_ms: u64) {
    seen_events.retain(|_, seen_at_ms| now_ms.saturating_sub(*seen_at_ms) <= SEEN_EVENT_TTL_MS);
    if seen_events.len() <= MAX_SEEN_EVENTS {
        return;
    }

    let mut by_age = seen_events
        .iter()
        .map(|(event_id, seen_at_ms)| (event_id.clone(), *seen_at_ms))
        .collect::<Vec<_>>();
    by_age.sort_by_key(|(_, seen_at_ms)| *seen_at_ms);
    let overflow = seen_events.len().saturating_sub(MAX_SEEN_EVENTS);
    for (event_id, _) in by_age.into_iter().take(overflow) {
        seen_events.remove(&event_id);
    }
}
