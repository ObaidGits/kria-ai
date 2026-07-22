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
            MemoryError::Storage(StorageError::Corruption(_)) | MemoryError::Recovery(_)
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
}
