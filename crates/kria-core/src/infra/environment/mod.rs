pub mod docker;
pub mod local;
pub mod remote_qemu;
pub mod traits;

pub use docker::{
    DockerEnvironment, DockerEnvironmentConfig, DockerNetworkPolicyMode, EnvironmentResetEvent,
    EnvironmentResetKind,
};
pub use local::LocalEnvironment;
pub use remote_qemu::{GuardrailSnapshot, QemuSshEnvironment, RemoteConfig, RemoteConfigValidationError};

pub use traits::{
    CommandExecutor, CommandRequest, CommandResult, EnvironmentError, EnvironmentLifecycle,
    EnvironmentProvider, FileSystemOps, ListDirRequest, ListDirResult, ReadFileRequest,
    ReadFileResult, ResetReason, ShellState, SharedShellState, WriteFileRequest, WriteFileResult,
};