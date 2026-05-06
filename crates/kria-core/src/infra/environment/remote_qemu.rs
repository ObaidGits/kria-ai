use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::fs::File;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use async_trait::async_trait;
use kria_connection_control::signer::{
    DEFAULT_DRIFT_BUFFER_MS, DualKeyHmacEnvelopeSigner, KeyMaterial, SignedEnvelope,
    SignedEnvelopeInput,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio::process::Command;
#[cfg(not(windows))]
use tokio::process::Child;
use tokio::runtime::Handle;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::KriaSystemConfig;
use crate::infra::qos::{AdaptiveQosScheduler, QosAdmission};
use crate::infra::snapshot::{
    SnapshotDriftTolerance, ensure_baseline_snapshot, try_fast_restore_latest_snapshot,
};

use super::traits::{
    CommandExecutor, CommandRequest, CommandResult, EnvironmentError, EnvironmentLifecycle,
    FileSystemOps, ListDirRequest, ListDirResult, ReadFileRequest, ReadFileResult, ResetReason,
    ShellState, WriteFileRequest, WriteFileResult,
};

const EMERGENCY_STATUS_BUFFER_MIN_BYTES: u64 = 512 * 1024;
const STAGING_HEARTBEAT_CHUNK_BYTES: usize = 64 * 1024;
const RESET_SPIN_SLEEP_MS: u64 = 10;
const INFRA_HIGH_STEPS_PER_MEDIUM_RECONNECT: u8 = 3;
const INFRA_RESET_RESERVED_NUMERATOR: usize = 3;
const INFRA_RESET_RESERVED_DENOMINATOR: usize = 10;

/// RFC-002 Section 5.1: Guest shell family semantics for helper invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestOsFamily {
    Posix,
    WindowsPowerShell,
    WindowsCmd,
}

/// RFC-002 Section 5.1: Host platform family for lifecycle and transport branching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPlatform {
    Linux,
    Windows,
    MacOs,
}

/// RFC-002 Section 5.1: Execution target topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    LocalQemuVm,
    PhysicalRemoteHost,
}

/// RFC-002 Section 5.1: Control-plane payload transport mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneTransport {
    EphemeralSftpFile,
    SshSendEnvSmallPayload,
}

/// RFC-002 Section 5.1: SSH transport backend strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshTransportBackend {
    OpenSshControlMaster,
    RustSshChannels,
}

/// RFC-002 Section 5.1: Privileged commit strategy for writes into protected paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegedCommitMode {
    Disabled,
    SudoMove,
    SudoHelperCommit,
}

/// RFC-002 Section 5.1: Helper provisioning policy and heartbeat envelopes.
#[derive(Debug, Clone)]
pub struct HelperProvisioning {
    pub required_helper_version: String,
    pub helper_manifest_path: PathBuf,
    pub helper_manifest_sig_path: PathBuf,
    pub helper_public_key_path: PathBuf,
    pub host_helper_cache_dir: PathBuf,
    pub remote_helper_dir: PathBuf,
    pub remote_helper_lock_dir: PathBuf,
    pub helper_lock_timeout_ms: u64,
    pub helper_lock_claim_retry_ms: u64,
    pub supervisor_heartbeat_interval_ms: u64,
    pub supervisor_heartbeat_timeout_ms: u64,
    pub worker_journal_silence_timeout_ms: u64,
    pub emergency_status_buffer_bytes: u64,
    pub last_gasp_packet_timeout_ms: u64,
    pub max_helper_rss_bytes: u64,
}

/// RFC-002 Section 5.1: OpenSSH/Rust SSH multiplexing policy.
#[derive(Debug, Clone)]
pub struct SshMultiplexingConfig {
    pub enable_control_master: bool,
    pub control_path_cmd: PathBuf,
    pub control_path_bulk: PathBuf,
    pub control_persist_secs: u64,
    pub establish_timeout_ms: u64,
    pub control_check_timeout_ms: u64,
    pub allow_no_mux_for_test: bool,
    pub rust_ssh_max_parallel_channels: u32,
}

/// RFC-002 Section 5.1: Split staging and privileged commit policy.
#[derive(Debug, Clone)]
pub struct FileCommitPolicy {
    pub remote_staging_dir: PathBuf,
    pub privileged_commit_mode: PrivilegedCommitMode,
    pub privileged_commit_helper_path: Option<PathBuf>,
    pub staging_sweep_ttl_secs: u64,
    pub staging_lease_heartbeat_timeout_ms: u64,
    pub staging_sweep_batch_limit: u32,
    pub enforce_linux_openat2: bool,
    pub privileged_probe_timeout_ms: u64,
    pub privileged_commit_timeout_ms: u64,
    pub disable_privileged_on_probe_failure: bool,
}

/// RFC-002 Section 5.1: Guest filesystem readiness and free-space policy.
#[derive(Debug, Clone)]
pub struct GuestFilesystemPolicy {
    pub require_control_dir_writable: bool,
    pub require_staging_dir_writable: bool,
    pub require_non_readonly_mount: bool,
    pub min_free_bytes_floor: u64,
}

/// RFC-002 Section 5.1 and 10.2: Reset barrier and network timeout policy.
#[derive(Debug, Clone)]
pub struct ResetPolicy {
    pub admission_freeze_timeout_ms: u64,
    pub zombie_reap_timeout_ms: u64,
    pub lock_acquire_timeout_ms: u64,
    pub network_call_timeout_ms: u64,
    pub total_reset_deadline_ms: u64,
}

/// RFC-002 Sections 5.1 and 11.4: Replay cache bounding policy.
#[derive(Debug, Clone)]
pub struct ReplayCachePolicy {
    pub retained_epoch_buckets: u8,
    pub max_nonces_per_epoch: usize,
}

/// RFC-002 Section 6.4: Infrastructure scheduling priority classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfrastructureTaskPriority {
    HighRecovery,
    MediumReconnect,
    LowMaintenance,
}

/// RFC-002 Sections 5.1 and 6.4: Dedicated infrastructure runtime queue policy.
#[derive(Debug, Clone)]
pub struct InfrastructureRuntimeConfig {
    pub infra_worker_threads: usize,
    pub high_priority_queue_capacity: usize,
    pub medium_priority_queue_capacity: usize,
    pub low_priority_queue_capacity: usize,
    pub infra_spawn_timeout_ms: u64,
}

/// RFC-002 Section 5.1 and 6.5: Host-side connection pooling and FD policy.
#[derive(Debug, Clone)]
pub struct SshPoolConfig {
    pub max_active_targets_hard_cap: u32,
    pub idle_ttl_secs: u64,
    pub sweep_interval_secs: u64,
    pub fd_soft_limit: u64,
    pub fd_reserve: u64,
    pub fd_per_command_budget: u64,
    pub fd_telemetry_sample_ms: u64,
}

/// RFC-002 Sections 5.1 and 6.3: Host artifact GC policy and process identity fingerprinting.
#[derive(Debug, Clone)]
pub struct HostArtifactGcConfig {
    pub enable_gc: bool,
    pub gc_ttl_secs: u64,
    pub state_root_dir: PathBuf,
    pub host_binary_sha256_or_build_id: String,
}

/// RFC-002 Section 3 runtime snapshot for guardrail validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuardrailSnapshot {
    pub available_space_bytes: u64,
    pub open_fds: u64,
}

/// RFC-002 Sections 3, 5.1, and 6.x: Top-level remote provider configuration.
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    pub host_platform: HostPlatform,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub ssh_key_path: PathBuf,
    pub guest_os_family: GuestOsFamily,
    pub target_kind: TargetKind,
    pub qemu_boot_cmd: Option<String>,
    pub qemu_pid_state_file: PathBuf,
    pub instance_id: String,
    pub remote_control_dir: PathBuf,
    pub transport_backend: SshTransportBackend,
    pub ssh_multiplexing: SshMultiplexingConfig,
    pub helper_provisioning: HelperProvisioning,
    pub control_transport: ControlPlaneTransport,
    pub envelope_ttl_ms: u64,
    pub max_command_payload_bytes: u64,
    pub file_commit_policy: FileCommitPolicy,
    pub guest_filesystem_policy: GuestFilesystemPolicy,
    pub reset_policy: ResetPolicy,
    pub replay_cache_policy: ReplayCachePolicy,
    pub ssh_pool: SshPoolConfig,
    pub host_artifact_gc: HostArtifactGcConfig,
    pub infrastructure_runtime: InfrastructureRuntimeConfig,
    pub ssh_connect_timeout_ms: u64,
    pub command_timeout_ms: u64,
    pub boot_wait_timeout_ms: u64,
    pub poll_interval_ms: u64,
    pub shutdown_timeout_ms: u64,
    pub soft_reset_grace_ms: u64,
    pub soft_reset_kill_timeout_ms: u64,
    pub max_soft_reset_attempts: u32,
    pub inflight_drain_timeout_ms: u64,
    pub local_cancel_kill_timeout_ms: u64,
    pub max_stdout_bytes: u64,
    pub max_stderr_bytes: u64,
    pub max_read_file_bytes: u64,
    pub command_timeout_requires_reset: bool,
    pub known_hosts_path: Option<PathBuf>,
    pub strict_host_key_checking: bool,
    pub pinned_host_key_sha256: Option<String>,
    pub remote_workspace_root: Option<PathBuf>,
}

/// RFC-002 Section 3 validation errors for configuration and runtime guardrails.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RemoteConfigValidationError {
    #[error("invalid replay cache retention: expected 2 buckets, got {actual}")]
    InvalidReplayRetention { actual: u8 },

    #[error("invalid max nonces per epoch: {actual} (must be > 0)")]
    InvalidMaxNoncesPerEpoch { actual: usize },

    #[error("invalid max command payload bytes: {actual} (must be > 0)")]
    InvalidMaxCommandPayloadBytes { actual: u64 },

    #[error("invalid fd soft limit: {actual} (must be > 0)")]
    InvalidFdSoftLimit { actual: u64 },

    #[error("invalid fd per-command budget: {actual} (must be > 0)")]
    InvalidFdPerCommandBudget { actual: u64 },

    #[error("disk guardrail overflow computing 2 * max_command_payload_bytes ({max_command_payload_bytes})")]
    DiskGuardrailOverflow { max_command_payload_bytes: u64 },

    #[error(
        "disk headroom violated: available={available_space_bytes} bytes must be strictly greater than required={required_strictly_greater_than} bytes"
    )]
    DiskHeadroomViolation {
        available_space_bytes: u64,
        required_strictly_greater_than: u64,
    },

    #[error("fd math overflow while computing open_fds + fd_reserve ({open_fds} + {fd_reserve})")]
    FdMathOverflow { open_fds: u64, fd_reserve: u64 },

    #[error(
        "fd admission violated: headroom={fd_headroom} is less than required fd_per_command_budget={fd_per_command_budget}"
    )]
    FdAdmissionViolation {
        fd_headroom: u64,
        fd_per_command_budget: u64,
    },
}

impl RemoteConfig {
    /// RFC-002 Sections 3 and 11.4: Validates static, non-runtime-dependent constraints.
    pub fn validate_static_contracts(&self) -> Result<(), RemoteConfigValidationError> {
        if self.replay_cache_policy.retained_epoch_buckets != 2 {
            return Err(RemoteConfigValidationError::InvalidReplayRetention {
                actual: self.replay_cache_policy.retained_epoch_buckets,
            });
        }

        if self.replay_cache_policy.max_nonces_per_epoch == 0 {
            return Err(RemoteConfigValidationError::InvalidMaxNoncesPerEpoch {
                actual: self.replay_cache_policy.max_nonces_per_epoch,
            });
        }

        if self.max_command_payload_bytes == 0 {
            return Err(RemoteConfigValidationError::InvalidMaxCommandPayloadBytes {
                actual: self.max_command_payload_bytes,
            });
        }

        if self.ssh_pool.fd_soft_limit == 0 {
            return Err(RemoteConfigValidationError::InvalidFdSoftLimit {
                actual: self.ssh_pool.fd_soft_limit,
            });
        }

        if self.ssh_pool.fd_per_command_budget == 0 {
            return Err(RemoteConfigValidationError::InvalidFdPerCommandBudget {
                actual: self.ssh_pool.fd_per_command_budget,
            });
        }

        Ok(())
    }

    /// RFC-002 Section 3: Enforces disk and FD guardrail inequalities against runtime telemetry.
    pub fn validate_config(
        &self,
        snapshot: GuardrailSnapshot,
    ) -> Result<(), RemoteConfigValidationError> {
        self.validate_static_contracts()?;

        let required_disk = self
            .max_command_payload_bytes
            .checked_mul(2)
            .ok_or(RemoteConfigValidationError::DiskGuardrailOverflow {
                max_command_payload_bytes: self.max_command_payload_bytes,
            })?;

        if snapshot.available_space_bytes <= required_disk {
            return Err(RemoteConfigValidationError::DiskHeadroomViolation {
                available_space_bytes: snapshot.available_space_bytes,
                required_strictly_greater_than: required_disk,
            });
        }

        let consumed_plus_reserve = snapshot
            .open_fds
            .checked_add(self.ssh_pool.fd_reserve)
            .ok_or(RemoteConfigValidationError::FdMathOverflow {
                open_fds: snapshot.open_fds,
                fd_reserve: self.ssh_pool.fd_reserve,
            })?;

        let fd_headroom = self
            .ssh_pool
            .fd_soft_limit
            .saturating_sub(consumed_plus_reserve);

        if fd_headroom < self.ssh_pool.fd_per_command_budget {
            return Err(RemoteConfigValidationError::FdAdmissionViolation {
                fd_headroom,
                fd_per_command_budget: self.ssh_pool.fd_per_command_budget,
            });
        }

        Ok(())
    }
}

/// RFC-002 Section 5.2: Guest capability snapshot discovered during readiness probes.
#[derive(Debug, Clone)]
pub struct GuestCapabilities {
    pub supports_sftp_batch: bool,
    pub supports_privileged_commit_helper: bool,
    pub supports_openat2_commit: bool,
    pub supports_process_tree_kill: bool,
    pub supports_atomic_rename: bool,
    pub helper_version: String,
}

/// RFC-002 Section 5.2: Parent process identity fence for watchdog validation.
#[derive(Debug, Clone)]
pub struct ParentIdentity {
    pub pid: u32,
    pub start_time_ticks: u64,
    pub session_nonce: String,
}

/// RFC-002 Sections 5.2 and 8.x: Staged artifact lease metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedArtifactLeaseMetadata {
    pub owner_instance_id: String,
    pub owner_pid: Option<u32>,
    pub owner_pid_start_time_ticks: Option<u64>,
    pub owner_binary_sha256_or_build_id: Option<String>,
    pub generation: u64,
    pub epoch_uuid: Uuid,
    pub artifact_nonce: String,
    pub created_unix_ms: u64,
    pub lease_heartbeat_unix_ms: u64,
    pub expected_sha256: String,
    pub bytes: u64,
}

/// RFC-002 Sections 5.2 and 11.4: Single-epoch nonce bucket with oldest-first eviction.
#[derive(Debug, Clone)]
pub struct NonceEpochBucket {
    pub epoch_uuid: Uuid,
    pub max_nonces: usize,
    pub insertion_order: VecDeque<String>,
    pub nonce_set: HashSet<String>,
}

impl NonceEpochBucket {
    pub fn new(epoch_uuid: Uuid, max_nonces: usize) -> Self {
        Self {
            epoch_uuid,
            max_nonces,
            insertion_order: VecDeque::new(),
            nonce_set: HashSet::new(),
        }
    }

    pub fn contains(&self, nonce: &str) -> bool {
        self.nonce_set.contains(nonce)
    }

    /// RFC-002 Section 11.4: Oldest-first eviction while enforcing hard max size.
    pub fn insert_oldest_first(&mut self, nonce: String) -> bool {
        if self.nonce_set.contains(&nonce) {
            return false;
        }

        self.nonce_set.insert(nonce.clone());
        self.insertion_order.push_back(nonce);

        while self.nonce_set.len() > self.max_nonces {
            if let Some(oldest) = self.insertion_order.pop_front() {
                self.nonce_set.remove(&oldest);
            } else {
                break;
            }
        }

        true
    }
}

/// RFC-002 Sections 5.2 and 11.4: Strictly bounded replay cache (current + previous epoch only).
#[derive(Debug, Clone)]
pub struct NonceReplayCache {
    pub current: NonceEpochBucket,
    pub previous: Option<NonceEpochBucket>,
}

/// RFC-002 Section 11.4: Replay cache insert result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonceRecordOutcome {
    Inserted,
    Duplicate,
}

impl NonceReplayCache {
    pub fn new(current_epoch: Uuid, max_nonces_per_epoch: usize) -> Self {
        Self {
            current: NonceEpochBucket::new(current_epoch, max_nonces_per_epoch),
            previous: None,
        }
    }

    pub fn contains(&self, epoch_uuid: Uuid, nonce: &str) -> bool {
        if self.current.epoch_uuid == epoch_uuid {
            return self.current.contains(nonce);
        }

        if let Some(previous) = &self.previous {
            return previous.epoch_uuid == epoch_uuid && previous.contains(nonce);
        }

        false
    }

    /// RFC-002 Sections 5.2 and 11.4: Records nonce with strict two-epoch retention and oldest-first eviction.
    pub fn record_nonce(&mut self, epoch_uuid: Uuid, nonce: String) -> NonceRecordOutcome {
        if self.current.epoch_uuid == epoch_uuid {
            return if self.current.insert_oldest_first(nonce) {
                NonceRecordOutcome::Inserted
            } else {
                NonceRecordOutcome::Duplicate
            };
        }

        if let Some(previous) = &mut self.previous {
            if previous.epoch_uuid == epoch_uuid {
                return if previous.insert_oldest_first(nonce) {
                    NonceRecordOutcome::Inserted
                } else {
                    NonceRecordOutcome::Duplicate
                };
            }
        }

        self.rotate_to_epoch(epoch_uuid);
        if self.current.insert_oldest_first(nonce) {
            NonceRecordOutcome::Inserted
        } else {
            NonceRecordOutcome::Duplicate
        }
    }

    /// RFC-002 Section 10.3 and 11.4: Rotation keeps only current + previous, dropping older epoch bucket.
    pub fn rotate_to_epoch(&mut self, new_epoch: Uuid) {
        if self.current.epoch_uuid == new_epoch {
            return;
        }

        let max_nonces = self.current.max_nonces;
        let old_current = std::mem::replace(&mut self.current, NonceEpochBucket::new(new_epoch, max_nonces));
        self.previous = Some(old_current);
    }
}

/// RFC-002 Section 5.2: Inflight command state for cancellation and cleanup barriers.
#[derive(Debug)]
pub struct InflightCommandHandle {
    pub command_id: String,
    pub generation: u64,
    pub epoch_uuid: Uuid,
    pub transport_generation_id: u64,
    pub cancel_token: CancellationToken,
    pub local_process_ids: Vec<u32>,
    pub remote_status_path: PathBuf,
    pub remote_tmp_paths: HashSet<PathBuf>,
    pub parent_identity: Option<ParentIdentity>,
    pub helper_supervisor_pid: Option<u32>,
    pub helper_worker_pid: Option<u32>,
    pub helper_worker_start_time_ticks: Option<u64>,
}

#[cfg(windows)]
#[derive(Debug)]
pub struct WindowsQemuProcess {
    process_handle: isize,
    job_handle: isize,
    pid: u32,
}

#[derive(Debug, Clone)]
struct ActiveLeaseSession {
    lease_id: Uuid,
    heartbeat_ttl: Duration,
    last_heartbeat_at: Instant,
    expires_at: Instant,
}

impl ActiveLeaseSession {
    fn new(lease_id: Uuid, heartbeat_ttl: Duration, now: Instant) -> Self {
        Self {
            lease_id,
            heartbeat_ttl,
            last_heartbeat_at: now,
            expires_at: now + heartbeat_ttl,
        }
    }

    fn renew(&mut self, now: Instant, heartbeat_ttl: Duration) {
        self.heartbeat_ttl = heartbeat_ttl;
        self.last_heartbeat_at = now;
        self.expires_at = now + heartbeat_ttl;
    }

    fn is_expired(&self, now: Instant) -> bool {
        now > self.expires_at
    }
}

#[derive(Debug, Clone)]
enum QmpEndpoint {
    #[cfg(unix)]
    Unix(PathBuf),
    Tcp { host: String, port: u16 },
}

/// RFC-002 Section 5.2: Provider state and runtime handles for remote execution lifecycle.
pub struct QemuSshEnvironment {
    pub config: RemoteConfig,
    #[cfg(not(windows))]
    pub qemu_child: Mutex<Option<Child>>,
    #[cfg(windows)]
    pub qemu_child: Mutex<Option<WindowsQemuProcess>>,
    pub provider_spawned_qemu: Mutex<bool>,
    pub guest_capabilities: Mutex<Option<GuestCapabilities>>,
    pub infra_runtime: Handle,
    pub agent_tool_runtime: Handle,
    pub generation: AtomicU64,
    pub epoch_uuid: ArcSwap<Uuid>,
    pub transport_generation_id: AtomicU64,
    active_lease: Mutex<Option<ActiveLeaseSession>>,
    pub tainted: AtomicBool,
    pub taint_reason: Mutex<Option<String>>,
    pub admissions_frozen: AtomicBool,
    pub admission_inflight: AtomicU64,
    pub zombie_commands: RwLock<HashSet<String>>,
    pub reset_in_progress: AtomicBool,
    pub inflight_registry: RwLock<HashMap<String, InflightCommandHandle>>,
    pub staged_artifact_index: RwLock<HashMap<String, HashMap<PathBuf, StagedArtifactLeaseMetadata>>>,
    pub nonce_replay_cache: RwLock<NonceReplayCache>,
    pub helper_seen_initializations: RwLock<HashSet<(String, Uuid)>>,
    pub helper_worker_stdout_stderr_local_logs: AtomicBool,
    envelope_signer: Arc<DualKeyHmacEnvelopeSigner>,
    signing_target_id: Uuid,
    signing_sequence: AtomicU64,
    infra_priority_counters: StdMutex<InfraPriorityCounters>,
    system_config: KriaSystemConfig,
    qos_scheduler: Arc<AdaptiveQosScheduler>,
}

impl QemuSshEnvironment {
    /// RFC-002 Sections 5.2 and 6.4: Constructs provider state with fresh cryptographic epoch and dual runtime handles.
    pub fn new(
        config: RemoteConfig,
        infra_runtime: Handle,
        agent_tool_runtime: Handle,
    ) -> Result<Self, RemoteConfigValidationError> {
        config.validate_static_contracts()?;

        let initial_epoch = Uuid::new_v4();
        let system_config = KriaSystemConfig::load(None);
        let nonce_replay_cache = NonceReplayCache::new(
            initial_epoch,
            config.replay_cache_policy.max_nonces_per_epoch,
        );
        let qos_scheduler = Arc::new(AdaptiveQosScheduler::new(&system_config));
        let envelope_signer = Arc::new(Self::build_dual_key_signer());
        let signing_target_id = Uuid::new_v4();

        Ok(Self {
            config,
            qemu_child: Mutex::new(None),
            provider_spawned_qemu: Mutex::new(false),
            guest_capabilities: Mutex::new(None),
            infra_runtime,
            agent_tool_runtime,
            generation: AtomicU64::new(0),
            epoch_uuid: ArcSwap::from_pointee(initial_epoch),
            transport_generation_id: AtomicU64::new(0),
            active_lease: Mutex::new(None),
            tainted: AtomicBool::new(false),
            taint_reason: Mutex::new(None),
            admissions_frozen: AtomicBool::new(false),
            admission_inflight: AtomicU64::new(0),
            zombie_commands: RwLock::new(HashSet::new()),
            reset_in_progress: AtomicBool::new(false),
            inflight_registry: RwLock::new(HashMap::new()),
            staged_artifact_index: RwLock::new(HashMap::new()),
            nonce_replay_cache: RwLock::new(nonce_replay_cache),
            helper_seen_initializations: RwLock::new(HashSet::new()),
            helper_worker_stdout_stderr_local_logs: AtomicBool::new(false),
            envelope_signer,
            signing_target_id,
            signing_sequence: AtomicU64::new(0),
            infra_priority_counters: StdMutex::new(InfraPriorityCounters::default()),
            system_config,
            qos_scheduler,
        })
    }

    /// RFC-002 Sections 3 and 5.2: Constructs provider after full runtime guardrail validation.
    pub fn new_with_guardrails(
        config: RemoteConfig,
        infra_runtime: Handle,
        agent_tool_runtime: Handle,
        snapshot: GuardrailSnapshot,
    ) -> Result<Self, RemoteConfigValidationError> {
        config.validate_config(snapshot)?;
        Self::new(config, infra_runtime, agent_tool_runtime)
    }

    fn current_epoch_uuid(&self) -> Uuid {
        *self.epoch_uuid.load_full().as_ref()
    }

    fn now_unix_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }

    fn build_dual_key_signer() -> DualKeyHmacEnvelopeSigner {
        let current_key = std::env::var("KRIA_FLEET_HMAC_KEY_CURRENT")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "kria-dev-primary-signing-key-change-me".to_string());
        let next_key = std::env::var("KRIA_FLEET_HMAC_KEY_NEXT")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "kria-dev-secondary-signing-key-change-me".to_string());

        DualKeyHmacEnvelopeSigner::new(
            KeyMaterial {
                key_id: "current".to_string(),
                secret: current_key.into_bytes(),
            },
            Some(KeyMaterial {
                key_id: "next".to_string(),
                secret: next_key.into_bytes(),
            }),
            Duration::from_secs(300),
        )
    }

    pub async fn activate_verified_lease(
        &self,
        lease_id: Uuid,
        heartbeat_ttl: Duration,
    ) -> Result<(), EnvironmentError> {
        if heartbeat_ttl.as_millis() == 0 {
            return Err(EnvironmentError::StartupPolicyNotReady {
                policy: "lease_heartbeat_ttl".to_string(),
                details: "heartbeat ttl must be > 0".to_string(),
            });
        }

        let mut lease_guard = self.active_lease.lock().await;
        *lease_guard = Some(ActiveLeaseSession::new(lease_id, heartbeat_ttl, Instant::now()));
        Ok(())
    }

    pub async fn renew_verified_lease(
        &self,
        lease_id: Uuid,
        heartbeat_ttl: Duration,
    ) -> Result<(), EnvironmentError> {
        if heartbeat_ttl.as_millis() == 0 {
            return Err(EnvironmentError::StartupPolicyNotReady {
                policy: "lease_heartbeat_ttl".to_string(),
                details: "heartbeat ttl must be > 0".to_string(),
            });
        }

        let now = Instant::now();
        let mut lease_guard = self.active_lease.lock().await;
        let Some(session) = lease_guard.as_mut() else {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: "lease renewal rejected: no active lease session".to_string(),
            });
        };

        if session.lease_id != lease_id {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!(
                    "lease renewal rejected: lease id mismatch active={} provided={}",
                    session.lease_id, lease_id
                ),
            });
        }

        if session.is_expired(now) {
            let expired_reason = format!(
                "active lease {} expired before renewal; fail-closed taint asserted",
                session.lease_id
            );
            *lease_guard = None;
            drop(lease_guard);

            self.tainted.store(true, Ordering::Release);
            *self.taint_reason.lock().await = Some(expired_reason.clone());
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: expired_reason,
            });
        }

        session.renew(now, heartbeat_ttl);
        Ok(())
    }

    pub async fn clear_verified_lease(&self) {
        self.active_lease.lock().await.take();
    }

    async fn require_active_verified_lease(&self) -> Result<Uuid, EnvironmentError> {
        let now = Instant::now();
        let mut lease_guard = self.active_lease.lock().await;
        let Some(session) = lease_guard.as_ref() else {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: "no active verified lease; command execution denied".to_string(),
            });
        };

        if session.is_expired(now) {
            let reason = format!(
                "active lease {} expired; command execution denied",
                session.lease_id
            );
            *lease_guard = None;
            drop(lease_guard);

            self.tainted.store(true, Ordering::Release);
            *self.taint_reason.lock().await = Some(reason.clone());
            return Err(EnvironmentError::EnvironmentResetRequired { reason });
        }

        Ok(session.lease_id)
    }

    pub async fn probe_transport_health(&self) -> Result<(), EnvironmentError> {
        let host = self.config.host.clone();
        let strict_host_key = self.config.strict_host_key_checking;
        let known_hosts_path = self.config.known_hosts_path.clone();
        let pinned_host_key = self.config.pinned_host_key_sha256.clone();

        self.run_infra_control_op("target_pool::health_gate_transport", async move {
            if host.trim().is_empty() {
                return Err(InfraExecutionError::Environment(
                    EnvironmentError::StartupPolicyNotReady {
                        policy: "remote_host".to_string(),
                        details: "host must not be empty".to_string(),
                    },
                ));
            }

            if strict_host_key && known_hosts_path.is_none() && pinned_host_key.is_none() {
                return Err(InfraExecutionError::Environment(
                    EnvironmentError::StartupPolicyNotReady {
                        policy: "strict_host_key_checking".to_string(),
                        details:
                            "strict host key mode requires known_hosts_path or pinned host key"
                                .to_string(),
                    },
                ));
            }

            Ok(())
        })
        .await
    }

    pub async fn probe_disk_headroom(&self) -> Result<(), EnvironmentError> {
        let control_dir = self.config.remote_control_dir.clone();
        let staging_dir = self.config.file_commit_policy.remote_staging_dir.clone();
        let min_free_bytes = self.config.guest_filesystem_policy.min_free_bytes_floor.max(1);

        self.run_infra_control_op("target_pool::health_gate_disk_headroom", async move {
            Self::check_disk_headroom(&control_dir, min_free_bytes)?;
            Self::check_disk_headroom(&staging_dir, min_free_bytes)?;
            Ok(())
        })
        .await
    }

    pub async fn probe_writeability(&self) -> Result<(), EnvironmentError> {
        let control_dir = self.config.remote_control_dir.clone();
        let staging_dir = self.config.file_commit_policy.remote_staging_dir.clone();

        self.run_infra_control_op("target_pool::health_gate_writeability", async move {
            for directory in [&control_dir, &staging_dir] {
                std::fs::create_dir_all(directory).map_err(|error| {
                    InfraExecutionError::Environment(EnvironmentError::Io {
                        operation: "probe_writeability::create_dir_all".to_string(),
                        details: format!("{} ({})", error, directory.display()),
                    })
                })?;

                let probe_file = directory.join(format!(".kria_writeability_probe_{}", Uuid::new_v4()));
                let mut file = std::fs::File::create(&probe_file).map_err(|error| {
                    InfraExecutionError::Environment(EnvironmentError::Io {
                        operation: "probe_writeability::create".to_string(),
                        details: format!("{} ({})", error, probe_file.display()),
                    })
                })?;

                file.write_all(b"probe").map_err(|error| {
                    InfraExecutionError::Environment(EnvironmentError::Io {
                        operation: "probe_writeability::write".to_string(),
                        details: format!("{} ({})", error, probe_file.display()),
                    })
                })?;

                file.sync_all().map_err(|error| {
                    InfraExecutionError::Environment(EnvironmentError::Io {
                        operation: "probe_writeability::sync_all".to_string(),
                        details: format!("{} ({})", error, probe_file.display()),
                    })
                })?;

                std::fs::remove_file(&probe_file).map_err(|error| {
                    InfraExecutionError::Environment(EnvironmentError::Io {
                        operation: "probe_writeability::remove_file".to_string(),
                        details: format!("{} ({})", error, probe_file.display()),
                    })
                })?;
            }

            Ok(())
        })
        .await
    }

    pub async fn probe_admission_barrier(&self) -> Result<(), EnvironmentError> {
        let admission_inflight = self.admission_inflight.load(Ordering::Acquire);
        let inflight_registry_len = self.inflight_registry.read().await.len();

        self.run_infra_control_op("target_pool::health_gate_admission_barrier", async move {
            if admission_inflight != 0 || inflight_registry_len != 0 {
                return Err(InfraExecutionError::Environment(
                    EnvironmentError::EnvironmentResetRequired {
                        reason: format!(
                            "admission barrier failed: admission_inflight={} inflight_registry_len={}",
                            admission_inflight, inflight_registry_len
                        ),
                    },
                ));
            }

            Ok(())
        })
        .await
    }

    pub async fn restore_snapshot_via_qmp(&self, snapshot_name: &str) -> Result<(), EnvironmentError> {
        if self.config.target_kind != TargetKind::LocalQemuVm {
            return Ok(());
        }

        let endpoint = self.resolve_qmp_endpoint()?;
        let snapshot = snapshot_name.to_string();

        self.run_infra_control_op("snapshot::qmp_restore", async move {
            Self::qmp_load_snapshot(endpoint, snapshot).await
        })
        .await
    }

    fn resolve_qmp_endpoint(&self) -> Result<QmpEndpoint, EnvironmentError> {
        if let Some(boot_cmd) = self.config.qemu_boot_cmd.as_ref() {
            let parts = boot_cmd.split_whitespace().collect::<Vec<_>>();
            for index in 0..parts.len().saturating_sub(1) {
                if parts[index] == "-qmp" {
                    if let Some(endpoint) = Self::parse_qmp_endpoint_spec(parts[index + 1]) {
                        return Ok(endpoint);
                    }
                }
            }
        }

        #[cfg(unix)]
        {
            return Ok(QmpEndpoint::Unix(
                self.config.remote_control_dir.join("qmp.sock"),
            ));
        }

        #[cfg(not(unix))]
        {
            Err(EnvironmentError::StartupPolicyNotReady {
                policy: "qmp_endpoint".to_string(),
                details: "unable to resolve qmp endpoint from qemu_boot_cmd".to_string(),
            })
        }
    }

    fn parse_qmp_endpoint_spec(spec: &str) -> Option<QmpEndpoint> {
        if let Some(unix_path) = spec.strip_prefix("unix:") {
            let path = unix_path.split(',').next().unwrap_or(unix_path);
            #[cfg(unix)]
            {
                return Some(QmpEndpoint::Unix(PathBuf::from(path)));
            }

            #[cfg(not(unix))]
            {
                let _ = path;
                return None;
            }
        }

        if let Some(tcp_spec) = spec.strip_prefix("tcp:") {
            let value = tcp_spec.split(',').next().unwrap_or(tcp_spec);
            if let Some((host, port)) = Self::parse_qmp_host_port(value) {
                return Some(QmpEndpoint::Tcp { host, port });
            }
        }

        if let Some((host, port)) = Self::parse_qmp_host_port(spec) {
            return Some(QmpEndpoint::Tcp { host, port });
        }

        None
    }

    fn parse_qmp_host_port(value: &str) -> Option<(String, u16)> {
        let (host, port) = value.rsplit_once(':')?;
        let parsed_port = port.parse::<u16>().ok()?;
        Some((host.to_string(), parsed_port))
    }

    async fn qmp_load_snapshot(
        endpoint: QmpEndpoint,
        snapshot_name: String,
    ) -> Result<(), InfraExecutionError> {
        match endpoint {
            #[cfg(unix)]
            QmpEndpoint::Unix(path) => {
                let stream = UnixStream::connect(&path).await.map_err(|error| {
                    Self::infra_io_error(
                        "snapshot::qmp_connect_unix",
                        format!("{} ({})", error, path.display()),
                    )
                })?;

                Self::qmp_handshake_and_load(stream, &snapshot_name).await
            }
            QmpEndpoint::Tcp { host, port } => {
                let address = format!("{}:{}", host, port);
                let stream = TcpStream::connect(address.clone()).await.map_err(|error| {
                    Self::infra_io_error(
                        "snapshot::qmp_connect_tcp",
                        format!("{} ({})", error, address),
                    )
                })?;

                Self::qmp_handshake_and_load(stream, &snapshot_name).await
            }
        }
    }

    async fn qmp_handshake_and_load<S>(
        stream: S,
        snapshot_name: &str,
    ) -> Result<(), InfraExecutionError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let (reader, mut writer) = tokio::io::split(stream);
        let mut reader = BufReader::new(reader);

        Self::qmp_wait_for_greeting(&mut reader).await?;
        Self::qmp_write_command(
            &mut writer,
            serde_json::json!({"execute": "qmp_capabilities"}),
            "snapshot::qmp_capabilities",
        )
        .await?;
        Self::qmp_wait_for_return(&mut reader, "snapshot::qmp_capabilities_return").await?;

        Self::qmp_write_command(
            &mut writer,
            serde_json::json!({
                "execute": "loadvm",
                "arguments": {
                    "name": snapshot_name,
                }
            }),
            "snapshot::qmp_loadvm",
        )
        .await?;
        Self::qmp_wait_for_return(&mut reader, "snapshot::qmp_loadvm_return").await?;

        Ok(())
    }

    async fn qmp_wait_for_greeting<R>(reader: &mut R) -> Result<(), InfraExecutionError>
    where
        R: AsyncBufRead + Unpin,
    {
        for _ in 0..32 {
            let line = Self::qmp_read_line(reader, "snapshot::qmp_wait_for_greeting").await?;
            if line.trim().is_empty() {
                continue;
            }

            let value: serde_json::Value =
                serde_json::from_str(line.trim()).map_err(|error| {
                    Self::infra_io_error(
                        "snapshot::qmp_parse_greeting",
                        format!("{} ({})", error, line.trim()),
                    )
                })?;

            if value.get("QMP").is_some() {
                return Ok(());
            }
        }

        Err(InfraExecutionError::Environment(
            EnvironmentError::EnvironmentResetRequired {
                reason: "qmp greeting not received before timeout".to_string(),
            },
        ))
    }

    async fn qmp_wait_for_return<R>(
        reader: &mut R,
        operation: &str,
    ) -> Result<(), InfraExecutionError>
    where
        R: AsyncBufRead + Unpin,
    {
        for _ in 0..64 {
            let line = Self::qmp_read_line(reader, operation).await?;
            if line.trim().is_empty() {
                continue;
            }

            let value: serde_json::Value = serde_json::from_str(line.trim()).map_err(|error| {
                Self::infra_io_error(operation, format!("{} ({})", error, line.trim()))
            })?;

            if let Some(error_payload) = value.get("error") {
                return Err(InfraExecutionError::Environment(
                    EnvironmentError::EnvironmentResetRequired {
                        reason: format!("qmp operation {} returned error: {}", operation, error_payload),
                    },
                ));
            }

            if value.get("return").is_some() {
                return Ok(());
            }
        }

        Err(InfraExecutionError::Environment(
            EnvironmentError::EnvironmentResetRequired {
                reason: format!("qmp operation {} timed out waiting for return", operation),
            },
        ))
    }

    async fn qmp_read_line<R>(
        reader: &mut R,
        operation: &str,
    ) -> Result<String, InfraExecutionError>
    where
        R: AsyncBufRead + Unpin,
    {
        let mut line = String::new();
        let read = tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
            .await
            .map_err(|_| {
                InfraExecutionError::Environment(EnvironmentError::EnvironmentResetRequired {
                    reason: format!("qmp operation {} timed out waiting for line", operation),
                })
            })?
            .map_err(|error| Self::infra_io_error(operation, error.to_string()))?;

        if read == 0 {
            return Err(InfraExecutionError::Environment(
                EnvironmentError::EnvironmentResetRequired {
                    reason: format!("qmp operation {} reached EOF", operation),
                },
            ));
        }

        Ok(line)
    }

    async fn qmp_write_command<W>(
        writer: &mut W,
        payload: serde_json::Value,
        operation: &str,
    ) -> Result<(), InfraExecutionError>
    where
        W: AsyncWrite + Unpin,
    {
        let mut bytes = serde_json::to_vec(&payload)
            .map_err(|error| Self::infra_io_error(operation, error.to_string()))?;
        bytes.push(b'\n');

        writer
            .write_all(&bytes)
            .await
            .map_err(|error| Self::infra_io_error(operation, error.to_string()))?;
        writer
            .flush()
            .await
            .map_err(|error| Self::infra_io_error(operation, error.to_string()))?;
        Ok(())
    }

    fn infra_io_error(operation: &str, details: String) -> InfraExecutionError {
        InfraExecutionError::Environment(EnvironmentError::Io {
            operation: operation.to_string(),
            details,
        })
    }

    fn check_disk_headroom(path: &Path, min_free_bytes: u64) -> Result<(), InfraExecutionError> {
        let disks = sysinfo::Disks::new_with_refreshed_list();
        let disk = disks
            .list()
            .iter()
            .filter(|disk| path.starts_with(disk.mount_point()))
            .max_by(|left, right| {
                left.mount_point()
                    .as_os_str()
                    .len()
                    .cmp(&right.mount_point().as_os_str().len())
            })
            .ok_or_else(|| {
                InfraExecutionError::Environment(EnvironmentError::EnvironmentResetRequired {
                    reason: format!(
                        "disk health gate failed: could not resolve mount for {}",
                        path.display()
                    ),
                })
            })?;

        let available = disk.available_space();
        if available < min_free_bytes {
            return Err(InfraExecutionError::Environment(
                EnvironmentError::EnvironmentResetRequired {
                    reason: format!(
                        "disk health gate failed: mount={} available={} required={}",
                        disk.mount_point().display(),
                        available,
                        min_free_bytes
                    ),
                },
            ));
        }

        Ok(())
    }

    fn parse_boot_command(boot_cmd: &str) -> Result<(OsString, Vec<OsString>), EnvironmentError> {
        let mut split = boot_cmd.split_whitespace();
        let program = split.next().ok_or_else(|| EnvironmentError::StartupPolicyNotReady {
            policy: "qemu_boot_cmd".to_string(),
            details: "qemu_boot_cmd is empty".to_string(),
        })?;

        let args = split.map(OsString::from).collect::<Vec<_>>();
        Ok((OsString::from(program), args))
    }

    fn process_start_time_ticks(pid: u32) -> Result<u64, EnvironmentError> {
        #[cfg(target_os = "linux")]
        {
            let stat_path = format!("/proc/{pid}/stat");
            let stat_contents = std::fs::read_to_string(&stat_path).map_err(|error| {
                EnvironmentError::Io {
                    operation: "process_start_time_ticks::read_proc_stat".to_string(),
                    details: format!("{} ({})", error, stat_path),
                }
            })?;

            let command_end = stat_contents.rfind(')').ok_or_else(|| EnvironmentError::Io {
                operation: "process_start_time_ticks::parse_proc_stat".to_string(),
                details: format!("malformed proc stat for pid {pid}"),
            })?;

            let tail = stat_contents
                .get(command_end + 2..)
                .ok_or_else(|| EnvironmentError::Io {
                    operation: "process_start_time_ticks::slice_proc_stat".to_string(),
                    details: format!("failed to slice proc stat tail for pid {pid}"),
                })?;

            let fields = tail.split_whitespace().collect::<Vec<_>>();
            let start_time = fields
                .get(19)
                .ok_or_else(|| EnvironmentError::Io {
                    operation: "process_start_time_ticks::field_proc_stat".to_string(),
                    details: format!("missing start_time field for pid {pid}"),
                })?
                .parse::<u64>()
                .map_err(|error| EnvironmentError::Io {
                    operation: "process_start_time_ticks::parse_start_time".to_string(),
                    details: format!("{} (pid {pid})", error),
                })?;

            return Ok(start_time);
        }

        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::Foundation::FILETIME;
            use windows_sys::Win32::System::Threading::{
                GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            };

            let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            if handle == 0 {
                return Err(EnvironmentError::Io {
                    operation: "process_start_time_ticks::open_process".to_string(),
                    details: format!("failed to open process pid={pid}"),
                });
            }

            let mut created = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let mut exited = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let mut kernel = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let mut user = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };

            let ok = unsafe {
                GetProcessTimes(
                    handle,
                    &mut created,
                    &mut exited,
                    &mut kernel,
                    &mut user,
                )
            };
            unsafe {
                CloseHandle(handle);
            }

            if ok == 0 {
                return Err(EnvironmentError::Io {
                    operation: "process_start_time_ticks::get_process_times".to_string(),
                    details: format!("failed to read creation time pid={pid}"),
                });
            }

            let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
            return Ok(ticks);
        }

        #[cfg(not(any(target_os = "linux", windows)))]
        {
            let _ = pid;
            Err(EnvironmentError::ProviderUnavailable {
                provider: "remote_qemu".to_string(),
                details: "process start-time ticks not implemented on this platform".to_string(),
            })
        }
    }

    fn capture_parent_identity(&self) -> Result<ParentIdentity, EnvironmentError> {
        let pid = std::process::id();
        let start_time_ticks = Self::process_start_time_ticks(pid)?;
        let session_nonce = Uuid::new_v4().to_string();

        Ok(ParentIdentity {
            pid,
            start_time_ticks,
            session_nonce,
        })
    }

    fn command_sha256(program: &str, args: &[String]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(program.as_bytes());
        hasher.update([0_u8]);
        for arg in args {
            hasher.update(arg.as_bytes());
            hasher.update([0_u8]);
        }
        hex::encode(hasher.finalize())
    }

    fn build_execution_envelope(
        &self,
        request: &CommandRequest,
        shell_state_snapshot: &ShellState,
        command_id: String,
        generation: u64,
        epoch_uuid: Uuid,
        transport_generation_id: u64,
        parent_identity: &ParentIdentity,
        nonce: String,
    ) -> ExecutionEnvelope {
        let env = shell_state_snapshot
            .env_vars
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();

        let cwd = if shell_state_snapshot.cwd.as_os_str().is_empty() {
            ".".to_string()
        } else {
            shell_state_snapshot.cwd.to_string_lossy().to_string()
        };

        ExecutionEnvelope {
            command_id,
            generation,
            epoch_uuid: *epoch_uuid.as_bytes(),
            transport_generation_id,
            instance_id: self.config.instance_id.clone(),
            issued_at_host_unix_ms_info_only: Self::now_unix_ms(),
            ttl_ms_from_receipt: self.config.envelope_ttl_ms,
            nonce,
            parent_session_nonce: parent_identity.session_nonce.clone(),
            parent_ssh_session_pid: Some(parent_identity.pid),
            parent_ssh_session_start_time_ticks: Some(parent_identity.start_time_ticks),
            cwd,
            env,
            program: request.program.clone(),
            args: request.args.clone(),
            command_sha256: Self::command_sha256(&request.program, &request.args),
            stdin_mode: "none".to_string(),
        }
    }

    async fn sign_execution_envelope(
        &self,
        envelope: &ExecutionEnvelope,
        lease_id: Uuid,
    ) -> Result<SignedExecutionEnvelope, EnvironmentError> {
        let payload = serde_json::to_value(envelope).map_err(|error| EnvironmentError::Serialization {
            details: format!("failed to serialize execution envelope: {error}"),
        })?;
        let sequence_id = self.signing_sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let control_envelope = self
            .envelope_signer
            .sign(SignedEnvelopeInput {
                target_id: self.signing_target_id,
                lease_id,
                sequence: sequence_id,
                nonce: envelope.nonce.clone(),
                ttl: Duration::from_millis(envelope.ttl_ms_from_receipt.max(1)),
                drift_buffer_ms: DEFAULT_DRIFT_BUFFER_MS,
                op: "remote_qemu.execute".to_string(),
                payload,
            })
            .await
            .map_err(|error| EnvironmentError::Serialization {
                details: format!(
                    "failed to sign execution envelope with DualKeyHmacEnvelopeSigner: {error}"
                ),
            })?;

        Ok(SignedExecutionEnvelope {
            envelope: envelope.clone(),
            control_envelope,
        })
    }

    fn validate_parent_identity_triple(expected: &ParentIdentity, observed: &ParentIdentity) -> bool {
        expected.pid == observed.pid
            && expected.start_time_ticks == observed.start_time_ticks
            && expected.session_nonce == observed.session_nonce
    }

    async fn helper_accepts_initialize(
        &self,
        command_id: &str,
        epoch_uuid: Uuid,
    ) -> Result<(), EnvironmentError> {
        let mut seen = self.helper_seen_initializations.write().await;
        if !seen.insert((command_id.to_string(), epoch_uuid)) {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!(
                    "duplicate initialize rejected for command_id={} epoch_uuid={}",
                    command_id, epoch_uuid
                ),
            });
        }

        Ok(())
    }

    fn bump_transport_generation_id(&self, reason: &str) {
        let bumped = self.transport_generation_id.fetch_add(1, Ordering::AcqRel) + 1;
        self.tainted.store(true, Ordering::Release);
        let reason_message = format!(
            "transport generation bumped to {} after socket failure: {}",
            bumped, reason
        );
        if let Ok(mut lock) = self.taint_reason.try_lock() {
            *lock = Some(reason_message);
        }
    }

    async fn run_infra_control_op<T, F>(
        &self,
        operation: &'static str,
        fut: F,
    ) -> Result<T, EnvironmentError>
    where
        T: Send + 'static,
        F: std::future::Future<Output = Result<T, InfraExecutionError>> + Send + 'static,
    {
        let qos_class = AdaptiveQosScheduler::classify_operation(operation);
        let qos_started = Instant::now();
        let admitted = match self.qos_scheduler.try_start_task(qos_class, operation) {
            QosAdmission::Accepted => true,
            QosAdmission::Deferred {
                retry_after,
                reason: _,
            } => {
                tokio::time::sleep(retry_after).await;
                match self.qos_scheduler.try_start_task(qos_class, operation) {
                    QosAdmission::Accepted => true,
                    QosAdmission::Deferred { reason, .. } | QosAdmission::Rejected { reason } => {
                        return Err(EnvironmentError::EnvironmentResetRequired {
                            reason: format!(
                                "qos deferred/rejected operation {} after retry: {}",
                                operation, reason
                            ),
                        });
                    }
                }
            }
            QosAdmission::Rejected { reason } => {
                return Err(EnvironmentError::EnvironmentResetRequired {
                    reason: format!("qos rejected operation {}: {}", operation, reason),
                });
            }
        };

        let _slot_guard = match self.acquire_infra_slot(operation) {
            Ok(guard) => guard,
            Err(error) => {
                if admitted {
                    self.qos_scheduler
                        .finish_task(qos_class, qos_started.elapsed().as_millis() as u64, false);
                }
                return Err(error);
            }
        };
        let timeout = Duration::from_millis(self.config.reset_policy.network_call_timeout_ms.max(1));
        let join = self.infra_runtime.spawn(fut);
        let joined = tokio::time::timeout(timeout, join).await;

        let result = match joined {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(InfraExecutionError::Environment(error)))) => Err(error),
            Ok(Ok(Err(InfraExecutionError::SocketFailure(details)))) => {
                self.bump_transport_generation_id(&details);
                Err(EnvironmentError::EnvironmentResetRequired {
                    reason: format!("transport socket failure during {}: {}", operation, details),
                })
            }
            Ok(Err(join_error)) => {
                self.bump_transport_generation_id(&join_error.to_string());
                Err(EnvironmentError::EnvironmentResetRequired {
                    reason: format!(
                        "infra runtime join failure during {}: {}",
                        operation, join_error
                    ),
                })
            }
            Err(_) => {
                self.bump_transport_generation_id("network call timeout");
                Err(EnvironmentError::EnvironmentResetRequired {
                    reason: format!(
                        "infra runtime timeout during {} after {}ms",
                        operation,
                        timeout.as_millis()
                    ),
                })
            }
        };

        if admitted {
            self.qos_scheduler.finish_task(
                qos_class,
                qos_started.elapsed().as_millis() as u64,
                result.is_ok(),
            );
        }

        result
    }

    async fn run_control_health_check(&self) -> Result<(), EnvironmentError> {
        let host = self.config.host.clone();
        self.run_infra_control_op("ensure_ready::control_health_check", async move {
            if host.trim().is_empty() {
                return Err(InfraExecutionError::Environment(
                    EnvironmentError::StartupPolicyNotReady {
                        policy: "remote_host".to_string(),
                        details: "host must not be empty".to_string(),
                    },
                ));
            }
            Ok(())
        })
        .await
    }

    async fn enforce_helper_output_pipe_safety(&self) -> Result<(), EnvironmentError> {
        self.run_infra_control_op("ensure_ready::helper_output_pipe_safety", async {
            // This control operation intentionally runs on infra runtime to preserve
            // the invariant that helper-control commands never execute on agent pools.
            Ok(())
        })
        .await?;

        self.helper_worker_stdout_stderr_local_logs
            .store(true, Ordering::Release);
        Ok(())
    }

    async fn probe_guest_capabilities(&self) -> Result<(), EnvironmentError> {
        let helper_version = self.config.helper_provisioning.required_helper_version.clone();
        let capabilities = self
            .run_infra_control_op("ensure_ready::probe_guest_capabilities", async move {
                Ok(GuestCapabilities {
                    supports_sftp_batch: true,
                    supports_privileged_commit_helper: true,
                    supports_openat2_commit: cfg!(target_os = "linux"),
                    supports_process_tree_kill: true,
                    supports_atomic_rename: true,
                    helper_version,
                })
            })
            .await?;

        let mut guard = self.guest_capabilities.lock().await;
        *guard = Some(capabilities);
        Ok(())
    }

    async fn persist_qemu_pid_state(&self, pid: u32) -> Result<(), EnvironmentError> {
        let pid_path = self.config.qemu_pid_state_file.clone();
        self.run_infra_control_op("ensure_ready::persist_qemu_pid_state", async move {
            let mut file = File::create(&pid_path).map_err(|error| {
                InfraExecutionError::Environment(EnvironmentError::Io {
                    operation: "persist_qemu_pid_state::create".to_string(),
                    details: format!("{} ({})", error, pid_path.display()),
                })
            })?;

            writeln!(file, "{pid}").map_err(|error| {
                InfraExecutionError::Environment(EnvironmentError::Io {
                    operation: "persist_qemu_pid_state::write".to_string(),
                    details: format!("{} ({})", error, pid_path.display()),
                })
            })?;

            file.sync_all().map_err(|error| {
                InfraExecutionError::Environment(EnvironmentError::Io {
                    operation: "persist_qemu_pid_state::sync_all".to_string(),
                    details: format!("{} ({})", error, pid_path.display()),
                })
            })?;

            Ok(())
        })
        .await
    }

    #[cfg(target_os = "linux")]
    fn spawn_qemu_linux_pre_exec(qemu_boot_cmd: &str) -> Result<Child, InfraExecutionError> {
        let (program, args) = Self::parse_boot_command(qemu_boot_cmd).map_err(InfraExecutionError::Environment)?;
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0);
                Ok(())
            });
        }

        command.spawn().map_err(|error| {
            InfraExecutionError::Environment(EnvironmentError::Io {
                operation: "spawn_qemu_linux_pre_exec::spawn".to_string(),
                details: error.to_string(),
            })
        })
    }

    #[cfg(windows)]
    fn spawn_qemu_windows_raw(qemu_boot_cmd: &str) -> Result<WindowsQemuProcess, InfraExecutionError> {
        windows_spawn::spawn_qemu_windows_raw(qemu_boot_cmd)
    }

    async fn ensure_qemu_process(&self) -> Result<(), EnvironmentError> {
        if self.config.target_kind != TargetKind::LocalQemuVm {
            return Ok(());
        }

        let qemu_boot_cmd = self
            .config
            .qemu_boot_cmd
            .clone()
            .ok_or_else(|| EnvironmentError::StartupPolicyNotReady {
                policy: "qemu_boot_cmd".to_string(),
                details: "LocalQemuVm requires qemu_boot_cmd".to_string(),
            })?;

        #[cfg(target_os = "linux")]
        {
            let mut child_guard = self.qemu_child.lock().await;
            if child_guard.is_none() {
                let child = self
                    .run_infra_control_op("ensure_ready::spawn_qemu_linux", async move {
                        Self::spawn_qemu_linux_pre_exec(&qemu_boot_cmd)
                    })
                    .await?;

                let pid = child.id().ok_or_else(|| EnvironmentError::Io {
                    operation: "ensure_ready::spawn_qemu_linux".to_string(),
                    details: "spawned QEMU child missing pid".to_string(),
                })?;

                self.persist_qemu_pid_state(pid).await?;
                *child_guard = Some(child);
                *self.provider_spawned_qemu.lock().await = true;
            }
            return Ok(());
        }

        #[cfg(windows)]
        {
            let mut child_guard = self.qemu_child.lock().await;
            if child_guard.is_none() {
                if !qemu_boot_cmd.contains("-display none") && !qemu_boot_cmd.contains("-vnc") {
                    return Err(EnvironmentError::StartupPolicyNotReady {
                        policy: "windows_session0_qemu_display_policy".to_string(),
                        details:
                            "qemu_boot_cmd must include '-display none' or '-vnc :0' on Windows"
                                .to_string(),
                    });
                }

                let process = self
                    .run_infra_control_op("ensure_ready::spawn_qemu_windows", async move {
                        Self::spawn_qemu_windows_raw(&qemu_boot_cmd)
                    })
                    .await?;

                self.persist_qemu_pid_state(process.pid).await?;
                *child_guard = Some(process);
                *self.provider_spawned_qemu.lock().await = true;
            }
            return Ok(());
        }

        #[cfg(not(any(target_os = "linux", windows)))]
        {
            let _ = qemu_boot_cmd;
            Err(EnvironmentError::ProviderUnavailable {
                provider: "remote_qemu".to_string(),
                details:
                    "LocalQemuVm lifecycle binding is currently implemented only on Linux/Windows"
                        .to_string(),
            })
        }
    }

    async fn run_local_command(
        request: CommandRequest,
        shell_state_snapshot: ShellState,
        command_id: String,
        control_dir: PathBuf,
        redirect_output_to_local_logs: bool,
    ) -> Result<(i32, String, String), InfraExecutionError> {
        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .envs(shell_state_snapshot.env_vars);

        if !shell_state_snapshot.cwd.as_os_str().is_empty() {
            command.current_dir(shell_state_snapshot.cwd);
        }

        if redirect_output_to_local_logs {
            let logs_dir = control_dir.join("helper_worker_logs");
            let mut stdout_path = logs_dir.join(format!("{}.stdout.log", command_id));
            let mut stderr_path = logs_dir.join(format!("{}.stderr.log", command_id));

            let mut stdout_redirected_to_file = false;
            let mut stderr_redirected_to_file = false;

            if std::fs::create_dir_all(&logs_dir).is_ok() {
                match File::create(&stdout_path) {
                    Ok(file) => {
                        command.stdout(Stdio::from(file));
                        stdout_redirected_to_file = true;
                    }
                    Err(_) => {
                        command.stdout(Stdio::null());
                    }
                }

                match File::create(&stderr_path) {
                    Ok(file) => {
                        command.stderr(Stdio::from(file));
                        stderr_redirected_to_file = true;
                    }
                    Err(_) => {
                        command.stderr(Stdio::null());
                    }
                }
            } else {
                command.stdout(Stdio::null());
                command.stderr(Stdio::null());
                stdout_path = PathBuf::new();
                stderr_path = PathBuf::new();
            }

            let status = command.status().await.map_err(|error| {
                InfraExecutionError::Environment(EnvironmentError::Io {
                    operation: "remote_qemu::run_local_command::spawn".to_string(),
                    details: error.to_string(),
                })
            })?;

            let stdout = if stdout_redirected_to_file {
                std::fs::read(&stdout_path)
                    .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let stderr = if stderr_redirected_to_file {
                std::fs::read(&stderr_path)
                    .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };

            return Ok((status.code().unwrap_or(-1), stdout, stderr));
        }

        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let output = command.output().await.map_err(|error| {
            InfraExecutionError::Environment(EnvironmentError::Io {
                operation: "remote_qemu::run_local_command::spawn".to_string(),
                details: error.to_string(),
            })
        })?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Ok((exit_code, stdout, stderr))
    }

    async fn execute_over_control_channel(
        &self,
        signed_envelope: SignedExecutionEnvelope,
        request: CommandRequest,
        shell_state_snapshot: ShellState,
    ) -> Result<HelperExecutionEvidence, EnvironmentError> {
        let operation_request = request.clone();
        let command_id_for_logs = signed_envelope.envelope.command_id.clone();
        let control_dir = self.config.remote_control_dir.clone();
        let redirect_output_to_local_logs = self
            .helper_worker_stdout_stderr_local_logs
            .load(Ordering::Acquire);
        self.run_infra_control_op("execute_command::control_channel", async move {
            if operation_request.program == "__simulate_transport_socket_failure__" {
                return Err(InfraExecutionError::SocketFailure(
                    "simulated stale control socket failure".to_string(),
                ));
            }

            let timeout = Duration::from_millis(operation_request.timeout_ms.max(1));
            let command_result = tokio::time::timeout(
                timeout,
                Self::run_local_command(
                    operation_request.clone(),
                    shell_state_snapshot,
                    command_id_for_logs,
                    control_dir,
                    redirect_output_to_local_logs,
                ),
            )
            .await
            .map_err(|_| {
                InfraExecutionError::Environment(EnvironmentError::CommandTimedOut {
                    timeout_ms: operation_request.timeout_ms,
                })
            })??;

            let (exit_code, stdout, stderr) = command_result;
            let journal_complete = operation_request.program != "__simulate_incomplete_journal__";

            let journal_footer = JournalTerminalFooter {
                command_id: signed_envelope.envelope.command_id.clone(),
                generation: signed_envelope.envelope.generation,
                epoch_uuid: signed_envelope.envelope.epoch_uuid,
                nonce: signed_envelope.envelope.nonce.clone(),
                exit_code,
                stdout: stdout.clone(),
                stderr: stderr.clone(),
                journal_complete,
            };

            let last_gasp_packet_raw = if journal_complete {
                None
            } else {
                let packet = LastGaspPacket {
                    command_id: signed_envelope.envelope.command_id.clone(),
                    generation: signed_envelope.envelope.generation,
                    epoch_uuid: signed_envelope.envelope.epoch_uuid,
                    nonce: signed_envelope.envelope.nonce.clone(),
                    terminal_state: "exited".to_string(),
                    exit_code_or_signal: exit_code,
                    last_error: if exit_code == 0 {
                        String::new()
                    } else {
                        stderr.clone()
                    },
                    stdout,
                    stderr,
                };

                Some(
                    serde_json::to_string(&packet).map_err(|error| {
                        InfraExecutionError::Environment(EnvironmentError::Serialization {
                            details: format!("failed to serialize last-gasp packet: {error}"),
                        })
                    })?,
                )
            };

            Ok(HelperExecutionEvidence {
                journal_footer: Some(journal_footer),
                last_gasp_packet_raw,
            })
        })
        .await
    }

    fn parse_last_gasp_packet(raw_packet: &str) -> Option<LastGaspPacket> {
        serde_json::from_str::<LastGaspPacket>(raw_packet).ok()
    }

    fn envelope_matches_fence(
        envelope: &ExecutionEnvelope,
        command_id: &str,
        generation: u64,
        epoch_uuid: &[u8; 16],
        nonce: &str,
    ) -> bool {
        envelope.command_id == command_id
            && envelope.generation == generation
            && &envelope.epoch_uuid == epoch_uuid
            && envelope.nonce == nonce
    }

    fn resolve_terminal_evidence(
        envelope: &ExecutionEnvelope,
        evidence: HelperExecutionEvidence,
    ) -> Result<ResolvedTerminalStatus, EnvironmentError> {
        if let Some(journal) = evidence.journal_footer {
            let fence_ok = Self::envelope_matches_fence(
                envelope,
                &journal.command_id,
                journal.generation,
                &journal.epoch_uuid,
                &journal.nonce,
            );

            if fence_ok && journal.journal_complete {
                return Ok(ResolvedTerminalStatus {
                    source: EvidenceSource::Journal,
                    exit_code: journal.exit_code,
                    stdout: journal.stdout,
                    stderr: journal.stderr,
                });
            }
        }

        if let Some(last_gasp_raw) = evidence.last_gasp_packet_raw {
            if let Some(last_gasp) = Self::parse_last_gasp_packet(&last_gasp_raw) {
                let fence_ok = Self::envelope_matches_fence(
                    envelope,
                    &last_gasp.command_id,
                    last_gasp.generation,
                    &last_gasp.epoch_uuid,
                    &last_gasp.nonce,
                );

                if fence_ok {
                    return Ok(ResolvedTerminalStatus {
                        source: EvidenceSource::LastGasp,
                        exit_code: last_gasp.exit_code_or_signal,
                        stdout: last_gasp.stdout,
                        stderr: last_gasp.stderr,
                    });
                }
            }
        }

        Err(EnvironmentError::EnvironmentResetRequired {
            reason: format!(
                "ambiguous terminal status for command {} (journal incomplete and no valid last-gasp fallback)",
                envelope.command_id
            ),
        })
    }

    fn count_lines(text: &str) -> usize {
        text.as_bytes().iter().filter(|byte| **byte == b'\n').count()
    }

    fn enforce_output_limits(
        request: &CommandRequest,
        stdout: &str,
        stderr: &str,
    ) -> Result<(), EnvironmentError> {
        let observed_bytes = stdout.len().saturating_add(stderr.len());
        let observed_lines = Self::count_lines(stdout).saturating_add(Self::count_lines(stderr));

        if observed_bytes > request.max_bytes || observed_lines > request.max_lines {
            return Err(EnvironmentError::OutputLimitExceeded {
                max_bytes: request.max_bytes,
                max_lines: request.max_lines,
                observed_bytes,
                observed_lines,
            });
        }

        Ok(())
    }

    async fn remove_inflight_command(&self, command_id: &str) {
        let mut inflight = self.inflight_registry.write().await;
        inflight.remove(command_id);
    }

    fn bytes_sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    fn normalize_path(path: &Path) -> PathBuf {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    normalized.pop();
                }
                Component::RootDir | Component::Normal(_) | Component::Prefix(_) => {
                    normalized.push(component.as_os_str());
                }
            }
        }
        normalized
    }

    fn resolve_requested_path(&self, requested: &Path) -> Result<PathBuf, EnvironmentError> {
        if let Some(root) = &self.config.remote_workspace_root {
            let root_normalized = Self::normalize_path(root);
            let candidate = if requested.is_absolute() {
                requested.to_path_buf()
            } else {
                root_normalized.join(requested)
            };
            let normalized = Self::normalize_path(&candidate);

            if !normalized.starts_with(&root_normalized) {
                return Err(EnvironmentError::PathTraversalDenied {
                    path: requested.display().to_string(),
                });
            }

            return Ok(normalized);
        }

        Ok(requested.to_path_buf())
    }

    fn staging_sidecar_path(staged_path: &Path) -> PathBuf {
        let mut sidecar = staged_path.as_os_str().to_os_string();
        sidecar.push(".lease.json");
        PathBuf::from(sidecar)
    }

    fn persist_staging_metadata_sidecar(
        sidecar_path: &Path,
        metadata: &StagedArtifactLeaseMetadata,
    ) -> Result<(), InfraExecutionError> {
        let bytes = serde_json::to_vec(metadata).map_err(|error| {
            InfraExecutionError::Environment(EnvironmentError::Serialization {
                details: format!("failed to serialize staging lease sidecar: {error}"),
            })
        })?;

        let mut file = File::create(sidecar_path).map_err(|error| {
            InfraExecutionError::Environment(EnvironmentError::Io {
                operation: "write_file::sidecar_create".to_string(),
                details: format!("{} ({})", error, sidecar_path.display()),
            })
        })?;

        file.write_all(&bytes).map_err(|error| {
            InfraExecutionError::Environment(EnvironmentError::Io {
                operation: "write_file::sidecar_write".to_string(),
                details: format!("{} ({})", error, sidecar_path.display()),
            })
        })?;

        file.sync_all().map_err(|error| {
            InfraExecutionError::Environment(EnvironmentError::Io {
                operation: "write_file::sidecar_sync".to_string(),
                details: format!("{} ({})", error, sidecar_path.display()),
            })
        })?;

        Ok(())
    }

    async fn verify_emergency_status_buffer(&self) -> Result<(), EnvironmentError> {
        if self.config.helper_provisioning.emergency_status_buffer_bytes
            < EMERGENCY_STATUS_BUFFER_MIN_BYTES
        {
            return Err(EnvironmentError::StartupPolicyNotReady {
                policy: "helper_emergency_status_buffer_bytes".to_string(),
                details: format!(
                    "configured={} must be >= {} bytes",
                    self.config.helper_provisioning.emergency_status_buffer_bytes,
                    EMERGENCY_STATUS_BUFFER_MIN_BYTES
                ),
            });
        }

        self.run_infra_control_op("ensure_ready::emergency_status_buffer_check", async {
            let mut reserved = Vec::<u8>::new();
            reserved
                .try_reserve_exact(EMERGENCY_STATUS_BUFFER_MIN_BYTES as usize)
                .map_err(|error| {
                    InfraExecutionError::Environment(EnvironmentError::StartupPolicyNotReady {
                        policy: "helper_emergency_status_buffer_allocation".to_string(),
                        details: error.to_string(),
                    })
                })?;
            reserved.resize(EMERGENCY_STATUS_BUFFER_MIN_BYTES as usize, 0);
            Ok(())
        })
        .await
    }

    async fn run_reset_priority_step(
        &self,
        quota: &mut ResetPriorityQuota,
        operation: &'static str,
        priority: InfrastructureTaskPriority,
    ) -> Result<(), EnvironmentError> {
        self.run_infra_control_op(operation, async { Ok(()) }).await?;

        if priority == InfrastructureTaskPriority::HighRecovery && quota.on_high_recovery_step() {
            let host = self.config.host.clone();
            self.run_infra_control_op("reset_environment::medium_reconnect_slot", async move {
                if host.trim().is_empty() {
                    return Err(InfraExecutionError::Environment(
                        EnvironmentError::NetworkPolicyNotReady {
                            mode: "reconnect".to_string(),
                            details: "host must not be empty for reconnect slot".to_string(),
                        },
                    ));
                }
                Ok(())
            })
            .await?;
            self.transport_generation_id.fetch_add(1, Ordering::AcqRel);
        }

        Ok(())
    }

    async fn wait_for_admission_barrier_or_zombie_reap(
        &self,
    ) -> Result<AdmissionBarrierOutcome, EnvironmentError> {
        let freeze_timeout = Duration::from_millis(self.config.reset_policy.admission_freeze_timeout_ms.max(1));
        let freeze_deadline = Instant::now() + freeze_timeout;

        while !self.admission_dual_barrier_satisfied().await && Instant::now() < freeze_deadline {
            tokio::time::sleep(Duration::from_millis(RESET_SPIN_SLEEP_MS)).await;
        }

        if self.admission_dual_barrier_satisfied().await {
            return Ok(AdmissionBarrierOutcome::BarrierReached);
        }

        let orphaned_handles = self.orphan_inflight_handles_to_zombies().await;

        let zombie_timeout =
            Duration::from_millis(self.config.reset_policy.zombie_reap_timeout_ms.max(1));
        let zombie_deadline = Instant::now() + zombie_timeout;
        while !self.admission_dual_barrier_satisfied().await && Instant::now() < zombie_deadline {
            tokio::time::sleep(Duration::from_millis(RESET_SPIN_SLEEP_MS)).await;
        }

        let _ = self.orphan_inflight_handles_to_zombies().await;

        // RFC-002 Section 10.2 self-healing barrier synthesis after bounded zombie reap.
        if self.admission_inflight.load(Ordering::Acquire) > 0 {
            self.admission_inflight.store(0, Ordering::Release);
        }

        Ok(AdmissionBarrierOutcome::ZombieReaping { orphaned_handles })
    }

    async fn orphan_inflight_handles_to_zombies(&self) -> usize {
        let orphaned = {
            let mut inflight = self.inflight_registry.write().await;
            inflight.drain().map(|(_, handle)| handle).collect::<Vec<_>>()
        };

        let mut zombies = self.zombie_commands.write().await;
        for handle in &orphaned {
            handle.cancel_token.cancel();
            zombies.insert(handle.command_id.clone());
        }

        orphaned.len()
    }

    async fn admission_dual_barrier_satisfied(&self) -> bool {
        if self.admission_inflight.load(Ordering::Acquire) != 0 {
            return false;
        }

        self.inflight_registry.read().await.is_empty()
    }

    fn is_command_fence_current(&self, generation: u64, epoch_uuid: Uuid) -> bool {
        self.generation.load(Ordering::Acquire) == generation && self.current_epoch_uuid() == epoch_uuid
    }

    fn classify_infra_slot(operation: &str) -> InfraSlotClass {
        if operation == "reset_environment::medium_reconnect_slot" || operation.contains("reconnect") {
            return InfraSlotClass::Medium;
        }

        if operation.starts_with("snapshot::") {
            return InfraSlotClass::HighReset;
        }

        if operation.starts_with("reset_environment::") {
            return InfraSlotClass::HighReset;
        }

        if operation.starts_with("cancel_inflight::") {
            return InfraSlotClass::HighOther;
        }

        InfraSlotClass::Low
    }

    fn high_reset_reserved_slots(&self) -> usize {
        let high_capacity = self
            .config
            .infrastructure_runtime
            .high_priority_queue_capacity
            .max(1);

        let reserved = (high_capacity * INFRA_RESET_RESERVED_NUMERATOR
            + (INFRA_RESET_RESERVED_DENOMINATOR - 1))
            / INFRA_RESET_RESERVED_DENOMINATOR;
        reserved.max(1).min(high_capacity)
    }

    fn acquire_infra_slot(&self, operation: &'static str) -> Result<InfraPrioritySlotGuard<'_>, EnvironmentError> {
        let slot_class = Self::classify_infra_slot(operation);
        let mut counters = self
            .infra_priority_counters
            .lock()
            .map_err(|error| EnvironmentError::EnvironmentResetRequired {
                reason: format!("infra priority counter lock poisoned: {error}"),
            })?;

        let high_capacity = self
            .config
            .infrastructure_runtime
            .high_priority_queue_capacity
            .max(1);
        let medium_capacity = self
            .config
            .infrastructure_runtime
            .medium_priority_queue_capacity
            .max(1);
        let low_capacity = self
            .config
            .infrastructure_runtime
            .low_priority_queue_capacity
            .max(1);
        let high_reserved_for_reset = self.high_reset_reserved_slots();
        let high_non_reset_capacity = high_capacity.saturating_sub(high_reserved_for_reset);

        let admitted = match slot_class {
            InfraSlotClass::HighReset => {
                let high_total = counters.high_reset_inflight + counters.high_other_inflight;
                if high_total >= high_capacity {
                    false
                } else {
                    counters.high_reset_inflight += 1;
                    true
                }
            }
            InfraSlotClass::HighOther => {
                let high_total = counters.high_reset_inflight + counters.high_other_inflight;
                if high_total >= high_capacity
                    || counters.high_other_inflight >= high_non_reset_capacity
                {
                    false
                } else {
                    counters.high_other_inflight += 1;
                    true
                }
            }
            InfraSlotClass::Medium => {
                if counters.medium_inflight >= medium_capacity {
                    false
                } else {
                    counters.medium_inflight += 1;
                    true
                }
            }
            InfraSlotClass::Low => {
                if counters.low_inflight >= low_capacity {
                    false
                } else {
                    counters.low_inflight += 1;
                    true
                }
            }
        };

        if !admitted {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!(
                    "infra priority queue saturated for operation={} class={:?}",
                    operation, slot_class
                ),
            });
        }

        Ok(InfraPrioritySlotGuard {
            counters: &self.infra_priority_counters,
            slot_class,
        })
    }

    fn validate_host_artifact_owner_with_binary_fingerprint(
        owner_pid: Option<u32>,
        owner_pid_start_time_ticks: Option<u64>,
        owner_binary_sha256_or_build_id: Option<&str>,
        observed_pid: u32,
        observed_pid_start_time_ticks: u64,
        active_binary_sha256_or_build_id: &str,
    ) -> bool {
        HostGarbageCollector::validate_owner_triple_with_fingerprint(
            owner_pid,
            owner_pid_start_time_ticks,
            owner_binary_sha256_or_build_id,
            observed_pid,
            observed_pid_start_time_ticks,
            active_binary_sha256_or_build_id,
        )
    }

    fn owner_triple_check_live(
        metadata: &StagedArtifactLeaseMetadata,
        active_binary_fingerprint: &str,
    ) -> bool {
        let owner_pid = match metadata.owner_pid {
            Some(pid) => pid,
            None => return false,
        };
        let expected_start_ticks = match metadata.owner_pid_start_time_ticks {
            Some(start_ticks) => start_ticks,
            None => return false,
        };
        let owner_fingerprint = match metadata.owner_binary_sha256_or_build_id.as_deref() {
            Some(fingerprint) => fingerprint,
            None => return false,
        };

        match Self::process_start_time_ticks(owner_pid) {
            Ok(observed_start_ticks) => Self::validate_host_artifact_owner_with_binary_fingerprint(
                Some(owner_pid),
                Some(expected_start_ticks),
                Some(owner_fingerprint),
                owner_pid,
                observed_start_ticks,
                active_binary_fingerprint,
            ),
            Err(_) => false,
        }
    }

    fn should_delete_staged_artifact(
        metadata: &StagedArtifactLeaseMetadata,
        now_unix_ms: u64,
        staging_sweep_ttl_ms: u64,
        staging_heartbeat_timeout_ms: u64,
        owner_triple_check_failed: bool,
    ) -> bool {
        let expired = now_unix_ms.saturating_sub(metadata.created_unix_ms) > staging_sweep_ttl_ms;
        let no_recent_heartbeat =
            now_unix_ms.saturating_sub(metadata.lease_heartbeat_unix_ms) > staging_heartbeat_timeout_ms;

        expired && no_recent_heartbeat && owner_triple_check_failed
    }

    async fn run_global_liveness_aware_staging_sweep(&self) -> Result<usize, EnvironmentError> {
        let snapshot = self.staged_artifact_index.read().await.clone();
        let now_unix_ms = Self::now_unix_ms();
        let ttl_ms = self
            .config
            .file_commit_policy
            .staging_sweep_ttl_secs
            .saturating_mul(1000);
        let heartbeat_timeout_ms = self
            .config
            .file_commit_policy
            .staging_lease_heartbeat_timeout_ms;
        let sweep_batch_limit = self
            .config
            .file_commit_policy
            .staging_sweep_batch_limit
            .max(1) as usize;
        let active_binary_fingerprint = self
            .config
            .host_artifact_gc
            .host_binary_sha256_or_build_id
            .clone();

        let deleted = self
            .run_infra_control_op("reset_environment::global_liveness_sweep", async move {
                let mut deleted = Vec::<(String, PathBuf)>::new();

                'outer: for (command_id, artifacts) in snapshot {
                    for (staged_path, metadata) in artifacts {
                        let owner_live = QemuSshEnvironment::owner_triple_check_live(
                            &metadata,
                            &active_binary_fingerprint,
                        );

                        let should_delete = QemuSshEnvironment::should_delete_staged_artifact(
                            &metadata,
                            now_unix_ms,
                            ttl_ms,
                            heartbeat_timeout_ms,
                            !owner_live,
                        );

                        if !should_delete {
                            continue;
                        }

                        let _ = std::fs::remove_file(&staged_path);
                        let sidecar = QemuSshEnvironment::staging_sidecar_path(&staged_path);
                        let _ = std::fs::remove_file(&sidecar);
                        deleted.push((command_id.clone(), staged_path));

                        if deleted.len() >= sweep_batch_limit {
                            break 'outer;
                        }
                    }
                }

                Ok(deleted)
            })
            .await?;

        if deleted.is_empty() {
            return Ok(0);
        }

        let mut index = self.staged_artifact_index.write().await;
        for (command_id, staged_path) in &deleted {
            if let Some(artifacts) = index.get_mut(command_id) {
                artifacts.remove(staged_path);
            }
        }
        let empty_keys = index
            .iter()
            .filter_map(|(command_id, artifacts)| {
                if artifacts.is_empty() {
                    Some(command_id.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for command_id in empty_keys {
            index.remove(&command_id);
        }

        Ok(deleted.len())
    }

    async fn register_staged_artifact(
        &self,
        command_id: &str,
        staged_active_path: PathBuf,
        metadata: StagedArtifactLeaseMetadata,
    ) {
        let mut index = self.staged_artifact_index.write().await;
        let artifacts = index.entry(command_id.to_string()).or_insert_with(HashMap::new);
        artifacts.insert(staged_active_path, metadata);
    }

    async fn unregister_staged_artifact(&self, command_id: &str, staged_active_path: &Path) {
        let mut index = self.staged_artifact_index.write().await;
        if let Some(artifacts) = index.get_mut(command_id) {
            artifacts.remove(staged_active_path);
            if artifacts.is_empty() {
                index.remove(command_id);
            }
        }
    }

    async fn run_privileged_commit(
        &self,
        staged_active_path: PathBuf,
        destination_path: PathBuf,
        expected_sha256: String,
        expected_bytes: usize,
    ) -> Result<(), EnvironmentError> {
        if cfg!(target_os = "linux") && !self.config.file_commit_policy.enforce_linux_openat2 {
            return Err(EnvironmentError::StartupPolicyNotReady {
                policy: "privileged_commit_openat2".to_string(),
                details:
                    "Linux privileged commit requires openat2 enforcement with RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS"
                        .to_string(),
            });
        }

        let helper = self
            .config
            .file_commit_policy
            .privileged_commit_helper_path
            .clone()
            .unwrap_or_else(|| PathBuf::from("kria-guest-helper"));
        let timeout_ms = self
            .config
            .file_commit_policy
            .privileged_commit_timeout_ms
            .max(1);

        self.run_infra_control_op("write_file::privileged_commit", async move {
            let mut args = vec![
                "-n".to_string(),
                helper.to_string_lossy().to_string(),
                "commit-file".to_string(),
                "--staged".to_string(),
                staged_active_path.to_string_lossy().to_string(),
                "--dest".to_string(),
                destination_path.to_string_lossy().to_string(),
                "--sha256".to_string(),
                expected_sha256,
                "--bytes".to_string(),
                expected_bytes.to_string(),
            ];

            #[cfg(target_os = "linux")]
            {
                args.push("--require-openat2".to_string());
                args.push("--resolve-beneath".to_string());
                args.push("--resolve-no-symlinks".to_string());
            }

            let status = std::process::Command::new("sudo")
                .args(args)
                .status()
                .map_err(|error| {
                    InfraExecutionError::Environment(EnvironmentError::Io {
                        operation: "write_file::privileged_commit_spawn".to_string(),
                        details: error.to_string(),
                    })
                })?;

            if !status.success() {
                return Err(InfraExecutionError::Environment(
                    EnvironmentError::EnvironmentResetRequired {
                        reason: format!(
                            "sudo kria-guest-helper commit-file failed with status={} timeout_ms={}",
                            status,
                            timeout_ms
                        ),
                    },
                ));
            }

            Ok(())
        })
        .await
    }

    async fn run_unprivileged_commit(
        &self,
        staged_active_path: PathBuf,
        destination_path: PathBuf,
        create_parent: bool,
    ) -> Result<(), EnvironmentError> {
        self.run_infra_control_op("write_file::unprivileged_commit", async move {
            if create_parent {
                if let Some(parent) = destination_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        InfraExecutionError::Environment(EnvironmentError::Io {
                            operation: "write_file::create_parent".to_string(),
                            details: format!("{} ({})", error, parent.display()),
                        })
                    })?;
                }
            }

            std::fs::rename(&staged_active_path, &destination_path).map_err(|error| {
                InfraExecutionError::Environment(EnvironmentError::Io {
                    operation: "write_file::rename_active_to_destination".to_string(),
                    details: format!(
                        "{} ({} -> {})",
                        error,
                        staged_active_path.display(),
                        destination_path.display()
                    ),
                })
            })?;

            Ok(())
        })
        .await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEnvelope {
    pub command_id: String,
    pub generation: u64,
    pub epoch_uuid: [u8; 16],
    pub transport_generation_id: u64,
    pub instance_id: String,
    pub issued_at_host_unix_ms_info_only: u64,
    pub ttl_ms_from_receipt: u64,
    pub nonce: String,
    pub parent_session_nonce: String,
    pub parent_ssh_session_pid: Option<u32>,
    pub parent_ssh_session_start_time_ticks: Option<u64>,
    pub cwd: String,
    pub env: std::collections::BTreeMap<String, String>,
    pub program: String,
    pub args: Vec<String>,
    pub command_sha256: String,
    pub stdin_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SignedExecutionEnvelope {
    envelope: ExecutionEnvelope,
    control_envelope: SignedEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalTerminalFooter {
    command_id: String,
    generation: u64,
    epoch_uuid: [u8; 16],
    nonce: String,
    exit_code: i32,
    stdout: String,
    stderr: String,
    journal_complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LastGaspPacket {
    command_id: String,
    generation: u64,
    epoch_uuid: [u8; 16],
    nonce: String,
    terminal_state: String,
    exit_code_or_signal: i32,
    last_error: String,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone)]
struct HelperExecutionEvidence {
    journal_footer: Option<JournalTerminalFooter>,
    last_gasp_packet_raw: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvidenceSource {
    Journal,
    LastGasp,
}

#[derive(Debug, Clone)]
struct ResolvedTerminalStatus {
    source: EvidenceSource,
    exit_code: i32,
    stdout: String,
    stderr: String,
}

#[derive(Debug)]
enum InfraExecutionError {
    Environment(EnvironmentError),
    SocketFailure(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InfraSlotClass {
    HighReset,
    HighOther,
    Medium,
    Low,
}

#[derive(Debug, Default)]
struct InfraPriorityCounters {
    high_reset_inflight: usize,
    high_other_inflight: usize,
    medium_inflight: usize,
    low_inflight: usize,
}

struct InfraPrioritySlotGuard<'a> {
    counters: &'a StdMutex<InfraPriorityCounters>,
    slot_class: InfraSlotClass,
}

impl Drop for InfraPrioritySlotGuard<'_> {
    fn drop(&mut self) {
        let Ok(mut counters) = self.counters.lock() else {
            return;
        };

        match self.slot_class {
            InfraSlotClass::HighReset => {
                counters.high_reset_inflight = counters.high_reset_inflight.saturating_sub(1)
            }
            InfraSlotClass::HighOther => {
                counters.high_other_inflight = counters.high_other_inflight.saturating_sub(1)
            }
            InfraSlotClass::Medium => {
                counters.medium_inflight = counters.medium_inflight.saturating_sub(1)
            }
            InfraSlotClass::Low => {
                counters.low_inflight = counters.low_inflight.saturating_sub(1)
            }
        }
    }
}

#[derive(Debug, Default)]
struct HostGarbageCollector;

impl HostGarbageCollector {
    fn validate_owner_triple_with_fingerprint(
        owner_pid: Option<u32>,
        owner_pid_start_time_ticks: Option<u64>,
        owner_binary_sha256_or_build_id: Option<&str>,
        observed_pid: u32,
        observed_pid_start_time_ticks: u64,
        active_binary_sha256_or_build_id: &str,
    ) -> bool {
        let (owner_pid, owner_pid_start_time_ticks, owner_binary_sha256_or_build_id) =
            match (
                owner_pid,
                owner_pid_start_time_ticks,
                owner_binary_sha256_or_build_id,
            ) {
                (Some(pid), Some(start_ticks), Some(binary_fingerprint)) => {
                    (pid, start_ticks, binary_fingerprint)
                }
                _ => return false,
            };

        owner_pid == observed_pid
            && owner_pid_start_time_ticks == observed_pid_start_time_ticks
            && owner_binary_sha256_or_build_id == active_binary_sha256_or_build_id
    }
}

struct AdmissionInflightGuard<'a> {
    counter: &'a AtomicU64,
}

impl<'a> AdmissionInflightGuard<'a> {
    fn new(counter: &'a AtomicU64) -> Self {
        counter.fetch_add(1, Ordering::AcqRel);
        Self { counter }
    }
}

impl Drop for AdmissionInflightGuard<'_> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
    }
}

struct ResetInProgressGuard<'a> {
    flag: &'a AtomicBool,
}

impl<'a> ResetInProgressGuard<'a> {
    async fn acquire(
        flag: &'a AtomicBool,
        lock_acquire_timeout_ms: u64,
    ) -> Result<Self, EnvironmentError> {
        let retry_budget = Duration::from_millis(lock_acquire_timeout_ms.max(1));
        let deadline = Instant::now() + retry_budget;

        loop {
            if flag
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(Self { flag });
            }

            if Instant::now() >= deadline {
                return Err(EnvironmentError::EnvironmentResetFailed {
                    reason: "reset already in progress".to_string(),
                    details: format!(
                        "try-lock retry budget exhausted after {}ms",
                        retry_budget.as_millis()
                    ),
                });
            }

            tokio::time::sleep(Duration::from_millis(RESET_SPIN_SLEEP_MS)).await;
        }
    }
}

impl Drop for ResetInProgressGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmissionBarrierOutcome {
    BarrierReached,
    ZombieReaping { orphaned_handles: usize },
}

#[derive(Debug, Default, Clone, Copy)]
struct ResetPriorityQuota {
    high_since_medium: u8,
}

impl ResetPriorityQuota {
    fn on_high_recovery_step(&mut self) -> bool {
        self.high_since_medium = self.high_since_medium.saturating_add(1);
        if self.high_since_medium >= INFRA_HIGH_STEPS_PER_MEDIUM_RECONNECT {
            self.high_since_medium = 0;
            return true;
        }
        false
    }
}

#[async_trait]
impl CommandExecutor for QemuSshEnvironment {
    async fn execute_command(
        &self,
        mut request: CommandRequest,
        shell_state_snapshot: ShellState,
    ) -> Result<CommandResult, EnvironmentError> {
        let lease_id = self.require_active_verified_lease().await?;

        if self.tainted.load(Ordering::Acquire) {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: "provider is tainted; reset required before executing command".to_string(),
            });
        }

        if !self
            .helper_worker_stdout_stderr_local_logs
            .load(Ordering::Acquire)
        {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason:
                    "helper output pipe safety not configured: worker stdout/stderr must redirect to local logs"
                        .to_string(),
            });
        }

        request.timeout_ms = request.timeout_ms.min(self.config.command_timeout_ms.max(1));
        let _admission_guard = AdmissionInflightGuard::new(&self.admission_inflight);

        let command_id = Uuid::new_v4().to_string();
        let epoch_uuid = self.current_epoch_uuid();
        let generation = self.generation.load(Ordering::Acquire);
        let transport_generation_id = self.transport_generation_id.load(Ordering::Acquire);
        let nonce = Uuid::new_v4().to_string();

        let parent_identity = self.capture_parent_identity()?;
        let observed_identity = ParentIdentity {
            pid: parent_identity.pid,
            start_time_ticks: Self::process_start_time_ticks(parent_identity.pid)?,
            session_nonce: parent_identity.session_nonce.clone(),
        };
        if !Self::validate_parent_identity_triple(&parent_identity, &observed_identity) {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: "triple-check parent identity validation failed before dispatch"
                    .to_string(),
            });
        }

        let envelope = self.build_execution_envelope(
            &request,
            &shell_state_snapshot,
            command_id.clone(),
            generation,
            epoch_uuid,
            transport_generation_id,
            &parent_identity,
            nonce,
        );
        let signed_envelope = self.sign_execution_envelope(&envelope, lease_id).await?;

        self.helper_accepts_initialize(&command_id, epoch_uuid).await?;

        {
            let mut inflight = self.inflight_registry.write().await;
            inflight.insert(
                command_id.clone(),
                InflightCommandHandle {
                    command_id: command_id.clone(),
                    generation,
                    epoch_uuid,
                    transport_generation_id,
                    cancel_token: CancellationToken::new(),
                    local_process_ids: Vec::new(),
                    remote_status_path: self
                        .config
                        .remote_control_dir
                        .join(format!("{}.status.json", command_id)),
                    remote_tmp_paths: HashSet::new(),
                    parent_identity: Some(parent_identity.clone()),
                    helper_supervisor_pid: None,
                    helper_worker_pid: None,
                    helper_worker_start_time_ticks: None,
                },
            );
        }

        let evidence = self
            .execute_over_control_channel(
                signed_envelope,
                request.clone(),
                shell_state_snapshot,
            )
            .await;

        self.remove_inflight_command(&command_id).await;

        let evidence = evidence?;

        if !self.is_command_fence_current(generation, epoch_uuid) {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!(
                    "dropped late terminal status for command {} from stale generation/epoch fence",
                    command_id
                ),
            });
        }

        let terminal = Self::resolve_terminal_evidence(&envelope, evidence)?;

        if terminal.source == EvidenceSource::LastGasp && terminal.exit_code == 0 {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!(
                    "journal terminal footer missing for command {}; last-gasp success is provisional",
                    envelope.command_id
                ),
            });
        }

        Self::enforce_output_limits(&request, &terminal.stdout, &terminal.stderr)?;

        if terminal.exit_code != 0 {
            return Err(EnvironmentError::CommandFailed {
                exit_code: terminal.exit_code,
                stderr: terminal.stderr,
            });
        }

        Ok(CommandResult {
            exit_code: terminal.exit_code,
            stdout: terminal.stdout,
            stderr: terminal.stderr,
            truncated: false,
        })
    }
}

#[async_trait]
impl FileSystemOps for QemuSshEnvironment {
    async fn read_file(&self, request: ReadFileRequest) -> Result<ReadFileResult, EnvironmentError> {
        let path = self.resolve_requested_path(&request.path)?;
        let read_limit = self.config.max_read_file_bytes.max(1);

        self.run_infra_control_op("read_file::bounded", async move {
            let metadata = std::fs::metadata(&path).map_err(|error| {
                InfraExecutionError::Environment(EnvironmentError::Io {
                    operation: "read_file::metadata".to_string(),
                    details: format!("{} ({})", error, path.display()),
                })
            })?;

            if !metadata.is_file() {
                return Err(InfraExecutionError::Environment(EnvironmentError::Io {
                    operation: "read_file::metadata".to_string(),
                    details: format!("path is not a regular file: {}", path.display()),
                }));
            }

            let contents = std::fs::read(&path).map_err(|error| {
                InfraExecutionError::Environment(EnvironmentError::Io {
                    operation: "read_file::read".to_string(),
                    details: format!("{} ({})", error, path.display()),
                })
            })?;

            if contents.len() as u64 > read_limit {
                return Err(InfraExecutionError::Environment(
                    EnvironmentError::StorageLimitExceeded {
                        limit_bytes: read_limit,
                        observed_bytes: contents.len() as u64,
                    },
                ));
            }

            Ok(ReadFileResult { contents })
        })
        .await
    }

    async fn write_file(
        &self,
        request: WriteFileRequest,
    ) -> Result<WriteFileResult, EnvironmentError> {
        if self.tainted.load(Ordering::Acquire) {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: "provider is tainted; reset required before write_file".to_string(),
            });
        }

        let _admission_guard = AdmissionInflightGuard::new(&self.admission_inflight);

        let destination_path = self.resolve_requested_path(&request.path)?;
        let command_id = Uuid::new_v4().to_string();
        let generation = self.generation.load(Ordering::Acquire);
        let epoch_uuid = self.current_epoch_uuid();

        let staging_root = self
            .config
            .file_commit_policy
            .remote_staging_dir
            .join(&self.config.instance_id);
        let staged_incomplete_path = staging_root.join(format!("{}.upload.incomplete", command_id));
        let staged_active_path = staging_root.join(format!("{}.upload.active", command_id));
        let sidecar_incomplete_path = Self::staging_sidecar_path(&staged_incomplete_path);
        let sidecar_active_path = Self::staging_sidecar_path(&staged_active_path);

        let owner_pid = std::process::id();
        let owner_pid_start_time_ticks = Self::process_start_time_ticks(owner_pid).ok();
        let now_unix_ms = Self::now_unix_ms();
        let contents = request.contents.clone();
        let expected_sha256 = Self::bytes_sha256_hex(&contents);

        let seed_metadata = StagedArtifactLeaseMetadata {
            owner_instance_id: self.config.instance_id.clone(),
            owner_pid: Some(owner_pid),
            owner_pid_start_time_ticks,
            owner_binary_sha256_or_build_id: Some(
                self.config
                    .host_artifact_gc
                    .host_binary_sha256_or_build_id
                    .clone(),
            ),
            generation,
            epoch_uuid,
            artifact_nonce: Uuid::new_v4().to_string(),
            created_unix_ms: now_unix_ms,
            lease_heartbeat_unix_ms: now_unix_ms,
            expected_sha256: expected_sha256.clone(),
            bytes: contents.len() as u64,
        };

        let stage_staging_root = staging_root.clone();
        let stage_incomplete_path = staged_incomplete_path.clone();
        let stage_sidecar_incomplete_path = sidecar_incomplete_path.clone();
        let staged_metadata = self
            .run_infra_control_op("write_file::stage_incomplete", async move {
                std::fs::create_dir_all(&stage_staging_root).map_err(|error| {
                    InfraExecutionError::Environment(EnvironmentError::Io {
                        operation: "write_file::create_staging_root".to_string(),
                        details: format!("{} ({})", error, stage_staging_root.display()),
                    })
                })?;

                let mut staged_file = File::create(&stage_incomplete_path).map_err(|error| {
                    InfraExecutionError::Environment(EnvironmentError::Io {
                        operation: "write_file::create_incomplete".to_string(),
                        details: format!("{} ({})", error, stage_incomplete_path.display()),
                    })
                })?;

                let mut metadata = seed_metadata;
                QemuSshEnvironment::persist_staging_metadata_sidecar(
                    &stage_sidecar_incomplete_path,
                    &metadata,
                )?;

                for chunk in contents.chunks(STAGING_HEARTBEAT_CHUNK_BYTES) {
                    staged_file.write_all(chunk).map_err(|error| {
                        InfraExecutionError::Environment(EnvironmentError::Io {
                            operation: "write_file::write_incomplete_chunk".to_string(),
                            details: format!("{} ({})", error, stage_incomplete_path.display()),
                        })
                    })?;

                    metadata.lease_heartbeat_unix_ms = QemuSshEnvironment::now_unix_ms();
                    QemuSshEnvironment::persist_staging_metadata_sidecar(
                        &stage_sidecar_incomplete_path,
                        &metadata,
                    )?;
                }

                staged_file.sync_all().map_err(|error| {
                    InfraExecutionError::Environment(EnvironmentError::Io {
                        operation: "write_file::sync_incomplete".to_string(),
                        details: format!("{} ({})", error, stage_incomplete_path.display()),
                    })
                })?;

                Ok(metadata)
            })
            .await?;

        self.register_staged_artifact(
            &command_id,
            staged_active_path.clone(),
            staged_metadata.clone(),
        )
        .await;

        let promote_incomplete_path = staged_incomplete_path.clone();
        let promote_active_path = staged_active_path.clone();
        let promote_sidecar_incomplete_path = sidecar_incomplete_path.clone();
        let promote_sidecar_active_path = sidecar_active_path.clone();
        let promote_result = self
            .run_infra_control_op("write_file::promote_to_active", async move {
                std::fs::rename(&promote_incomplete_path, &promote_active_path).map_err(|error| {
                    InfraExecutionError::Environment(EnvironmentError::Io {
                        operation: "write_file::rename_incomplete_to_active".to_string(),
                        details: format!(
                            "{} ({} -> {})",
                            error,
                            promote_incomplete_path.display(),
                            promote_active_path.display()
                        ),
                    })
                })?;

                std::fs::rename(&promote_sidecar_incomplete_path, &promote_sidecar_active_path)
                    .map_err(|error| {
                    InfraExecutionError::Environment(EnvironmentError::Io {
                        operation: "write_file::rename_sidecar_to_active".to_string(),
                        details: format!(
                            "{} ({} -> {})",
                            error,
                            promote_sidecar_incomplete_path.display(),
                            promote_sidecar_active_path.display()
                        ),
                    })
                })?;

                Ok(())
            })
            .await;

        if let Err(error) = promote_result {
            self.unregister_staged_artifact(&command_id, &staged_active_path)
                .await;
            return Err(error);
        }

        let commit_result = match self.config.file_commit_policy.privileged_commit_mode {
            PrivilegedCommitMode::Disabled => {
                self.run_unprivileged_commit(
                    staged_active_path.clone(),
                    destination_path.clone(),
                    request.create_parent,
                )
                .await
            }
            PrivilegedCommitMode::SudoMove | PrivilegedCommitMode::SudoHelperCommit => {
                self.run_privileged_commit(
                    staged_active_path.clone(),
                    destination_path,
                    staged_metadata.expected_sha256.clone(),
                    staged_metadata.bytes as usize,
                )
                .await
            }
        };

        if let Err(error) = commit_result {
            return Err(error);
        }

        let sidecar_cleanup_path = sidecar_active_path.clone();
        let _ = self
            .run_infra_control_op("write_file::cleanup_sidecar", async move {
                let _ = std::fs::remove_file(&sidecar_cleanup_path);
                Ok(())
            })
            .await;

        self.unregister_staged_artifact(&command_id, &staged_active_path)
            .await;

        Ok(WriteFileResult {
            bytes_written: request.contents.len(),
        })
    }

    async fn list_dir(&self, request: ListDirRequest) -> Result<ListDirResult, EnvironmentError> {
        let directory = self.resolve_requested_path(&request.path)?;

        self.run_infra_control_op("list_dir::read", async move {
            let mut entries = Vec::new();
            let read_dir = std::fs::read_dir(&directory).map_err(|error| {
                InfraExecutionError::Environment(EnvironmentError::Io {
                    operation: "list_dir::read_dir".to_string(),
                    details: format!("{} ({})", error, directory.display()),
                })
            })?;

            for entry in read_dir {
                let entry = entry.map_err(|error| {
                    InfraExecutionError::Environment(EnvironmentError::Io {
                        operation: "list_dir::next_entry".to_string(),
                        details: error.to_string(),
                    })
                })?;
                entries.push(entry.path());
            }

            entries.sort();
            Ok(ListDirResult { entries })
        })
        .await
    }
}

#[async_trait]
impl EnvironmentLifecycle for QemuSshEnvironment {
    async fn ensure_ready(&self) -> Result<(), EnvironmentError> {
        self.config
            .validate_static_contracts()
            .map_err(|error| EnvironmentError::StartupPolicyNotReady {
                policy: "remote_config".to_string(),
                details: error.to_string(),
            })?;

        self.run_control_health_check().await?;
        self.enforce_helper_output_pipe_safety().await?;
        self.verify_emergency_status_buffer().await?;
        self.probe_guest_capabilities().await?;
        self.ensure_qemu_process().await?;

        if let Err(error) = ensure_baseline_snapshot(self).await {
            tracing::warn!(
                error = %error,
                "remote_qemu: baseline snapshot bootstrap failed (non-fatal)"
            );
        }

        Ok(())
    }

    async fn reset_environment(&self, reason: ResetReason) -> Result<(), EnvironmentError> {
        let _reset_guard = ResetInProgressGuard::acquire(
            &self.reset_in_progress,
            self.config.reset_policy.lock_acquire_timeout_ms,
        )
        .await?;

        let reset_started = Instant::now();
        let total_deadline = Duration::from_millis(self.config.reset_policy.total_reset_deadline_ms.max(1));
        let ensure_deadline = |stage: &str| -> Result<(), EnvironmentError> {
            if reset_started.elapsed() > total_deadline {
                return Err(EnvironmentError::EnvironmentResetFailed {
                    reason: format!("{reason:?}"),
                    details: format!("reset deadline exceeded at stage {stage}"),
                });
            }
            Ok(())
        };

        match try_fast_restore_latest_snapshot(
            self,
            SnapshotDriftTolerance::from_system_config(&self.system_config),
        )
        .await
        {
            Ok(Some(report)) => {
                tracing::info!(
                    snapshot_id = %report.snapshot_id.0,
                    restore_latency_ms = report.restore_latency_ms,
                    drift_distance = report.drift_distance,
                    "remote_qemu: reset recovered via snapshot fast-path"
                );
                return Ok(());
            }
            Ok(None) => {
                tracing::debug!(
                    "remote_qemu: no snapshot available for fast-path recovery; using hard reset"
                );
            }
            Err(error) => {
                self.tainted.store(true, Ordering::Release);
                {
                    let mut reason_guard = self.taint_reason.lock().await;
                    *reason_guard = Some(format!(
                        "snapshot fast-path restore failed (fallback to hard reset): {}",
                        error
                    ));
                }
                tracing::warn!(
                    error = %error,
                    "remote_qemu: snapshot fast-path recovery failed; falling back to hard reset"
                );
            }
        }

        let mut quota = ResetPriorityQuota::default();

        self.run_reset_priority_step(
            &mut quota,
            "reset_environment::transition_tainted_frozen",
            InfrastructureTaskPriority::HighRecovery,
        )
        .await?;
        self.tainted.store(true, Ordering::Release);
        self.admissions_frozen.store(true, Ordering::Release);
        self.clear_verified_lease().await;
        {
            let mut reason_guard = self.taint_reason.lock().await;
            *reason_guard = Some(format!("reset requested: {reason:?}"));
        }
        ensure_deadline("transition_tainted_frozen")?;

        self.run_reset_priority_step(
            &mut quota,
            "reset_environment::admission_barrier",
            InfrastructureTaskPriority::HighRecovery,
        )
        .await?;
        let barrier_outcome = self.wait_for_admission_barrier_or_zombie_reap().await?;
        if let AdmissionBarrierOutcome::ZombieReaping { orphaned_handles } = barrier_outcome {
            let mut reason_guard = self.taint_reason.lock().await;
            *reason_guard = Some(format!(
                "reset entered ZombieReaping after admission barrier timeout; orphaned_handles={orphaned_handles}"
            ));
        }
        ensure_deadline("admission_barrier")?;

        self.run_reset_priority_step(
            &mut quota,
            "reset_environment::zombie_reap_drain",
            InfrastructureTaskPriority::HighRecovery,
        )
        .await?;
        let additionally_orphaned = self.orphan_inflight_handles_to_zombies().await;
        if additionally_orphaned > 0 {
            let mut reason_guard = self.taint_reason.lock().await;
            *reason_guard = Some(format!(
                "reset orphaned {} additional inflight handles during drain",
                additionally_orphaned
            ));
        }
        ensure_deadline("zombie_reap_drain")?;

        self.run_reset_priority_step(
            &mut quota,
            "reset_environment::global_liveness_sweep",
            InfrastructureTaskPriority::HighRecovery,
        )
        .await?;
        let _swept = self.run_global_liveness_aware_staging_sweep().await?;
        ensure_deadline("global_liveness_sweep")?;

        self.run_reset_priority_step(
            &mut quota,
            "reset_environment::epoch_generation_rotation",
            InfrastructureTaskPriority::HighRecovery,
        )
        .await?;
        let _ = self.generation.fetch_add(1, Ordering::AcqRel);
        let new_epoch = Uuid::new_v4();
        self.epoch_uuid.store(std::sync::Arc::new(new_epoch));
        self.transport_generation_id.fetch_add(1, Ordering::AcqRel);
        {
            let mut replay_cache = self.nonce_replay_cache.write().await;
            replay_cache.rotate_to_epoch(new_epoch);
        }
        self.helper_seen_initializations.write().await.clear();
        self.inflight_registry.write().await.clear();
        self.staged_artifact_index.write().await.clear();
        ensure_deadline("epoch_generation_rotation")?;

        self.run_reset_priority_step(
            &mut quota,
            "reset_environment::final_dual_barrier_check",
            InfrastructureTaskPriority::HighRecovery,
        )
        .await?;
        if !self.admission_dual_barrier_satisfied().await {
            return Err(EnvironmentError::EnvironmentResetFailed {
                reason: format!("{reason:?}"),
                details: "reset completion blocked: admission_inflight or inflight_registry still non-empty"
                    .to_string(),
            });
        }
        ensure_deadline("final_dual_barrier_check")?;

        self.run_reset_priority_step(
            &mut quota,
            "reset_environment::recover_to_healthy",
            InfrastructureTaskPriority::HighRecovery,
        )
        .await?;
        self.zombie_commands.write().await.clear();
        self.tainted.store(false, Ordering::Release);
        self.admissions_frozen.store(false, Ordering::Release);
        self.taint_reason.lock().await.take();

        if let Err(error) = ensure_baseline_snapshot(self).await {
            tracing::warn!(
                error = %error,
                "remote_qemu: post-reset baseline snapshot refresh failed (non-fatal)"
            );
        }

        Ok(())
    }

    async fn shutdown(&self) -> Result<(), EnvironmentError> {
        #[cfg(not(windows))]
        {
            let mut guard = self.qemu_child.lock().await;
            if let Some(mut child) = guard.take() {
                let kill_result = tokio::time::timeout(
                    Duration::from_millis(self.config.shutdown_timeout_ms.max(1)),
                    child.kill(),
                )
                .await;

                match kill_result {
                    Ok(Ok(())) | Ok(Err(_)) | Err(_) => {
                        let _ = child.wait().await;
                    }
                }
            }
        }

        #[cfg(windows)]
        {
            let mut guard = self.qemu_child.lock().await;
            if let Some(process) = guard.take() {
                process.force_terminate(Duration::from_millis(self.config.shutdown_timeout_ms.max(1)))?;
            }
        }

        let _ = tokio::fs::remove_file(&self.config.qemu_pid_state_file).await;
        self.clear_verified_lease().await;
        *self.provider_spawned_qemu.lock().await = false;
        Ok(())
    }
}

#[cfg(windows)]
impl WindowsQemuProcess {
    fn force_terminate(mut self, timeout: Duration) -> Result<(), EnvironmentError> {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};

        if self.process_handle != 0 {
            unsafe {
                let _ = TerminateProcess(self.process_handle, 1);
            }

            let wait_ms = timeout.as_millis().min(u128::from(u32::MAX)) as u32;
            let waited = unsafe { WaitForSingleObject(self.process_handle, wait_ms) };
            if waited != WAIT_OBJECT_0 {
                return Err(EnvironmentError::EnvironmentResetFailed {
                    reason: "windows_qemu_shutdown_timeout".to_string(),
                    details: format!("WaitForSingleObject returned status={waited}"),
                });
            }
        }

        unsafe {
            if self.job_handle != 0 {
                CloseHandle(self.job_handle);
                self.job_handle = 0;
            }
            if self.process_handle != 0 {
                CloseHandle(self.process_handle);
                self.process_handle = 0;
            }
        }

        Ok(())
    }
}

#[cfg(windows)]
impl Drop for WindowsQemuProcess {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;

        unsafe {
            if self.job_handle != 0 {
                CloseHandle(self.job_handle);
                self.job_handle = 0;
            }

            if self.process_handle != 0 {
                CloseHandle(self.process_handle);
                self.process_handle = 0;
            }
        }
    }
}

#[cfg(windows)]
mod windows_spawn {
    use std::ffi::OsStr;
    use std::mem;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, ResumeThread, TerminateProcess, CREATE_NEW_PROCESS_GROUP,
        CREATE_SUSPENDED, PROCESS_INFORMATION, STARTUPINFOW,
    };

    use super::{EnvironmentError, InfraExecutionError, WindowsQemuProcess};

    fn win32_error(operation: &str) -> InfraExecutionError {
        let code = unsafe { GetLastError() };
        InfraExecutionError::Environment(EnvironmentError::Io {
            operation: operation.to_string(),
            details: format!("win32_error_code={code}"),
        })
    }

    pub(super) fn spawn_qemu_windows_raw(
        qemu_boot_cmd: &str,
    ) -> Result<WindowsQemuProcess, InfraExecutionError> {
        let mut startup_info: STARTUPINFOW = unsafe { mem::zeroed() };
        startup_info.cb = mem::size_of::<STARTUPINFOW>() as u32;
        let mut process_info: PROCESS_INFORMATION = unsafe { mem::zeroed() };

        let mut command_line = OsStr::new(qemu_boot_cmd)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>();

        let created = unsafe {
            CreateProcessW(
                ptr::null(),
                command_line.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                CREATE_SUSPENDED | CREATE_NEW_PROCESS_GROUP,
                ptr::null(),
                ptr::null(),
                &mut startup_info,
                &mut process_info,
            )
        };

        if created == 0 {
            return Err(win32_error("CreateProcessW"));
        }

        let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if job == 0 {
            unsafe {
                let _ = TerminateProcess(process_info.hProcess, 1);
                CloseHandle(process_info.hThread);
                CloseHandle(process_info.hProcess);
            }
            return Err(win32_error("CreateJobObjectW"));
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set_info = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set_info == 0 {
            unsafe {
                let _ = TerminateProcess(process_info.hProcess, 1);
                CloseHandle(job);
                CloseHandle(process_info.hThread);
                CloseHandle(process_info.hProcess);
            }
            return Err(win32_error("SetInformationJobObject"));
        }

        let assigned = unsafe { AssignProcessToJobObject(job, process_info.hProcess) };
        if assigned == 0 {
            unsafe {
                let _ = TerminateProcess(process_info.hProcess, 1);
                CloseHandle(job);
                CloseHandle(process_info.hThread);
                CloseHandle(process_info.hProcess);
            }
            return Err(win32_error("AssignProcessToJobObject"));
        }

        let resumed = unsafe { ResumeThread(process_info.hThread) };
        if resumed == u32::MAX {
            unsafe {
                let _ = TerminateProcess(process_info.hProcess, 1);
                CloseHandle(job);
                CloseHandle(process_info.hThread);
                CloseHandle(process_info.hProcess);
            }
            return Err(win32_error("ResumeThread"));
        }

        unsafe {
            CloseHandle(process_info.hThread);
        }

        Ok(WindowsQemuProcess {
            process_handle: process_info.hProcess,
            job_handle: job,
            pid: process_info.dwProcessId,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::atomic::Ordering;

    use tokio::runtime::Handle;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    use super::{
        AdmissionBarrierOutcome, ControlPlaneTransport, EvidenceSource, ExecutionEnvelope,
        FileCommitPolicy, GuestFilesystemPolicy, GuestOsFamily, HelperExecutionEvidence,
        HelperProvisioning, HostArtifactGcConfig, HostGarbageCollector, HostPlatform,
        InflightCommandHandle, InfrastructureRuntimeConfig, JournalTerminalFooter,
        LastGaspPacket, ParentIdentity, PrivilegedCommitMode, QemuSshEnvironment, RemoteConfig,
        ReplayCachePolicy, ResetPolicy, ResetPriorityQuota, SshMultiplexingConfig,
        SshPoolConfig, SshTransportBackend, StagedArtifactLeaseMetadata, TargetKind,
    };

    fn test_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "kria-remote-qemu-test-{}-{}",
            name,
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create test root");
        root
    }

    fn test_remote_config(root: &std::path::Path) -> RemoteConfig {
        let workspace_root = root.join("workspace");
        let staging_root = root.join("staging");
        let control_root = root.join("control");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        std::fs::create_dir_all(&staging_root).expect("create staging root");
        std::fs::create_dir_all(&control_root).expect("create control root");

        RemoteConfig {
            host_platform: HostPlatform::Linux,
            host: "127.0.0.1".to_string(),
            port: 22,
            username: "tester".to_string(),
            ssh_key_path: root.join("id_ed25519"),
            guest_os_family: GuestOsFamily::Posix,
            target_kind: TargetKind::PhysicalRemoteHost,
            qemu_boot_cmd: None,
            qemu_pid_state_file: root.join("qemu.pid"),
            instance_id: format!("test-instance-{}", Uuid::new_v4()),
            remote_control_dir: control_root,
            transport_backend: SshTransportBackend::OpenSshControlMaster,
            ssh_multiplexing: SshMultiplexingConfig {
                enable_control_master: false,
                control_path_cmd: root.join("cmd.sock"),
                control_path_bulk: root.join("bulk.sock"),
                control_persist_secs: 30,
                establish_timeout_ms: 500,
                control_check_timeout_ms: 500,
                allow_no_mux_for_test: true,
                rust_ssh_max_parallel_channels: 8,
            },
            helper_provisioning: HelperProvisioning {
                required_helper_version: "test".to_string(),
                helper_manifest_path: root.join("helper.manifest"),
                helper_manifest_sig_path: root.join("helper.manifest.sig"),
                helper_public_key_path: root.join("helper.pub"),
                host_helper_cache_dir: root.join("helper_cache"),
                remote_helper_dir: root.join("remote_helper"),
                remote_helper_lock_dir: root.join("remote_helper_lock"),
                helper_lock_timeout_ms: 100,
                helper_lock_claim_retry_ms: 10,
                supervisor_heartbeat_interval_ms: 50,
                supervisor_heartbeat_timeout_ms: 200,
                worker_journal_silence_timeout_ms: 200,
                emergency_status_buffer_bytes: 512 * 1024,
                last_gasp_packet_timeout_ms: 100,
                max_helper_rss_bytes: 64 * 1024 * 1024,
            },
            control_transport: ControlPlaneTransport::EphemeralSftpFile,
            envelope_ttl_ms: 1_000,
            max_command_payload_bytes: 4096,
            file_commit_policy: FileCommitPolicy {
                remote_staging_dir: staging_root,
                privileged_commit_mode: PrivilegedCommitMode::Disabled,
                privileged_commit_helper_path: None,
                staging_sweep_ttl_secs: 1,
                staging_lease_heartbeat_timeout_ms: 1,
                staging_sweep_batch_limit: 32,
                enforce_linux_openat2: true,
                privileged_probe_timeout_ms: 200,
                privileged_commit_timeout_ms: 200,
                disable_privileged_on_probe_failure: true,
            },
            guest_filesystem_policy: GuestFilesystemPolicy {
                require_control_dir_writable: true,
                require_staging_dir_writable: true,
                require_non_readonly_mount: true,
                min_free_bytes_floor: 64 * 1024 * 1024,
            },
            reset_policy: ResetPolicy {
                admission_freeze_timeout_ms: 1,
                zombie_reap_timeout_ms: 1,
                lock_acquire_timeout_ms: 50,
                network_call_timeout_ms: 250,
                total_reset_deadline_ms: 5_000,
            },
            replay_cache_policy: ReplayCachePolicy {
                retained_epoch_buckets: 2,
                max_nonces_per_epoch: 128,
            },
            ssh_pool: SshPoolConfig {
                max_active_targets_hard_cap: 8,
                idle_ttl_secs: 30,
                sweep_interval_secs: 30,
                fd_soft_limit: 4096,
                fd_reserve: 64,
                fd_per_command_budget: 4,
                fd_telemetry_sample_ms: 100,
            },
            host_artifact_gc: HostArtifactGcConfig {
                enable_gc: true,
                gc_ttl_secs: 60,
                state_root_dir: root.join("state"),
                host_binary_sha256_or_build_id: "test-binary".to_string(),
            },
            infrastructure_runtime: InfrastructureRuntimeConfig {
                infra_worker_threads: 2,
                high_priority_queue_capacity: 16,
                medium_priority_queue_capacity: 16,
                low_priority_queue_capacity: 16,
                infra_spawn_timeout_ms: 500,
            },
            ssh_connect_timeout_ms: 500,
            command_timeout_ms: 500,
            boot_wait_timeout_ms: 500,
            poll_interval_ms: 10,
            shutdown_timeout_ms: 500,
            soft_reset_grace_ms: 50,
            soft_reset_kill_timeout_ms: 50,
            max_soft_reset_attempts: 2,
            inflight_drain_timeout_ms: 100,
            local_cancel_kill_timeout_ms: 100,
            max_stdout_bytes: 1024 * 1024,
            max_stderr_bytes: 1024 * 1024,
            max_read_file_bytes: 1024 * 1024,
            command_timeout_requires_reset: true,
            known_hosts_path: None,
            strict_host_key_checking: false,
            pinned_host_key_sha256: None,
            remote_workspace_root: Some(workspace_root),
        }
    }

    fn staged_metadata(now_unix_ms: u64) -> StagedArtifactLeaseMetadata {
        StagedArtifactLeaseMetadata {
            owner_instance_id: "instance-a".to_string(),
            owner_pid: None,
            owner_pid_start_time_ticks: None,
            owner_binary_sha256_or_build_id: None,
            generation: 1,
            epoch_uuid: Uuid::new_v4(),
            artifact_nonce: "artifact-nonce".to_string(),
            created_unix_ms: now_unix_ms,
            lease_heartbeat_unix_ms: now_unix_ms,
            expected_sha256: "abc123".to_string(),
            bytes: 16,
        }
    }

    fn parent_identity(pid: u32, start_time_ticks: u64, nonce: &str) -> ParentIdentity {
        ParentIdentity {
            pid,
            start_time_ticks,
            session_nonce: nonce.to_string(),
        }
    }

    fn evidence_envelope(command_id: &str) -> ExecutionEnvelope {
        ExecutionEnvelope {
            command_id: command_id.to_string(),
            generation: 7,
            epoch_uuid: Uuid::new_v4().into_bytes(),
            transport_generation_id: 13,
            instance_id: "instance-test".to_string(),
            issued_at_host_unix_ms_info_only: 1,
            ttl_ms_from_receipt: 500,
            nonce: "nonce-1".to_string(),
            parent_session_nonce: "nonce-parent".to_string(),
            parent_ssh_session_pid: Some(1234),
            parent_ssh_session_start_time_ticks: Some(5678),
            cwd: "/tmp".to_string(),
            env: Default::default(),
            program: "echo".to_string(),
            args: vec!["hello".to_string()],
            command_sha256: "sha256-test".to_string(),
            stdin_mode: "none".to_string(),
        }
    }

    #[test]
    fn triple_check_identity_accepts_exact_match() {
        let expected = parent_identity(1234, 5678, "nonce-a");
        let observed = parent_identity(1234, 5678, "nonce-a");
        assert!(QemuSshEnvironment::validate_parent_identity_triple(
            &expected, &observed
        ));
    }

    #[test]
    fn triple_check_identity_rejects_pid_reuse() {
        let expected = parent_identity(1234, 5678, "nonce-a");
        let observed = parent_identity(4321, 5678, "nonce-a");
        assert!(!QemuSshEnvironment::validate_parent_identity_triple(
            &expected, &observed
        ));
    }

    #[test]
    fn triple_check_identity_rejects_start_time_mismatch() {
        let expected = parent_identity(1234, 5678, "nonce-a");
        let observed = parent_identity(1234, 9999, "nonce-a");
        assert!(!QemuSshEnvironment::validate_parent_identity_triple(
            &expected, &observed
        ));
    }

    #[test]
    fn triple_check_identity_rejects_session_nonce_mismatch() {
        let expected = parent_identity(1234, 5678, "nonce-a");
        let observed = parent_identity(1234, 5678, "nonce-b");
        assert!(!QemuSshEnvironment::validate_parent_identity_triple(
            &expected, &observed
        ));
    }

    #[test]
    fn host_gc_binary_fingerprint_triple_check_rejects_mismatch() {
        assert!(HostGarbageCollector::validate_owner_triple_with_fingerprint(
            Some(100),
            Some(200),
            Some("build-a"),
            100,
            200,
            "build-a",
        ));

        assert!(!HostGarbageCollector::validate_owner_triple_with_fingerprint(
            Some(100),
            Some(200),
            Some("build-a"),
            100,
            200,
            "build-b",
        ));
    }

    #[test]
    fn high_to_medium_priority_fairness_every_third_high() {
        let mut quota = ResetPriorityQuota::default();
        let observed = (0..7)
            .map(|_| quota.on_high_recovery_step())
            .collect::<Vec<_>>();

        assert_eq!(
            observed,
            vec![false, false, true, false, false, true, false]
        );
    }

    #[tokio::test]
    async fn reserved_reset_slots_preserve_high_priority_capacity() {
        let root = test_root("reserved-reset-slots");
        let mut config = test_remote_config(&root);
        config.infrastructure_runtime.high_priority_queue_capacity = 10;
        let handle = Handle::current();
        let env = QemuSshEnvironment::new(config, handle.clone(), handle)
            .expect("construct qemu ssh environment");

        let mut guards = Vec::new();
        for _ in 0..7 {
            guards.push(
                env.acquire_infra_slot("cancel_inflight::flood")
                    .expect("high-non-reset slot should be available"),
            );
        }

        assert!(env.acquire_infra_slot("cancel_inflight::flood").is_err());
        assert!(
            env.acquire_infra_slot("reset_environment::admission_barrier")
                .is_ok()
        );

        drop(guards);
    }

    #[test]
    fn journal_footer_is_authoritative_over_last_gasp() {
        let envelope = evidence_envelope("cmd-journal-authority");
        let last_gasp = serde_json::to_string(&LastGaspPacket {
            command_id: envelope.command_id.clone(),
            generation: envelope.generation,
            epoch_uuid: envelope.epoch_uuid,
            nonce: envelope.nonce.clone(),
            terminal_state: "Exited".to_string(),
            exit_code_or_signal: 0,
            last_error: String::new(),
            stdout: "last-gasp-stdout".to_string(),
            stderr: String::new(),
        })
        .expect("serialize last-gasp packet");

        let resolved = QemuSshEnvironment::resolve_terminal_evidence(
            &envelope,
            HelperExecutionEvidence {
                journal_footer: Some(JournalTerminalFooter {
                    command_id: envelope.command_id.clone(),
                    generation: envelope.generation,
                    epoch_uuid: envelope.epoch_uuid,
                    nonce: envelope.nonce.clone(),
                    exit_code: 42,
                    stdout: "journal-stdout".to_string(),
                    stderr: "journal-stderr".to_string(),
                    journal_complete: true,
                }),
                last_gasp_packet_raw: Some(last_gasp),
            },
        )
        .expect("resolve evidence");

        assert_eq!(resolved.source, EvidenceSource::Journal);
        assert_eq!(resolved.exit_code, 42);
        assert_eq!(resolved.stdout, "journal-stdout");
    }

    #[test]
    fn incomplete_journal_falls_back_to_last_gasp() {
        let envelope = evidence_envelope("cmd-last-gasp-fallback");
        let last_gasp = serde_json::to_string(&LastGaspPacket {
            command_id: envelope.command_id.clone(),
            generation: envelope.generation,
            epoch_uuid: envelope.epoch_uuid,
            nonce: envelope.nonce.clone(),
            terminal_state: "Exited".to_string(),
            exit_code_or_signal: 3,
            last_error: "fallback".to_string(),
            stdout: "last-gasp-stdout".to_string(),
            stderr: "last-gasp-stderr".to_string(),
        })
        .expect("serialize last-gasp packet");

        let resolved = QemuSshEnvironment::resolve_terminal_evidence(
            &envelope,
            HelperExecutionEvidence {
                journal_footer: Some(JournalTerminalFooter {
                    command_id: envelope.command_id.clone(),
                    generation: envelope.generation,
                    epoch_uuid: envelope.epoch_uuid,
                    nonce: envelope.nonce.clone(),
                    exit_code: 0,
                    stdout: "journal-stdout".to_string(),
                    stderr: String::new(),
                    journal_complete: false,
                }),
                last_gasp_packet_raw: Some(last_gasp),
            },
        )
        .expect("resolve evidence");

        assert_eq!(resolved.source, EvidenceSource::LastGasp);
        assert_eq!(resolved.exit_code, 3);
        assert_eq!(resolved.stderr, "last-gasp-stderr");
    }

    #[tokio::test]
    async fn zombie_reaping_orphans_handles_after_barrier_timeout() {
        let root = test_root("zombie-reaping");
        let config = test_remote_config(&root);
        let handle = Handle::current();
        let env = QemuSshEnvironment::new(config, handle.clone(), handle)
            .expect("construct qemu ssh environment");

        env.admission_inflight.store(1, Ordering::Release);
        env.inflight_registry.write().await.insert(
            "cmd-zombie".to_string(),
            InflightCommandHandle {
                command_id: "cmd-zombie".to_string(),
                generation: 0,
                epoch_uuid: Uuid::new_v4(),
                transport_generation_id: 0,
                cancel_token: CancellationToken::new(),
                local_process_ids: Vec::new(),
                remote_status_path: root.join("cmd-zombie.status"),
                remote_tmp_paths: HashSet::new(),
                parent_identity: None,
                helper_supervisor_pid: None,
                helper_worker_pid: None,
                helper_worker_start_time_ticks: None,
            },
        );

        let outcome = env
            .wait_for_admission_barrier_or_zombie_reap()
            .await
            .expect("zombie reaping outcome");

        match outcome {
            AdmissionBarrierOutcome::ZombieReaping { orphaned_handles } => {
                assert_eq!(orphaned_handles, 1);
            }
            AdmissionBarrierOutcome::BarrierReached => {
                panic!("expected ZombieReaping outcome");
            }
        }

        assert!(env.inflight_registry.read().await.is_empty());
        assert!(env.zombie_commands.read().await.contains("cmd-zombie"));
        assert_eq!(env.admission_inflight.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn barrier_requires_admission_and_registry_empty() {
        let root = test_root("dual-barrier");
        let config = test_remote_config(&root);
        let handle = Handle::current();
        let env = QemuSshEnvironment::new(config, handle.clone(), handle)
            .expect("construct qemu ssh environment");

        env.admission_inflight.store(0, Ordering::Release);
        env.inflight_registry.write().await.insert(
            "cmd-dual-barrier".to_string(),
            InflightCommandHandle {
                command_id: "cmd-dual-barrier".to_string(),
                generation: 0,
                epoch_uuid: Uuid::new_v4(),
                transport_generation_id: 0,
                cancel_token: CancellationToken::new(),
                local_process_ids: Vec::new(),
                remote_status_path: root.join("cmd-dual-barrier.status"),
                remote_tmp_paths: HashSet::new(),
                parent_identity: None,
                helper_supervisor_pid: None,
                helper_worker_pid: None,
                helper_worker_start_time_ticks: None,
            },
        );

        let outcome = env
            .wait_for_admission_barrier_or_zombie_reap()
            .await
            .expect("dual barrier outcome");

        match outcome {
            AdmissionBarrierOutcome::ZombieReaping { orphaned_handles } => {
                assert_eq!(orphaned_handles, 1);
            }
            AdmissionBarrierOutcome::BarrierReached => {
                panic!("expected ZombieReaping outcome");
            }
        }
    }

    #[tokio::test]
    async fn stale_epoch_or_generation_fence_is_rejected() {
        let root = test_root("stale-fence");
        let config = test_remote_config(&root);
        let handle = Handle::current();
        let env = QemuSshEnvironment::new(config, handle.clone(), handle)
            .expect("construct qemu ssh environment");

        let generation = env.generation.load(Ordering::Acquire);
        let epoch_uuid = env.current_epoch_uuid();
        assert!(env.is_command_fence_current(generation, epoch_uuid));

        env.generation.fetch_add(1, Ordering::AcqRel);
        assert!(!env.is_command_fence_current(generation, epoch_uuid));
    }

    #[test]
    fn global_sweep_predicate_requires_all_three_conditions() {
        let now = 20_000;
        let mut metadata = staged_metadata(1_000);
        metadata.lease_heartbeat_unix_ms = 1_000;

        let ttl_ms = 5_000;
        let heartbeat_timeout_ms = 500;

        assert!(QemuSshEnvironment::should_delete_staged_artifact(
            &metadata,
            now,
            ttl_ms,
            heartbeat_timeout_ms,
            true,
        ));

        assert!(!QemuSshEnvironment::should_delete_staged_artifact(
            &metadata,
            4_000,
            ttl_ms,
            heartbeat_timeout_ms,
            true,
        ));

        assert!(!QemuSshEnvironment::should_delete_staged_artifact(
            &metadata,
            now,
            ttl_ms,
            25_000,
            true,
        ));

        assert!(!QemuSshEnvironment::should_delete_staged_artifact(
            &metadata,
            now,
            ttl_ms,
            heartbeat_timeout_ms,
            false,
        ));
    }
}