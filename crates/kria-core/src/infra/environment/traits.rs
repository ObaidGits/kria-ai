use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::Mutex;

/// RFC-001 FINAL (Section 3.2): Persisted shell state shared across tool calls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShellState {
    pub cwd: PathBuf,
    pub env_vars: HashMap<String, String>,
    pub generation: u64,
}

/// RFC-001 FINAL (Section 3.2): Single-lock shared shell state handle.
pub type SharedShellState = Arc<Mutex<ShellState>>;

/// RFC-001 FINAL (Section 7): Bounded command execution request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRequest {
    pub program: String,
    pub args: Vec<String>,
    pub timeout_ms: u64,
    pub max_bytes: usize,
    pub max_lines: usize,
}

/// RFC-001 FINAL (Section 3.3): Provider-agnostic command result payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

/// RFC-001 FINAL (Section 3.3): Provider-agnostic file read request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadFileRequest {
    pub path: PathBuf,
}

/// RFC-001 FINAL (Section 3.3): Provider-agnostic file read result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadFileResult {
    pub contents: Vec<u8>,
}

/// RFC-001 FINAL (Section 3.3): Provider-agnostic file write request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteFileRequest {
    pub path: PathBuf,
    pub contents: Vec<u8>,
    pub create_parent: bool,
}

/// RFC-001 FINAL (Section 3.3): Provider-agnostic file write result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteFileResult {
    pub bytes_written: usize,
}

/// RFC-001 FINAL (Section 3.3): Provider-agnostic directory listing request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListDirRequest {
    pub path: PathBuf,
}

/// RFC-001 FINAL (Section 3.3): Provider-agnostic directory listing result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListDirResult {
    pub entries: Vec<PathBuf>,
}

/// RFC-001 FINAL (Section 3.3): Environment lifecycle reset reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResetReason {
    Manual,
    Policy,
    ResourceExhaustion,
    RuntimeFailure,
    Other(String),
}

/// RFC-001 FINAL (Section 7): Canonical provider error taxonomy.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EnvironmentError {
    #[error("provider unavailable: {provider}: {details}")]
    ProviderUnavailable { provider: String, details: String },

    #[error("startup policy not ready: {policy}: {details}")]
    StartupPolicyNotReady { policy: String, details: String },

    #[error("network policy not ready: {mode}: {details}")]
    NetworkPolicyNotReady { mode: String, details: String },

    #[error("workspace isolation violation: {details}")]
    WorkspaceIsolationViolation { details: String },

    #[error("bind mount forbidden: {mount}")]
    BindMountForbidden { mount: String },

    #[error("path traversal denied: {path}")]
    PathTraversalDenied { path: String },

    #[error(
        "shell state conflict: expected generation {expected_generation}, actual generation {actual_generation}"
    )]
    ShellStateConflict {
        expected_generation: u64,
        actual_generation: u64,
    },

    #[error("command failed with exit code {exit_code}: {stderr}")]
    CommandFailed { exit_code: i32, stderr: String },

    #[error("command timed out after {timeout_ms}ms")]
    CommandTimedOut { timeout_ms: u64 },

    #[error("command cancelled")]
    CancellationRequested,

    #[error(
        "output limit exceeded (bytes {observed_bytes}/{max_bytes}, lines {observed_lines}/{max_lines})"
    )]
    OutputLimitExceeded {
        max_bytes: usize,
        max_lines: usize,
        observed_bytes: usize,
        observed_lines: usize,
    },

    #[error("storage limit exceeded: {observed_bytes}/{limit_bytes} bytes")]
    StorageLimitExceeded {
        limit_bytes: u64,
        observed_bytes: u64,
    },

    #[error("container OOM killed (exit code: {exit_code:?})")]
    ContainerOomKilled { exit_code: Option<i64> },

    #[error("pid limit exceeded: {limit}")]
    PidLimitExceeded { limit: i64 },

    #[error("environment reset required: {reason}")]
    EnvironmentResetRequired { reason: String },

    #[error("environment reset failed: {reason}: {details}")]
    EnvironmentResetFailed { reason: String, details: String },

    #[error("docker api error during {operation}: {details}")]
    DockerApi { operation: String, details: String },

    #[error("io error during {operation}: {details}")]
    Io { operation: String, details: String },

    #[error("serialization error: {details}")]
    Serialization { details: String },
}

/// RFC-001 FINAL (Section 3.3): Command execution capability.
#[async_trait]
pub trait CommandExecutor: Send + Sync {
    async fn execute_command(
        &self,
        request: CommandRequest,
        shell_state_snapshot: ShellState,
    ) -> Result<CommandResult, EnvironmentError>;
}

/// RFC-001 FINAL (Section 3.3): Filesystem capability.
#[async_trait]
pub trait FileSystemOps: Send + Sync {
    async fn read_file(&self, request: ReadFileRequest)
        -> Result<ReadFileResult, EnvironmentError>;

    async fn write_file(
        &self,
        request: WriteFileRequest,
    ) -> Result<WriteFileResult, EnvironmentError>;

    async fn list_dir(&self, request: ListDirRequest) -> Result<ListDirResult, EnvironmentError>;
}

/// RFC-001 FINAL (Section 3.3): Provider lifecycle capability.
#[async_trait]
pub trait EnvironmentLifecycle: Send + Sync {
    async fn ensure_ready(&self) -> Result<(), EnvironmentError>;

    async fn reset_environment(&self, reason: ResetReason) -> Result<(), EnvironmentError>;

    async fn shutdown(&self) -> Result<(), EnvironmentError>;
}

/// RFC-001 FINAL (Section 3.3): Composite provider capability surface.
pub trait EnvironmentProvider:
    CommandExecutor + FileSystemOps + EnvironmentLifecycle + Send + Sync
{
}

impl<T> EnvironmentProvider for T where
    T: CommandExecutor + FileSystemOps + EnvironmentLifecycle + Send + Sync
{
}
