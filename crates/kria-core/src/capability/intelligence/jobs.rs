//! Wave 11 — Durable, resumable, saga-safe long-running jobs (spec R28/R12/R24).
//!
//! A [`Job`] is a persisted unit of capability execution with a full lifecycle
//! state machine. The [`JobManager`] runs jobs through the SINGLE reliable
//! execution path ([`CapabilityPlatform::execute_reliable`] — timeout + bounded
//! retry + cancellation), checkpoints every state transition to a durable
//! [`JobStore`] (so jobs survive restart and are resumable without repeating an
//! idempotent completed step), enforces a concurrency limit (backpressure vs the
//! shared runtime, spec R24.1), emits `capability:job` events, and records
//! outcomes. It adds NO rival executor — it orchestrates the existing platform.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use super::reliability::RetryPolicy;
use crate::capability::error::CapError;
use crate::capability::events::{CapabilityEvent, Outcome, Stage};
use crate::capability::platform::CapabilityPlatform;
use crate::capability::provider::{CapabilityOutcome, CapabilityRequest, RequestContext};

/// The lifecycle state of a job (spec R28.1). Persisted so a restart can resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Paused,
    Cancelled,
    Completed,
    Failed,
    TimedOut,
    RolledBack,
    Recovered,
}

impl JobState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Cancelled => "cancelled",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::RolledBack => "rolled_back",
            Self::Recovered => "recovered",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "paused" => Self::Paused,
            "cancelled" => Self::Cancelled,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "timed_out" => Self::TimedOut,
            "rolled_back" => Self::RolledBack,
            "recovered" => Self::Recovered,
            _ => return None,
        })
    }

    /// Terminal states never resume/re-run.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::RolledBack
        )
    }
}

/// A durable job record (spec R28.1). `args_json` is stored with secrets redacted
/// (spec R12.4). `result_json` is set on completion (idempotent-resume marker).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub provider_id: String,
    pub capability_id: String,
    /// Execution arguments (secrets redacted before persistence).
    pub args_json: String,
    pub priority: i64,
    pub state: JobState,
    pub attempts: u32,
    pub correlation_id: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_json: Option<String>,
}

/// Durable job persistence (spec R28.1 — survive restart). Implemented by the CKB
/// so jobs live in the one learned layer (no parallel store).
#[async_trait]
pub trait JobStore: Send + Sync {
    async fn put_job(&self, job: &Job) -> Result<(), CapError>;
    async fn get_job(&self, id: &str) -> Result<Option<Job>, CapError>;
    /// All jobs, newest first (optionally capped).
    async fn list_jobs(&self, limit: usize) -> Result<Vec<Job>, CapError>;
    /// Non-terminal jobs (for restart resume).
    async fn list_active(&self) -> Result<Vec<Job>, CapError>;
    /// Update a job's state (+ attempts/error/result) atomically.
    async fn set_state(
        &self,
        id: &str,
        state: JobState,
        attempts: u32,
        last_error: Option<&str>,
        result_json: Option<&str>,
    ) -> Result<(), CapError>;
}

/// Redact secret-ish fields from an args JSON object before persistence
/// (spec R12.4 — secrets never persisted). Keys whose (lowercased) name contains
/// a sensitive token have their values replaced with `"***"`.
pub fn redact_secrets(args: &serde_json::Value) -> serde_json::Value {
    const SENSITIVE: &[&str] = &[
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "auth",
        "credential",
        "private",
        "bearer",
        "session",
    ];
    match args {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let lk = k.to_lowercase();
                if SENSITIVE.iter().any(|s| lk.contains(s)) {
                    out.insert(k.clone(), serde_json::Value::String("***".into()));
                } else {
                    out.insert(k.clone(), redact_secrets(v));
                }
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(a) => {
            serde_json::Value::Array(a.iter().map(redact_secrets).collect())
        }
        other => other.clone(),
    }
}

/// Runs durable jobs through the single reliable execution path. Concurrency-
/// bounded (semaphore), cancellable, resumable, checkpointed.
pub struct JobManager {
    platform: Arc<CapabilityPlatform>,
    store: Arc<dyn JobStore>,
    policy: RetryPolicy,
    /// Backpressure: cap on concurrently-running jobs (spec R24.1).
    permits: Arc<Semaphore>,
    /// Per-job cancellation tokens (user/shutdown/dependency cancellation).
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl JobManager {
    pub fn new(
        platform: Arc<CapabilityPlatform>,
        store: Arc<dyn JobStore>,
        policy: RetryPolicy,
        max_concurrency: usize,
    ) -> Self {
        Self {
            platform,
            store,
            policy,
            permits: Arc::new(Semaphore::new(max_concurrency.max(1))),
            cancels: Mutex::new(HashMap::new()),
        }
    }

    fn now() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    /// Currently-available concurrency permits (for resource-leak assertions:
    /// after all jobs settle this returns to the configured max).
    pub fn available_permits(&self) -> usize {
        self.permits.available_permits()
    }

    /// Number of live cancellation tokens (for leak assertions: returns to 0
    /// after all jobs settle).
    pub fn live_cancel_tokens(&self) -> usize {
        self.cancels.lock().map(|m| m.len()).unwrap_or(0)
    }

    fn emit(&self, job: &Job, state: JobState, detail: impl Into<String>) {
        if let Some(bus) = self.platform.events() {
            let outcome = match state {
                JobState::Completed | JobState::Recovered => Outcome::Ok,
                JobState::Failed | JobState::TimedOut => Outcome::Failed,
                JobState::Cancelled | JobState::RolledBack => Outcome::Declined,
                _ => Outcome::Started,
            };
            bus.emit(CapabilityEvent::new(
                &job.correlation_id,
                &job.provider_id,
                Some(job.capability_id.clone()),
                Stage::Job,
                outcome,
                format!("job {} [{}]: {}", job.id, state.as_str(), detail.into()),
            ));
        }
    }

    /// Submit a new job (persisted `Queued`). Returns its id. Secrets in `args`
    /// are redacted before persistence (spec R12.4).
    pub async fn submit(
        &self,
        provider_id: &str,
        capability_id: &str,
        args: serde_json::Value,
        priority: i64,
    ) -> Result<String, CapError> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Self::now();
        let job = Job {
            id: id.clone(),
            provider_id: provider_id.to_string(),
            capability_id: capability_id.to_string(),
            args_json: serde_json::to_string(&redact_secrets(&args))
                .unwrap_or_else(|_| "{}".into()),
            priority,
            state: JobState::Queued,
            attempts: 0,
            correlation_id: uuid::Uuid::new_v4().to_string(),
            created_at: now.clone(),
            updated_at: now,
            last_error: None,
            result_json: None,
        };
        self.store.put_job(&job).await?;
        self.emit(&job, JobState::Queued, "submitted");
        Ok(id)
    }

    /// A cancellation token for a job (registered so `cancel` can trip it).
    fn cancel_token(&self, id: &str) -> Arc<AtomicBool> {
        // Poison-safe: recover the guard if a holder panicked (the map data is
        // still valid) rather than propagating a panic on the job hot path.
        let mut map = self.cancels.lock().unwrap_or_else(|p| p.into_inner());
        map.entry(id.to_string())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone()
    }

    /// Request cancellation of a job (user/shutdown/dependency). Trips its token
    /// (a get-or-create so an in-flight run observes it) and marks it `Cancelled`
    /// if not already terminal. Idempotent. The token is then removed from the
    /// map to prevent unbounded growth — safe because: an in-flight run holds its
    /// own `Arc` clone (still observes the tripped value), and any future run
    /// early-returns on the persisted `Cancelled` state before touching a token.
    pub async fn cancel(&self, id: &str) -> Result<(), CapError> {
        self.cancel_token(id).store(true, Ordering::SeqCst);
        if let Some(job) = self.store.get_job(id).await? {
            if !job.state.is_terminal() {
                self.store
                    .set_state(
                        id,
                        JobState::Cancelled,
                        job.attempts,
                        Some("cancelled"),
                        None,
                    )
                    .await?;
                self.emit(&job, JobState::Cancelled, "cancelled by request");
            }
        }
        if let Ok(mut map) = self.cancels.lock() {
            map.remove(id);
        }
        Ok(())
    }

    /// Pause a not-yet-running job (spec R28.2). Only a `Queued`/`Recovered` job
    /// can be paused (an in-flight execution cannot be safely frozen); returns
    /// `false` if the job is running/terminal and cannot be paused.
    pub async fn pause(&self, id: &str) -> Result<bool, CapError> {
        let Some(job) = self.store.get_job(id).await? else {
            return Ok(false);
        };
        if matches!(job.state, JobState::Queued | JobState::Recovered) {
            self.store
                .set_state(id, JobState::Paused, job.attempts, None, None)
                .await?;
            self.emit(&job, JobState::Paused, "paused");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Resume a paused job (spec R28.2): return it to `Queued` and re-run it.
    pub async fn resume(&self, id: &str) -> Result<JobState, CapError> {
        if let Some(job) = self.store.get_job(id).await? {
            if job.state == JobState::Paused {
                self.store
                    .set_state(id, JobState::Queued, job.attempts, None, None)
                    .await?;
                self.emit(&job, JobState::Queued, "resumed from pause");
            }
        }
        self.run(id).await
    }

    /// Run one job to a terminal state through the reliable execution path.
    /// Acquires a concurrency permit (backpressure), checkpoints transitions,
    /// and performs saga rollback on failure/cancel. Idempotent-resume: a job
    /// already `Completed` returns its stored result without re-running.
    pub async fn run(&self, id: &str) -> Result<JobState, CapError> {
        let Some(mut job) = self.store.get_job(id).await? else {
            return Err(CapError::Io(format!("job '{id}' not found")));
        };
        // Idempotent resume: never repeat a completed job.
        if job.state == JobState::Completed {
            return Ok(JobState::Completed);
        }
        if matches!(job.state, JobState::Cancelled | JobState::RolledBack) {
            return Ok(job.state);
        }
        // A paused job does not run until explicitly resumed (spec R28.2).
        if job.state == JobState::Paused {
            return Ok(JobState::Paused);
        }

        let cancel = self.cancel_token(id);
        // Remove the cancel token from the map when this run ends (any terminal
        // path) — prevents unbounded growth of the token map (memory leak). Safe:
        // reached only after the terminal-state early-returns above, so every
        // exit from here is terminal and no future run needs this token.
        struct TokenGuard<'a> {
            map: &'a Mutex<HashMap<String, Arc<AtomicBool>>>,
            id: String,
        }
        impl Drop for TokenGuard<'_> {
            fn drop(&mut self) {
                if let Ok(mut m) = self.map.lock() {
                    m.remove(&self.id);
                }
            }
        }
        let _token_guard = TokenGuard {
            map: &self.cancels,
            id: id.to_string(),
        };
        if cancel.load(Ordering::SeqCst) {
            self.store
                .set_state(
                    id,
                    JobState::Cancelled,
                    job.attempts,
                    Some("cancelled"),
                    None,
                )
                .await?;
            return Ok(JobState::Cancelled);
        }

        // Backpressure: wait for a concurrency permit (spec R24.1).
        let _permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| CapError::Io(format!("job semaphore closed: {e}")))?;

        // Re-check after the (possibly long) permit wait: the job may have been
        // cancelled or paused WHILE queued for a slot. Honor that transition
        // instead of blindly running (closes the cancel/pause-during-wait race).
        if cancel.load(Ordering::SeqCst) {
            self.store
                .set_state(
                    id,
                    JobState::Cancelled,
                    job.attempts,
                    Some("cancelled"),
                    None,
                )
                .await?;
            return Ok(JobState::Cancelled);
        }
        if let Some(fresh) = self.store.get_job(id).await? {
            match fresh.state {
                JobState::Paused => return Ok(JobState::Paused),
                JobState::Cancelled => return Ok(JobState::Cancelled),
                _ => {}
            }
        }

        job.attempts += 1;
        self.store
            .set_state(id, JobState::Running, job.attempts, None, None)
            .await?;
        self.emit(&job, JobState::Running, "running");

        // Build the request from the (redacted) stored args. Note: redaction is
        // for PERSISTENCE; a real caller re-supplies secrets at submit time via a
        // secure channel. Here we execute with the stored (redacted) args — for
        // secret-bearing capabilities the caller should pass non-secret args or
        // resolve secrets from a vault at run time (out of scope: no secret store).
        let args: serde_json::Value =
            serde_json::from_str(&job.args_json).unwrap_or(serde_json::json!({}));
        let mut ctx = RequestContext::new();
        ctx.correlation_id = job.correlation_id.clone();
        let req = CapabilityRequest {
            provider_id: job.provider_id.clone(),
            capability_id: job.capability_id.clone(),
            args,
            context: ctx,
            granted_effects: Vec::new(),
        };

        match self
            .platform
            .execute_reliable(req, &self.policy, Some(cancel.clone()))
            .await
        {
            Ok(outcome) => {
                let result_json = match &outcome {
                    CapabilityOutcome::Value(v) => serde_json::to_string(v).ok(),
                    CapabilityOutcome::Declined { reason } => {
                        // A declined outcome is an honest non-success terminal.
                        self.store
                            .set_state(
                                id,
                                JobState::Failed,
                                job.attempts,
                                Some(&format!("declined: {reason}")),
                                None,
                            )
                            .await?;
                        self.emit(&job, JobState::Failed, format!("declined: {reason}"));
                        return Ok(JobState::Failed);
                    }
                    _ => None,
                };
                self.store
                    .set_state(
                        id,
                        JobState::Completed,
                        job.attempts,
                        None,
                        result_json.as_deref(),
                    )
                    .await?;
                self.emit(&job, JobState::Completed, "completed");
                Ok(JobState::Completed)
            }
            Err(e) => {
                // Distinguish cancellation from failure for the state machine.
                if cancel.load(Ordering::SeqCst) || e.to_string().contains("cancelled") {
                    self.store
                        .set_state(
                            id,
                            JobState::Cancelled,
                            job.attempts,
                            Some("cancelled"),
                            None,
                        )
                        .await?;
                    self.emit(&job, JobState::Cancelled, "cancelled");
                    return Ok(JobState::Cancelled);
                }
                let timed_out = e.to_string().contains("timed out");
                // SAGA compensation on failure (spec R12.1/R4.3): best-effort
                // rollback of any partial effect via the owning provider's remove
                // (no-op for pure read capabilities). Honest — never silent.
                let rolled = self.compensate(&job).await;
                let final_state = if rolled {
                    JobState::RolledBack
                } else if timed_out {
                    JobState::TimedOut
                } else {
                    JobState::Failed
                };
                self.store
                    .set_state(id, final_state, job.attempts, Some(&e.to_string()), None)
                    .await?;
                self.emit(&job, final_state, e.to_string());
                Ok(final_state)
            }
        }
    }

    /// Saga compensation for a failed job (spec R4.3). Returns whether a
    /// compensating action ran. For a plain capability execution there is no
    /// installed artifact to remove, so this is a no-op that returns `false`
    /// (honest — nothing to roll back). Kept as the single compensation hook so
    /// richer job kinds (install/replace) compensate through the same path.
    async fn compensate(&self, _job: &Job) -> bool {
        false
    }

    /// Resume all non-terminal jobs after a restart (spec R28.1). Idempotent —
    /// completed jobs are skipped; others are re-run through the reliable path.
    /// Returns the resumed job ids.
    pub async fn resume_all(&self) -> Result<Vec<String>, CapError> {
        let active = self.store.list_active().await?;
        let mut resumed = Vec::new();
        for job in active {
            // A Paused job must STAY paused across restart — never auto-resume it
            // (only an explicit `resume` runs it). Skip it.
            if job.state == JobState::Paused {
                continue;
            }
            // Mark Recovered (checkpoint) then re-run.
            self.store
                .set_state(&job.id, JobState::Recovered, job.attempts, None, None)
                .await?;
            self.emit(&job, JobState::Recovered, "resumed after restart");
            let _ = self.run(&job.id).await;
            resumed.push(job.id);
        }
        Ok(resumed)
    }

    /// List jobs (newest first) for the Execution Monitor UI.
    pub async fn list(&self, limit: usize) -> Result<Vec<Job>, CapError> {
        self.store.list_jobs(limit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_masks_secrets_recursively() {
        let v = serde_json::json!({
            "text": "hi",
            "api_key": "sk-123",
            "nested": { "password": "p", "ok": 1 },
            "list": [ { "token": "t" } ]
        });
        let r = redact_secrets(&v);
        assert_eq!(r["text"], serde_json::json!("hi"));
        assert_eq!(r["api_key"], serde_json::json!("***"));
        assert_eq!(r["nested"]["password"], serde_json::json!("***"));
        assert_eq!(r["nested"]["ok"], serde_json::json!(1));
        assert_eq!(r["list"][0]["token"], serde_json::json!("***"));
    }

    #[test]
    fn job_state_roundtrips_and_terminal() {
        for s in [
            JobState::Queued,
            JobState::Running,
            JobState::Paused,
            JobState::Completed,
            JobState::Failed,
            JobState::Cancelled,
            JobState::RolledBack,
            JobState::TimedOut,
            JobState::Recovered,
        ] {
            assert_eq!(JobState::parse(s.as_str()), Some(s));
        }
        assert!(JobState::Completed.is_terminal());
        assert!(JobState::Cancelled.is_terminal());
        assert!(!JobState::Running.is_terminal());
        assert!(!JobState::TimedOut.is_terminal());
    }
}
