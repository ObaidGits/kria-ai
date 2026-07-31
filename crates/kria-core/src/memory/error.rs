//! Canonical error taxonomy for the cognitive memory system (memory-upgrade
//! design §43). One hierarchy — errors are never invented ad hoc.
//!
//! Rules:
//! * `EmbeddingError::Unavailable`, `RetrievalError::StrategyDown`, and
//!   `SchedulerError::QueueFull` are **degradation signals** handled internally
//!   (L8), never surfaced as hard errors to the user.
//! * `StorageError::Corruption` and `RecoveryError::*` trigger the §30 recovery
//!   flow.
//! * A policy `Rejected` result is a normal outcome of `remember()`, not an
//!   `Err` — it lives on [`crate::memory::types::WriteDecision`], not here.
//! * Internal helpers may use `anyhow`, but convert to `MemoryError` at module
//!   boundaries.

use super::types::MemoryMode;

/// Crate-public result alias for the memory system.
pub type MemoryResult<T> = Result<T, MemoryError>;

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("memory feature is disabled")]
    Disabled,
    /// The system is in Recovery_Mode (design §5.3): durable writes are blocked
    /// until a verified restore succeeds. Callers should surface the
    /// `fault_class` and `correlation_id` to the user for recovery triage.
    #[error(
        "system is in Recovery_Mode (fault_class={fault_class}, correlation_id={correlation_id}): \
         durable writes are blocked until a verified restore succeeds"
    )]
    InRecoveryMode {
        /// Policy-safe fault classification (corruption class only, no content).
        fault_class: String,
        /// Stable correlation ID from the startup check that triggered this mode.
        correlation_id: String,
    },
    #[error("storage: {0}")]
    Storage(#[from] StorageError),
    #[error("retrieval: {0}")]
    Retrieval(#[from] RetrievalError),
    #[error("embedding: {0}")]
    Embedding(#[from] EmbeddingError),
    #[error("consistency: {0}")]
    Consistency(#[from] ConsistencyError),
    #[error("migration: {0}")]
    Migration(#[from] MigrationError),
    #[error("permission: {0}")]
    Permission(#[from] PermissionError),
    #[error("security: {0}")]
    Security(#[from] SecurityError),
    #[error("scheduler: {0}")]
    Scheduler(#[from] SchedulerError),
    #[error("recovery: {0}")]
    Recovery(#[from] RecoveryError),
    /// Escape hatch for genuinely unexpected internal errors crossing a
    /// boundary. Prefer a typed variant where one fits.
    #[error("internal: {0}")]
    Internal(String),
}

impl MemoryError {
    /// Whether this error is a graceful-degradation signal that the caller
    /// should treat as "feature unavailable" rather than a failure (L8).
    pub fn is_degradation(&self) -> bool {
        matches!(
            self,
            MemoryError::Embedding(EmbeddingError::Unavailable)
                | MemoryError::Retrieval(RetrievalError::StrategyDown(_))
                | MemoryError::Scheduler(SchedulerError::QueueFull)
        )
    }

    /// Whether this error should trigger the recovery/repair flow (design §30).
    pub fn needs_recovery(&self) -> bool {
        matches!(
            self,
            MemoryError::Storage(StorageError::Corruption(_))
                | MemoryError::Recovery(_)
                | MemoryError::InRecoveryMode { .. }
        )
    }
}

impl From<anyhow::Error> for MemoryError {
    fn from(e: anyhow::Error) -> Self {
        MemoryError::Internal(e.to_string())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("vector store: {0}")]
    Vector(String),
    #[error("search index: {0}")]
    Search(String),
    #[error("graph: {0}")]
    Graph(String),
    #[error("corruption detected: {0}")]
    Corruption(String),
    #[error("disk full")]
    DiskFull,
    #[error("busy/timeout")]
    Busy,
    #[error("authority is on a network filesystem (L14): {0}")]
    NetworkFilesystem(String),
    /// A connection-open pragma did not take effect (design §4/§4.4): the pragma
    /// was set but reading it back returned an unexpected value, meaning a
    /// durability or integrity invariant would be silently violated.
    #[error("pragma assertion failed: {0}")]
    PragmaAssertion(String),
    /// A required SQLite capability (e.g. JSON1) is not compiled into the linked
    /// library. The authority depends on it (design §4 uses `json_valid`).
    #[error("required sqlite capability missing: {0}")]
    CapabilityMissing(String),
    /// A value failed canonical-encoding validation at the authority write
    /// boundary (design §4: canonical time / UUID / boolean encodings).
    #[error("canonical encoding violation: {0}")]
    Encoding(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization: {0}")]
    Serde(String),
}

#[derive(Debug, thiserror::Error)]
pub enum RetrievalError {
    #[error("query classification failed")]
    Classify,
    #[error("strategy {0} unavailable")]
    StrategyDown(String),
    #[error("token budget exceeded")]
    Budget,
}

#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("embedding model unavailable")]
    Unavailable,
    #[error("dimension mismatch: expected {expected}, got {got}")]
    DimMismatch { expected: usize, got: usize },
    #[error("model checksum mismatch")]
    Checksum,
    #[error("model version mismatch: {0}")]
    Version(String),
    #[error("inference failed: {0}")]
    Inference(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ConsistencyError {
    #[error("outbox relay failed: {0}")]
    Relay(String),
    #[error("orphan detected: {0}")]
    Orphan(String),
    #[error("integrity check failed: {0}")]
    Integrity(String),
}

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("schema {found} is older than required {required}")]
    SchemaTooOld { found: u32, required: u32 },
    #[error("irreversible downgrade refused")]
    DowngradeRefused,
    #[error("re-embed batch failed at cursor {0}")]
    ReembedBatch(String),
    #[error("migration script error: {0}")]
    Script(String),
}

#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    #[error("namespace violation: {0}")]
    Namespace(String),
    #[error("mode forbids write: {0}")]
    Mode(MemoryMode),
    #[error("scope isolation violated: {0}")]
    Scope(String),
    #[error("consent required (not granted): {0}")]
    Consent(String),
}

#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("injection detected: {0}")]
    Injection(String),
    #[error("shred key destroyed")]
    Shredded,
    #[error("secret write refused")]
    SecretWrite,
    #[error("confirmation required")]
    NeedsConfirmation,
}

#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("job cancelled")]
    Cancelled,
    #[error("checkpoint io: {0}")]
    Checkpoint(String),
    #[error("queue full")]
    QueueFull,
    #[error("dead-lettered after {0} attempts")]
    DeadLetter(u32),
}

#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    #[error("backup checksum invalid")]
    BackupChecksum,
    #[error("no valid backup available")]
    NoBackup,
    #[error("rebuild failed: {0}")]
    Rebuild(String),
    /// Attempted to force-exit Recovery_Mode without a verified restore
    /// (design §5.3: "no RecoveryMode → Healthy transition is allowed without
    /// passing the startup checker").
    #[error(
        "cannot exit Recovery_Mode without a verified restore: \
         call recovery_restore() with a valid backup source"
    )]
    CannotExitWithoutVerifiedRestore,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degradation_signals_classified() {
        assert!(MemoryError::from(EmbeddingError::Unavailable).is_degradation());
        assert!(MemoryError::from(RetrievalError::StrategyDown("vector".into())).is_degradation());
        assert!(!MemoryError::from(StorageError::DiskFull).is_degradation());
    }

    #[test]
    fn corruption_triggers_recovery() {
        assert!(MemoryError::from(StorageError::Corruption("bad page".into())).needs_recovery());
        assert!(MemoryError::from(RecoveryError::NoBackup).needs_recovery());
        assert!(!MemoryError::from(EmbeddingError::Unavailable).needs_recovery());
    }

    #[test]
    fn in_recovery_mode_triggers_recovery() {
        let err = MemoryError::InRecoveryMode {
            fault_class: "sqlite_integrity_violation".to_string(),
            correlation_id: "corr-1".to_string(),
        };
        assert!(err.needs_recovery());
        assert!(!err.is_degradation());
        // Error message must include fault_class and correlation_id but not
        // protected content (verified by inspecting the format string only).
        let s = err.to_string();
        assert!(s.contains("Recovery_Mode"));
        assert!(s.contains("sqlite_integrity_violation"));
        assert!(s.contains("corr-1"));
    }
}
