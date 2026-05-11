use std::collections::HashMap;
use std::io::{Cursor, ErrorKind, Read};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, DownloadFromContainerOptions,
    InspectContainerOptions, LogOutput, RemoveContainerOptions, StartContainerOptions,
    StopContainerOptions, UploadToContainerOptions,
};
use bollard::errors::Error as BollardError;
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use bollard::models::HostConfig;
use bollard::Docker;
use futures::StreamExt;
use tar::{Archive, Builder, Header};
use tokio::process::Command;
use tokio::sync::{broadcast, Mutex};

use super::traits::{
    CommandExecutor, CommandRequest, CommandResult, EnvironmentError, EnvironmentLifecycle,
    FileSystemOps, ListDirRequest, ListDirResult, ReadFileRequest, ReadFileResult, ResetReason,
    ShellState, WriteFileRequest, WriteFileResult,
};

const UBUNTU_24_04_PINNED_DIGEST: &str =
    "ubuntu:24.04@sha256:cdb5fd928fced577cfecf12c8966e830fcdf42ee481fb0b91904eeddc2fe5eff";
const DEFAULT_NETWORK_NAME: &str = "kria_exec_net";
const DEFAULT_SECCOMP_PROFILE: &str = "config/seccomp/kria-seccomp.json";
const DEFAULT_NET_POLICY_CHECK_SCRIPT: &str = "scripts/setup-kria-net.sh";
const CONTAINER_WORKSPACE_ROOT: &str = "/workspace";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerNetworkPolicyMode {
    PreprovisionedFirewall,
    Internal,
}

impl DockerNetworkPolicyMode {
    fn from_env(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "internal" => Self::Internal,
            _ => Self::PreprovisionedFirewall,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::PreprovisionedFirewall => "preprovisioned_firewall",
            Self::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentResetKind {
    Oom,
    PidLimit,
}

impl EnvironmentResetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Oom => "oom",
            Self::PidLimit => "pid_limit",
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnvironmentResetEvent {
    pub kind: EnvironmentResetKind,
    pub reason: String,
    pub generation: u64,
}

#[derive(Debug, Clone)]
pub struct DockerEnvironmentConfig {
    pub image: String,
    pub network_name: String,
    pub network_mode: String,
    pub network_policy_mode: DockerNetworkPolicyMode,
    pub workspace_tmpfs_size_mb: i64,
    pub memory_mb: i64,
    pub cpus: f64,
    pub pids_limit: i64,
    pub readonly_rootfs: bool,
    pub seccomp_profile: PathBuf,
    pub network_policy_check_script: PathBuf,
    pub container_name_prefix: String,
    pub inject_host_uid_gid: bool,
}

impl Default for DockerEnvironmentConfig {
    fn default() -> Self {
        Self {
            image: UBUNTU_24_04_PINNED_DIGEST.to_string(),
            network_name: DEFAULT_NETWORK_NAME.to_string(),
            network_mode: "bridge".to_string(),
            network_policy_mode: DockerNetworkPolicyMode::PreprovisionedFirewall,
            workspace_tmpfs_size_mb: 256,
            memory_mb: 512,
            cpus: 1.0,
            pids_limit: 128,
            readonly_rootfs: true,
            seccomp_profile: PathBuf::from(DEFAULT_SECCOMP_PROFILE),
            network_policy_check_script: PathBuf::from(DEFAULT_NET_POLICY_CHECK_SCRIPT),
            container_name_prefix: "kria-exec".to_string(),
            inject_host_uid_gid: true,
        }
    }
}

impl DockerEnvironmentConfig {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();

        if let Ok(value) = std::env::var("KRIA_EXEC_DOCKER_IMAGE") {
            if !value.trim().is_empty() {
                cfg.image = value;
            }
        }
        if let Ok(value) = std::env::var("KRIA_EXEC_DOCKER_NETWORK_NAME") {
            if !value.trim().is_empty() {
                cfg.network_name = value;
            }
        }
        if let Ok(value) = std::env::var("KRIA_EXEC_DOCKER_NETWORK_MODE") {
            if !value.trim().is_empty() {
                cfg.network_mode = value;
            }
        }
        if let Ok(value) = std::env::var("KRIA_EXEC_DOCKER_NETWORK_POLICY_MODE") {
            if !value.trim().is_empty() {
                cfg.network_policy_mode = DockerNetworkPolicyMode::from_env(&value);
            }
        }
        if let Ok(value) = std::env::var("KRIA_EXEC_DOCKER_WORKSPACE_TMPFS_SIZE_MB") {
            if let Ok(parsed) = value.parse::<i64>() {
                cfg.workspace_tmpfs_size_mb = parsed.max(16);
            }
        }
        if let Ok(value) = std::env::var("KRIA_EXEC_DOCKER_MEMORY_MB") {
            if let Ok(parsed) = value.parse::<i64>() {
                cfg.memory_mb = parsed.max(64);
            }
        }
        if let Ok(value) = std::env::var("KRIA_EXEC_DOCKER_CPUS") {
            if let Ok(parsed) = value.parse::<f64>() {
                cfg.cpus = parsed.max(0.1);
            }
        }
        if let Ok(value) = std::env::var("KRIA_EXEC_DOCKER_PIDS_LIMIT") {
            if let Ok(parsed) = value.parse::<i64>() {
                cfg.pids_limit = parsed.max(16);
            }
        }
        if let Ok(value) = std::env::var("KRIA_EXEC_DOCKER_READONLY_ROOTFS") {
            let normalized = value.trim().to_ascii_lowercase();
            cfg.readonly_rootfs = !matches!(normalized.as_str(), "0" | "false" | "no" | "off");
        }
        if let Ok(value) = std::env::var("KRIA_EXEC_DOCKER_SECCOMP_PROFILE") {
            if !value.trim().is_empty() {
                cfg.seccomp_profile = PathBuf::from(value);
            }
        }
        if let Ok(value) = std::env::var("KRIA_EXEC_DOCKER_NETWORK_POLICY_CHECK_SCRIPT") {
            if !value.trim().is_empty() {
                cfg.network_policy_check_script = PathBuf::from(value);
            }
        }

        cfg
    }
}

#[derive(Debug, Clone)]
enum LifecycleState {
    Healthy,
    Busy,
    Broken,
    Recreating,
}

#[derive(Debug, Clone)]
struct LifecycleActor {
    state: LifecycleState,
    container_id: Option<String>,
    reset_generation: u64,
}

impl Default for LifecycleActor {
    fn default() -> Self {
        Self {
            state: LifecycleState::Broken,
            container_id: None,
            reset_generation: 0,
        }
    }
}

#[derive(Debug)]
struct ContainerStatus {
    running: bool,
    oom_killed: bool,
}

#[derive(Debug)]
pub struct DockerEnvironment {
    docker: Docker,
    config: DockerEnvironmentConfig,
    lifecycle: Mutex<LifecycleActor>,
    reset_tx: broadcast::Sender<EnvironmentResetEvent>,
}

impl DockerEnvironment {
    pub fn new(config: DockerEnvironmentConfig) -> Result<Self, EnvironmentError> {
        let docker = Docker::connect_with_local_defaults().map_err(|error| {
            EnvironmentError::ProviderUnavailable {
                provider: "docker".to_string(),
                details: error.to_string(),
            }
        })?;
        let (reset_tx, _) = broadcast::channel(64);

        Ok(Self {
            docker,
            config,
            lifecycle: Mutex::new(LifecycleActor::default()),
            reset_tx,
        })
    }

    pub fn from_env() -> Result<Self, EnvironmentError> {
        Self::new(DockerEnvironmentConfig::from_env())
    }

    pub fn subscribe_environment_resets(&self) -> broadcast::Receiver<EnvironmentResetEvent> {
        self.reset_tx.subscribe()
    }

    fn normalize_workspace_path(&self, raw_path: &Path) -> Result<PathBuf, EnvironmentError> {
        let mut relative = PathBuf::new();

        for component in raw_path.components() {
            match component {
                Component::Normal(segment) => relative.push(segment),
                Component::ParentDir => {
                    if !relative.pop() {
                        return Err(EnvironmentError::PathTraversalDenied {
                            path: raw_path.display().to_string(),
                        });
                    }
                }
                Component::CurDir | Component::RootDir => {}
                Component::Prefix(_) => {
                    return Err(EnvironmentError::PathTraversalDenied {
                        path: raw_path.display().to_string(),
                    });
                }
            }
        }

        let mut out = PathBuf::from(CONTAINER_WORKSPACE_ROOT);
        if !relative.as_os_str().is_empty() {
            out.push(relative);
        }
        Ok(out)
    }

    fn normalize_workspace_cwd(&self, raw_cwd: &Path) -> PathBuf {
        match self.normalize_workspace_path(raw_cwd) {
            Ok(path) => path,
            Err(_) => PathBuf::from(CONTAINER_WORKSPACE_ROOT),
        }
    }

    fn build_exec_env(&self, shell_state_snapshot: &ShellState) -> Vec<String> {
        let mut env = shell_state_snapshot
            .env_vars
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>();
        env.push("DEBIAN_FRONTEND=noninteractive".to_string());
        env
    }

    fn seccomp_profile_path(&self) -> Result<PathBuf, EnvironmentError> {
        if self.config.seccomp_profile.is_absolute() {
            return Ok(self.config.seccomp_profile.clone());
        }

        let cwd = std::env::current_dir().map_err(|error| EnvironmentError::Io {
            operation: "resolve_seccomp_profile_cwd".to_string(),
            details: error.to_string(),
        })?;
        Ok(cwd.join(&self.config.seccomp_profile))
    }

    fn seccomp_profile_json(&self) -> Result<String, EnvironmentError> {
        let seccomp_profile = self.seccomp_profile_path()?;

        match std::fs::read_to_string(&seccomp_profile) {
            Ok(json_string) => Ok(json_string),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                Err(EnvironmentError::StartupPolicyNotReady {
                    policy: "seccomp_profile".to_string(),
                    details: format!("missing profile at {}", seccomp_profile.display()),
                })
            }
            Err(error) => Err(EnvironmentError::Io {
                operation: "read_seccomp_profile".to_string(),
                details: format!("{} ({})", error, seccomp_profile.display()),
            }),
        }
    }

    fn network_policy_script_path(&self) -> Result<PathBuf, EnvironmentError> {
        if self.config.network_policy_check_script.is_absolute() {
            return Ok(self.config.network_policy_check_script.clone());
        }

        let cwd = std::env::current_dir().map_err(|error| EnvironmentError::Io {
            operation: "resolve_network_policy_script_cwd".to_string(),
            details: error.to_string(),
        })?;
        Ok(cwd.join(&self.config.network_policy_check_script))
    }

    fn now_suffix() -> String {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        format!("{}-{}", ts.as_secs(), ts.subsec_nanos())
    }

    fn map_docker_error(&self, operation: &str, error: BollardError) -> EnvironmentError {
        EnvironmentError::DockerApi {
            operation: operation.to_string(),
            details: error.to_string(),
        }
    }

    async fn ensure_startup_preconditions(&self) -> Result<(), EnvironmentError> {
        if self.config.image != UBUNTU_24_04_PINNED_DIGEST {
            return Err(EnvironmentError::StartupPolicyNotReady {
                policy: "immutable_image_digest".to_string(),
                details: format!(
                    "expected image '{}', got '{}'",
                    UBUNTU_24_04_PINNED_DIGEST, self.config.image
                ),
            });
        }

        let seccomp_profile = self.seccomp_profile_path()?;
        if !seccomp_profile.exists() {
            return Err(EnvironmentError::StartupPolicyNotReady {
                policy: "seccomp_profile".to_string(),
                details: format!("missing profile at {}", seccomp_profile.display()),
            });
        }

        self.docker
            .ping()
            .await
            .map_err(|error| EnvironmentError::ProviderUnavailable {
                provider: "docker".to_string(),
                details: error.to_string(),
            })?;

        let bridge_mode = self.config.network_mode.eq_ignore_ascii_case("bridge");
        if bridge_mode
            && self.config.network_policy_mode == DockerNetworkPolicyMode::PreprovisionedFirewall
        {
            let script_path = self.network_policy_script_path()?;
            if !script_path.exists() {
                return Err(EnvironmentError::NetworkPolicyNotReady {
                    mode: self.config.network_policy_mode.as_str().to_string(),
                    details: format!("network policy script missing at {}", script_path.display()),
                });
            }

            let output = Command::new(&script_path)
                .arg("--check")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .await
                .map_err(|error| EnvironmentError::Io {
                    operation: "network_policy_check".to_string(),
                    details: error.to_string(),
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(EnvironmentError::NetworkPolicyNotReady {
                    mode: self.config.network_policy_mode.as_str().to_string(),
                    details: if stderr.is_empty() {
                        "setup-kria-net.sh --check returned non-zero".to_string()
                    } else {
                        stderr
                    },
                });
            }
        }

        Ok(())
    }

    async fn inspect_container_status(
        &self,
        container_id: &str,
    ) -> Result<ContainerStatus, EnvironmentError> {
        let inspect = self
            .docker
            .inspect_container(container_id, None::<InspectContainerOptions>)
            .await
            .map_err(|error| self.map_docker_error("inspect_container", error))?;

        let state = inspect.state;
        let running = state.as_ref().and_then(|s| s.running).unwrap_or(false);
        let oom_killed = state.as_ref().and_then(|s| s.oom_killed).unwrap_or(false);
        Ok(ContainerStatus {
            running,
            oom_killed,
        })
    }

    fn effective_network_mode(&self) -> String {
        if self.config.network_policy_mode == DockerNetworkPolicyMode::Internal {
            return "none".to_string();
        }

        if self.config.network_mode.trim().is_empty() {
            "bridge".to_string()
        } else {
            self.config.network_mode.clone()
        }
    }

    fn host_uid_gid(&self) -> Option<String> {
        if !self.config.inject_host_uid_gid {
            return None;
        }

        #[cfg(unix)]
        {
            let uid = nix_uid();
            let gid = nix_gid();
            Some(format!("{uid}:{gid}"))
        }

        #[cfg(not(unix))]
        {
            None
        }
    }

    async fn create_hardened_container(&self) -> Result<String, EnvironmentError> {
        let seccomp_json = self.seccomp_profile_json()?;
        let mut tmpfs = HashMap::new();
        tmpfs.insert(
            CONTAINER_WORKSPACE_ROOT.to_string(),
            format!(
                "rw,nosuid,nodev,noexec,size={}m",
                self.config.workspace_tmpfs_size_mb
            ),
        );

        let host_config = HostConfig {
            cap_drop: Some(vec!["ALL".to_string()]),
            security_opt: Some(vec![
                format!("seccomp={}", seccomp_json),
                "no-new-privileges".into(),
            ]),
            memory: Some(self.config.memory_mb * 1024 * 1024),
            nano_cpus: Some((self.config.cpus * 1_000_000_000_f64) as i64),
            pids_limit: Some(self.config.pids_limit),
            tmpfs: Some(tmpfs),
            network_mode: Some(self.effective_network_mode()),
            readonly_rootfs: Some(self.config.readonly_rootfs),
            ..Default::default()
        };

        let mut env = vec!["DEBIAN_FRONTEND=noninteractive".to_string()];
        if let Some(user) = self.host_uid_gid() {
            env.push(format!("KRIA_HOST_UID_GID={user}"));
        }

        let container_name = format!(
            "{}-{}",
            self.config.container_name_prefix,
            Self::now_suffix()
        );
        let create_options = CreateContainerOptions {
            name: container_name,
            platform: None,
        };

        let config = ContainerConfig {
            image: Some(self.config.image.clone()),
            host_config: Some(host_config),
            env: Some(env),
            working_dir: Some(CONTAINER_WORKSPACE_ROOT.to_string()),
            cmd: Some(vec![
                "/bin/sh".to_string(),
                "-lc".to_string(),
                "trap : TERM INT; while :; do sleep 3600; done".to_string(),
            ]),
            open_stdin: Some(false),
            attach_stdin: Some(false),
            attach_stdout: Some(false),
            attach_stderr: Some(false),
            tty: Some(false),
            user: self.host_uid_gid(),
            ..Default::default()
        };

        let response = self
            .docker
            .create_container(Some(create_options), config)
            .await
            .map_err(|error| self.map_docker_error("create_container", error))?;

        self.docker
            .start_container(&response.id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|error| self.map_docker_error("start_container", error))?;

        Ok(response.id)
    }

    async fn remove_container_if_exists(&self, container_id: &str) -> Result<(), EnvironmentError> {
        let _ = self
            .docker
            .stop_container(container_id, None::<StopContainerOptions>)
            .await;

        self.docker
            .remove_container(
                container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|error| self.map_docker_error("remove_container", error))?;

        Ok(())
    }

    async fn recycle_container(
        &self,
        kind: Option<EnvironmentResetKind>,
        reason: impl Into<String>,
    ) -> Result<String, EnvironmentError> {
        let reason = reason.into();

        {
            let mut lifecycle = self.lifecycle.lock().await;
            lifecycle.state = LifecycleState::Recreating;
        }

        let old_id = {
            let lifecycle = self.lifecycle.lock().await;
            lifecycle.container_id.clone()
        };

        if let Some(id) = old_id {
            let _ = self.remove_container_if_exists(&id).await;
        }

        let new_id = self.create_hardened_container().await?;

        let generation = {
            let mut lifecycle = self.lifecycle.lock().await;
            lifecycle.container_id = Some(new_id.clone());
            lifecycle.state = LifecycleState::Healthy;
            lifecycle.reset_generation = lifecycle.reset_generation.saturating_add(1);
            lifecycle.reset_generation
        };

        if let Some(kind) = kind {
            let _ = self.reset_tx.send(EnvironmentResetEvent {
                kind,
                reason,
                generation,
            });
        }

        Ok(new_id)
    }

    async fn ensure_container_id(&self) -> Result<String, EnvironmentError> {
        self.ensure_startup_preconditions().await?;

        let existing = {
            let lifecycle = self.lifecycle.lock().await;
            lifecycle.container_id.clone()
        };

        if let Some(container_id) = existing {
            match self.inspect_container_status(&container_id).await {
                Ok(status) if status.running => {
                    let mut lifecycle = self.lifecycle.lock().await;
                    lifecycle.state = LifecycleState::Healthy;
                    return Ok(container_id);
                }
                Ok(status) if status.oom_killed => {
                    return self
                        .recycle_container(
                            Some(EnvironmentResetKind::Oom),
                            "container reported OOMKilled while ensuring readiness",
                        )
                        .await;
                }
                Ok(_) => {
                    return self
                        .recycle_container(None, "container was not running during ensure_ready")
                        .await;
                }
                Err(error) => {
                    let mut lifecycle = self.lifecycle.lock().await;
                    lifecycle.state = LifecycleState::Broken;
                    return Err(error);
                }
            }
        }

        self.recycle_container(None, "initial container create")
            .await
    }

    async fn execute_in_container(
        &self,
        container_id: &str,
        request: CommandRequest,
        shell_state_snapshot: ShellState,
    ) -> Result<CommandResult, EnvironmentError> {
        let mut command = vec![request.program.clone()];
        command.extend(request.args.iter().cloned());

        let working_dir = self
            .normalize_workspace_cwd(&shell_state_snapshot.cwd)
            .to_string_lossy()
            .to_string();

        let create_exec = CreateExecOptions {
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            attach_stdin: Some(false),
            tty: Some(false),
            cmd: Some(command),
            env: Some(self.build_exec_env(&shell_state_snapshot)),
            working_dir: Some(working_dir),
            user: self.host_uid_gid(),
            ..Default::default()
        };

        let exec = self
            .docker
            .create_exec(container_id, create_exec)
            .await
            .map_err(|error| {
                let details = error.to_string();
                if looks_like_pid_limit_error(&details) {
                    EnvironmentError::PidLimitExceeded {
                        limit: self.config.pids_limit,
                    }
                } else {
                    self.map_docker_error("create_exec", error)
                }
            })?;

        let timeout = Duration::from_millis(request.timeout_ms);
        let start_result = self
            .docker
            .start_exec(&exec.id, None::<StartExecOptions>)
            .await
            .map_err(|error| self.map_docker_error("start_exec", error))?;

        let (stdout, stderr, truncated, observed_bytes, observed_lines) = match start_result {
            StartExecResults::Attached { mut output, .. } => {
                let collect = async {
                    let mut stdout = Vec::<u8>::new();
                    let mut stderr = Vec::<u8>::new();
                    let mut observed_bytes = 0usize;
                    let mut observed_lines = 0usize;

                    while let Some(item) = output.next().await {
                        let log = item
                            .map_err(|error| self.map_docker_error("start_exec_stream", error))?;
                        let (message, write_to_stderr) = match log {
                            LogOutput::StdOut { message } => (message, false),
                            LogOutput::StdErr { message } => (message, true),
                            LogOutput::Console { message } => (message, false),
                            LogOutput::StdIn { message } => (message, false),
                        };

                        observed_bytes = observed_bytes.saturating_add(message.len());
                        observed_lines = observed_lines
                            .saturating_add(message.iter().filter(|&&byte| byte == b'\n').count());

                        if observed_bytes > request.max_bytes || observed_lines > request.max_lines
                        {
                            return Err(EnvironmentError::OutputLimitExceeded {
                                max_bytes: request.max_bytes,
                                max_lines: request.max_lines,
                                observed_bytes,
                                observed_lines,
                            });
                        }

                        if write_to_stderr {
                            stderr.extend_from_slice(&message);
                        } else {
                            stdout.extend_from_slice(&message);
                        }
                    }

                    Ok((stdout, stderr, false, observed_bytes, observed_lines))
                };

                match tokio::time::timeout(timeout, collect).await {
                    Ok(result) => result?,
                    Err(_) => {
                        self.recycle_container(None, "exec timeout recycle").await?;
                        return Err(EnvironmentError::CommandTimedOut {
                            timeout_ms: request.timeout_ms,
                        });
                    }
                }
            }
            StartExecResults::Detached => {
                return Err(EnvironmentError::DockerApi {
                    operation: "start_exec".to_string(),
                    details: "received detached exec result in attached mode".to_string(),
                })
            }
        };

        let inspect = self
            .docker
            .inspect_exec(&exec.id)
            .await
            .map_err(|error| self.map_docker_error("inspect_exec", error))?;

        let exit_code = inspect.exit_code.unwrap_or_default() as i32;
        let stdout_text = String::from_utf8_lossy(&stdout).to_string();
        let stderr_text = String::from_utf8_lossy(&stderr).to_string();

        if exit_code != 0 {
            let container_status = self.inspect_container_status(container_id).await?;
            if container_status.oom_killed || exit_code == 137 {
                self.recycle_container(
                    Some(EnvironmentResetKind::Oom),
                    format!("container recycled after OOM-style exit {exit_code}"),
                )
                .await?;
                return Err(EnvironmentError::ContainerOomKilled {
                    exit_code: Some(exit_code as i64),
                });
            }

            if looks_like_pid_limit_error(&stderr_text) {
                self.recycle_container(
                    Some(EnvironmentResetKind::PidLimit),
                    "container recycled after PID limit breach",
                )
                .await?;
                return Err(EnvironmentError::PidLimitExceeded {
                    limit: self.config.pids_limit,
                });
            }

            if stderr_text
                .to_ascii_lowercase()
                .contains("no space left on device")
            {
                return Err(EnvironmentError::StorageLimitExceeded {
                    limit_bytes: (self.config.workspace_tmpfs_size_mb * 1024 * 1024) as u64,
                    observed_bytes: observed_bytes as u64,
                });
            }

            return Err(EnvironmentError::CommandFailed {
                exit_code,
                stderr: stderr_text,
            });
        }

        Ok(CommandResult {
            exit_code,
            stdout: stdout_text,
            stderr: stderr_text,
            truncated: truncated
                || observed_bytes >= request.max_bytes
                || observed_lines >= request.max_lines,
        })
    }

    async fn run_internal_command(
        &self,
        container_id: &str,
        program: &str,
        args: Vec<String>,
    ) -> Result<CommandResult, EnvironmentError> {
        self.execute_in_container(
            container_id,
            CommandRequest {
                program: program.to_string(),
                args,
                timeout_ms: 15_000,
                max_bytes: 512 * 1024,
                max_lines: 16_000,
            },
            ShellState {
                cwd: PathBuf::from(CONTAINER_WORKSPACE_ROOT),
                env_vars: HashMap::new(),
                generation: 0,
            },
        )
        .await
    }

    fn tar_single_file(entry_name: &str, contents: &[u8]) -> Result<Vec<u8>, EnvironmentError> {
        let mut builder = Builder::new(Vec::<u8>::new());
        let mut header = Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, entry_name, Cursor::new(contents))
            .map_err(|error| EnvironmentError::Io {
                operation: "tar_append_data".to_string(),
                details: error.to_string(),
            })?;
        builder.finish().map_err(|error| EnvironmentError::Io {
            operation: "tar_finish".to_string(),
            details: error.to_string(),
        })?;
        builder.into_inner().map_err(|error| EnvironmentError::Io {
            operation: "tar_into_inner".to_string(),
            details: error.to_string(),
        })
    }

    fn untar_single_file(archive_bytes: &[u8]) -> Result<Vec<u8>, EnvironmentError> {
        let mut archive = Archive::new(Cursor::new(archive_bytes));
        let entries = archive.entries().map_err(|error| EnvironmentError::Io {
            operation: "untar_entries".to_string(),
            details: error.to_string(),
        })?;

        for entry_result in entries {
            let mut entry = entry_result.map_err(|error| EnvironmentError::Io {
                operation: "untar_next_entry".to_string(),
                details: error.to_string(),
            })?;

            if entry.header().entry_type().is_dir() {
                continue;
            }

            let mut contents = Vec::new();
            entry
                .read_to_end(&mut contents)
                .map_err(|error| EnvironmentError::Io {
                    operation: "untar_read_entry".to_string(),
                    details: error.to_string(),
                })?;
            return Ok(contents);
        }

        Err(EnvironmentError::Io {
            operation: "untar_single_file".to_string(),
            details: "archive did not contain a file entry".to_string(),
        })
    }
}

#[async_trait]
impl CommandExecutor for DockerEnvironment {
    async fn execute_command(
        &self,
        request: CommandRequest,
        shell_state_snapshot: ShellState,
    ) -> Result<CommandResult, EnvironmentError> {
        let container_id = self.ensure_container_id().await?;
        self.execute_in_container(&container_id, request, shell_state_snapshot)
            .await
    }
}

#[async_trait]
impl FileSystemOps for DockerEnvironment {
    async fn read_file(
        &self,
        request: ReadFileRequest,
    ) -> Result<ReadFileResult, EnvironmentError> {
        let container_id = self.ensure_container_id().await?;
        let path = self.normalize_workspace_path(&request.path)?;

        let mut stream = self.docker.download_from_container(
            &container_id,
            Some(DownloadFromContainerOptions {
                path: path.to_string_lossy().to_string(),
            }),
        );

        let mut archive_bytes = Vec::new();
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|error| self.map_docker_error("download_from_container", error))?;
            archive_bytes.extend_from_slice(&chunk);
        }

        let contents = Self::untar_single_file(&archive_bytes)?;
        Ok(ReadFileResult { contents })
    }

    async fn write_file(
        &self,
        request: WriteFileRequest,
    ) -> Result<WriteFileResult, EnvironmentError> {
        let container_id = self.ensure_container_id().await?;
        let path = self.normalize_workspace_path(&request.path)?;
        let parent = path
            .parent()
            .unwrap_or_else(|| Path::new(CONTAINER_WORKSPACE_ROOT));
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());

        if request.create_parent {
            let _ = self
                .run_internal_command(
                    &container_id,
                    "/bin/mkdir",
                    vec!["-p".to_string(), parent.to_string_lossy().to_string()],
                )
                .await?;
        }

        let tar_bytes = Self::tar_single_file(&file_name, &request.contents)?;

        self.docker
            .upload_to_container(
                &container_id,
                Some(UploadToContainerOptions {
                    path: parent.to_string_lossy().to_string(),
                    no_overwrite_dir_non_dir: "false".to_string(),
                }),
                tar_bytes.into(),
            )
            .await
            .map_err(|error| self.map_docker_error("upload_to_container", error))?;

        Ok(WriteFileResult {
            bytes_written: request.contents.len(),
        })
    }

    async fn list_dir(&self, request: ListDirRequest) -> Result<ListDirResult, EnvironmentError> {
        let container_id = self.ensure_container_id().await?;
        let path = self.normalize_workspace_path(&request.path)?;

        let result = self
            .run_internal_command(
                &container_id,
                "/usr/bin/find",
                vec![
                    path.to_string_lossy().to_string(),
                    "-mindepth".to_string(),
                    "1".to_string(),
                    "-maxdepth".to_string(),
                    "1".to_string(),
                    "-print".to_string(),
                ],
            )
            .await?;

        let mut entries = result
            .stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        entries.sort();

        Ok(ListDirResult { entries })
    }
}

#[async_trait]
impl EnvironmentLifecycle for DockerEnvironment {
    async fn ensure_ready(&self) -> Result<(), EnvironmentError> {
        let _ = self.ensure_container_id().await?;
        Ok(())
    }

    async fn reset_environment(&self, reason: ResetReason) -> Result<(), EnvironmentError> {
        let kind = match reason {
            ResetReason::ResourceExhaustion => Some(EnvironmentResetKind::Oom),
            _ => None,
        };
        let _ = self
            .recycle_container(kind, format!("manual reset requested: {reason:?}"))
            .await?;
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), EnvironmentError> {
        let container_id = {
            let mut lifecycle = self.lifecycle.lock().await;
            lifecycle.state = LifecycleState::Busy;
            lifecycle.container_id.take()
        };

        if let Some(id) = container_id {
            self.remove_container_if_exists(&id).await?;
        }

        let mut lifecycle = self.lifecycle.lock().await;
        lifecycle.state = LifecycleState::Broken;
        Ok(())
    }
}

fn looks_like_pid_limit_error(details: &str) -> bool {
    let lower = details.to_ascii_lowercase();
    lower.contains("pids")
        || lower.contains("process limit")
        || lower.contains("cannot fork")
        || lower.contains("resource temporarily unavailable")
}

#[cfg(unix)]
fn nix_uid() -> u32 {
    // SAFETY: libc getter has no preconditions.
    unsafe { libc::geteuid() as u32 }
}

#[cfg(unix)]
fn nix_gid() -> u32 {
    // SAFETY: libc getter has no preconditions.
    unsafe { libc::getegid() as u32 }
}
