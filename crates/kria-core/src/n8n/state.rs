use super::types::{N8nCallbackEnvelope, N8nRunStatus};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

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
    seen_events: Mutex<HashSet<String>>,
    dead_letters: Mutex<Vec<N8nDeadLetter>>,
}

impl N8nWorkflowStateStore {
    pub fn ingest(&self, envelope: N8nCallbackEnvelope) -> N8nIngestDecision {
        {
            let mut seen_events = self.seen_events.lock().expect("n8n seen_events poisoned");
            if !seen_events.insert(envelope.event_id.clone()) {
                self.record_dead_letter(&envelope, N8nIngestDecision::Duplicate);
                return N8nIngestDecision::Duplicate;
            }
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
        self.dead_letters
            .lock()
            .expect("n8n dead_letters poisoned")
            .push(N8nDeadLetter {
                reason,
                correlation_id: envelope.correlation_id.clone(),
                event_id: envelope.event_id.clone(),
                workflow_id: envelope.workflow_id.clone(),
                sequence_number: envelope.sequence_number,
            });
    }
}
