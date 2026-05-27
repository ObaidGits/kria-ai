//! Workflow Session Persistence
//!
//! Provides long-horizon workflow support via:
//! - Checkpoint saving (persist workflow state to disk)
//! - Session continuation (resume interrupted workflows)
//! - Workflow memory (remember what was done across turns)
//! - Replay support (re-execute from a checkpoint)
//!
//! ## Architecture
//! Sessions are stored as JSON files in ~/.kria/sessions/.
//! Each session has a unique ID and contains:
//! - The original user intent
//! - Completed steps with their results
//! - Artifacts created
//! - Current state
//! - Continuation hints

use std::{collections::HashSet, path::PathBuf};
use tracing::debug;

/// A single completed step in a workflow session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionStep {
    /// Step number
    pub step: usize,
    /// Action that was executed
    pub action: String,
    /// Parameters used
    pub params: serde_json::Value,
    /// Whether the step succeeded
    pub success: bool,
    /// Evidence/result
    pub evidence: String,
    /// Timestamp (Unix seconds)
    pub timestamp: u64,
}

/// A workflow session checkpoint.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkflowSession {
    /// Unique session ID
    pub session_id: String,
    /// Original user intent
    pub user_intent: String,
    /// Substrate used
    pub substrate: String,
    /// Completed steps
    pub completed_steps: Vec<SessionStep>,
    /// Artifacts created (file paths)
    pub artifacts: Vec<String>,
    /// Whether the session is complete
    pub complete: bool,
    /// Error if the session failed
    pub error: Option<String>,
    /// Continuation hint for the next turn
    pub continuation_hint: Option<String>,
    /// Created timestamp
    pub created_at: u64,
    /// Last updated timestamp
    pub updated_at: u64,
}

impl WorkflowSession {
    /// Create a new session.
    pub fn new(session_id: String, user_intent: String, substrate: String) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            session_id,
            user_intent,
            substrate,
            completed_steps: Vec::new(),
            artifacts: Vec::new(),
            complete: false,
            error: None,
            continuation_hint: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Add a completed step.
    pub fn add_step(&mut self, step: SessionStep) {
        self.updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.completed_steps.push(step);
    }

    /// Mark the session as complete.
    pub fn mark_complete(&mut self, artifacts: Vec<String>) {
        self.complete = true;
        self.artifacts = artifacts;
        self.updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
    }

    /// Mark the session as failed.
    pub fn mark_failed(&mut self, error: String, continuation_hint: Option<String>) {
        self.error = Some(error);
        self.continuation_hint = continuation_hint;
        self.updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
    }

    /// Get a human-readable summary.
    pub fn summary(&self) -> String {
        if self.complete {
            format!(
                "Session '{}' completed: {} steps, {} artifacts",
                self.session_id,
                self.completed_steps.len(),
                self.artifacts.len()
            )
        } else if let Some(ref err) = self.error {
            format!(
                "Session '{}' failed at step {}: {}",
                self.session_id,
                self.completed_steps.len(),
                err
            )
        } else {
            format!(
                "Session '{}' in progress: {} steps completed",
                self.session_id,
                self.completed_steps.len()
            )
        }
    }
}

/// Session persistence manager.
pub struct SessionManager {
    sessions_dir: PathBuf,
}

impl SessionManager {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let sessions_dir = PathBuf::from(home).join(".kria").join("sessions");
        let _ = std::fs::create_dir_all(&sessions_dir);
        Self { sessions_dir }
    }

    /// Save a session checkpoint to disk atomically.
    ///
    /// Uses a write-to-temp-then-rename pattern to prevent partial writes
    /// from corrupting the session file on crash or power loss.
    pub fn save(&self, session: &WorkflowSession) -> Result<(), String> {
        let path = self
            .sessions_dir
            .join(format!("{}.json", session.session_id));
        let json = serde_json::to_string_pretty(session)
            .map_err(|e| format!("Serialization failed: {}", e))?;

        // Write to a temp file first, then atomically rename.
        // This prevents partial writes from corrupting the session file.
        let tmp_path = self
            .sessions_dir
            .join(format!("{}.json.tmp", session.session_id));
        std::fs::write(&tmp_path, &json).map_err(|e| format!("Temp write failed: {}", e))?;
        std::fs::rename(&tmp_path, &path).map_err(|e| {
            // Clean up temp file on rename failure
            let _ = std::fs::remove_file(&tmp_path);
            format!("Atomic rename failed: {}", e)
        })?;

        debug!(
            target: "session_manager",
            session_id = %session.session_id,
            path = %path.display(),
            "Session checkpoint saved atomically"
        );
        Ok(())
    }

    /// Load a session from disk.
    pub fn load(&self, session_id: &str) -> Option<WorkflowSession> {
        let path = self.sessions_dir.join(format!("{}.json", session_id));
        let json = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&json).ok()
    }

    /// List all sessions, most recent first. Capped at 100 to prevent OOM.
    pub fn list_sessions(&self) -> Vec<WorkflowSession> {
        let mut sessions = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.sessions_dir) {
            for entry in entries.flatten() {
                if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
                    // Skip temp files from atomic writes
                    if entry.path().to_string_lossy().ends_with(".tmp") {
                        continue;
                    }
                    if let Ok(json) = std::fs::read_to_string(entry.path()) {
                        if let Ok(session) = serde_json::from_str::<WorkflowSession>(&json) {
                            sessions.push(session);
                        }
                    }
                }
                // Cap at 200 entries to prevent OOM on large session directories
                if sessions.len() >= 200 {
                    break;
                }
            }
        }
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        // Return at most 100 sessions
        sessions.truncate(100);
        sessions
    }

    /// Find sessions that can be continued (failed with continuation hint).
    pub fn find_continuable(&self) -> Vec<WorkflowSession> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let max_age_secs = 48 * 3600;
        let mut seen = HashSet::new();

        self.list_sessions()
            .into_iter()
            .filter(|s| !s.complete && s.continuation_hint.is_some())
            .filter(|s| now.saturating_sub(s.updated_at) <= max_age_secs)
            .filter(|s| seen.insert(continuation_dedup_key(s)))
            .take(10)
            .collect()
    }

    /// Return the on-disk path for a session file.
    pub fn session_path(&self, session_id: &str) -> std::path::PathBuf {
        self.sessions_dir.join(format!("{}.json", session_id))
    }

    /// Delete a session.
    pub fn delete(&self, session_id: &str) {
        let path = self.sessions_dir.join(format!("{}.json", session_id));
        let _ = std::fs::remove_file(path);
    }

    /// Clean up sessions older than `max_age_hours`.
    pub fn cleanup_old_sessions(&self, max_age_hours: u64) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let max_age_secs = max_age_hours * 3600;

        for session in self.list_sessions() {
            if now.saturating_sub(session.updated_at) > max_age_secs {
                self.delete(&session.session_id);
                debug!(
                    target: "session_manager",
                    session_id = %session.session_id,
                    "Cleaned up old session"
                );
            }
        }
    }
}

fn continuation_dedup_key(session: &WorkflowSession) -> String {
    format!(
        "{}|{}|{}",
        normalize_for_dedup(&session.user_intent),
        normalize_for_dedup(&session.substrate),
        session
            .error
            .as_deref()
            .map(normalize_for_dedup)
            .unwrap_or_default()
    )
}

fn normalize_for_dedup(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .split_whitespace()
        .take(24)
        .collect::<Vec<_>>()
        .join(" ")
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}
