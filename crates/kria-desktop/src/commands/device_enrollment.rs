use super::*;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewTargetRequest {
    pub display_name: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub ssh_private_key_path: Option<String>,
    pub expected_hostkey_sha256: Option<String>,
    #[serde(alias = "commanderEpoch")]
    pub controller_epoch: Option<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct NormalizedNewTargetRequest {
    pub(crate) display_name: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: String,
    pub(crate) ssh_private_key_path: PathBuf,
    pub(crate) expected_hostkey_sha256_b64: Option<String>,
    pub(crate) controller_epoch: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterNewTargetResponse {
    pub target_id: String,
    pub display_name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub mode: String,
    pub ssh_hostkey_sha256_b64: String,
    pub ssh_private_key_path: String,
    pub ssh_public_key_path: String,
    pub controller_epoch: i64,
    pub created_new_target: bool,
    pub created_local_key: bool,
    pub enrolled_at_unix_ms: i64,
    pub registry_path: String,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegisterNewTargetErrorCode {
    ValidationFailed,
    ConnectionRefused,
    AuthenticationFailed,
    HostKeyChanged,
    DependencyMissing,
    BootstrapFailed,
    PersistenceFailed,
    Unknown,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterNewTargetError {
    pub code: RegisterNewTargetErrorCode,
    pub message: String,
    pub detail: Option<String>,
}

impl RegisterNewTargetError {
    pub(crate) fn new(
        code: RegisterNewTargetErrorCode,
        message: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            detail,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FleetEnrollmentRegistry {
    pub(crate) schema_version: u32,
    pub(crate) targets: Vec<EnrolledTargetRecord>,
}

impl Default for FleetEnrollmentRegistry {
    fn default() -> Self {
        Self {
            schema_version: 1,
            targets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnrolledTargetRecord {
    pub(crate) target_id: String,
    pub(crate) display_name: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: String,
    pub(crate) mode: String,
    pub(crate) ssh_private_key_path: String,
    pub(crate) ssh_public_key_path: String,
    pub(crate) ssh_hostkey_sha256_b64: String,
    #[serde(alias = "commanderEpoch")]
    pub(crate) controller_epoch: i64,
    pub(crate) enrolled_at_unix_ms: i64,
    pub(crate) last_verified_unix_ms: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct EnrolledTargetStatusSnapshot {
    pub(crate) target_id: String,
    pub(crate) display_name: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: String,
    pub(crate) mode: String,
    pub(crate) ssh_hostkey_sha256_b64: String,
    pub(crate) controller_epoch: i64,
    pub(crate) enrolled_at_unix_ms: i64,
    pub(crate) last_verified_unix_ms: i64,
}

#[derive(Debug)]
pub(crate) struct CommandRunOutput {
    pub(crate) status: std::process::ExitStatus,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) struct TempFileGuard {
    path: PathBuf,
}

impl TempFileGuard {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        self.path.as_path()
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) fn truncate_for_error(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...[truncated]");
            break;
        }
        out.push(ch);
    }
    out
}

fn expand_tilde_path(raw_path: &str) -> PathBuf {
    let trimmed = raw_path.trim();
    if trimmed == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    }

    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }

    PathBuf::from(trimmed)
}

fn normalize_hostkey_sha256_b64(raw: &str) -> String {
    raw.trim().trim_start_matches("SHA256:").trim().to_string()
}

pub(crate) fn shell_quote_single(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn normalize_new_target_request(
    request: NewTargetRequest,
) -> Result<NormalizedNewTargetRequest, RegisterNewTargetError> {
    let display_name = request.display_name.trim().to_string();
    let host = request.host.trim().to_string();
    let username = request.username.trim().to_string();
    let port = request.port.unwrap_or(TARGET_ENROLLMENT_DEFAULT_SSH_PORT);

    if host.is_empty() {
        return Err(RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::ValidationFailed,
            "Host is required",
            None,
        ));
    }

    if host.chars().any(char::is_whitespace) {
        return Err(RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::ValidationFailed,
            "Host cannot contain whitespace",
            Some(host),
        ));
    }

    if username.is_empty() {
        return Err(RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::ValidationFailed,
            "Username is required",
            None,
        ));
    }

    if username.chars().any(char::is_whitespace) {
        return Err(RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::ValidationFailed,
            "Username cannot contain whitespace",
            Some(username),
        ));
    }

    if port == 0 {
        return Err(RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::ValidationFailed,
            "SSH port must be between 1 and 65535",
            None,
        ));
    }

    let resolved_display_name = if display_name.is_empty() {
        host.clone()
    } else {
        display_name
    };

    let private_key_raw = request
        .ssh_private_key_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(TARGET_ENROLLMENT_KEY_DEFAULT_PATH);
    let ssh_private_key_path = expand_tilde_path(private_key_raw);

    let expected_hostkey_sha256_b64 = request
        .expected_hostkey_sha256
        .as_deref()
        .map(normalize_hostkey_sha256_b64)
        .filter(|value| !value.is_empty());

    Ok(NormalizedNewTargetRequest {
        display_name: resolved_display_name,
        host,
        port,
        username,
        ssh_private_key_path,
        expected_hostkey_sha256_b64,
        controller_epoch: request.controller_epoch,
    })
}

pub(crate) async fn run_external_command(
    binary: &str,
    args: &[String],
    timeout_secs: u64,
) -> Result<CommandRunOutput, RegisterNewTargetError> {
    let mut command = tokio::process::Command::new(binary);
    command
        .kill_on_drop(true)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let output = timeout(Duration::from_secs(timeout_secs), command.output())
        .await
        .map_err(|_| {
            RegisterNewTargetError::new(
                RegisterNewTargetErrorCode::BootstrapFailed,
                format!("Command timed out: {binary}"),
                Some(format!(
                    "args={} timeout_secs={timeout_secs}",
                    args.join(" ")
                )),
            )
        })?
        .map_err(|error| {
            RegisterNewTargetError::new(
                RegisterNewTargetErrorCode::DependencyMissing,
                format!("Failed to launch command: {binary}"),
                Some(error.to_string()),
            )
        })?;

    Ok(CommandRunOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn parse_ssh_hostkey_fingerprint(ssh_keygen_stdout: &str) -> Option<String> {
    let mut fallback: Option<String> = None;
    for line in ssh_keygen_stdout.lines() {
        let token = line
            .split_whitespace()
            .find(|part| part.starts_with("SHA256:"));
        let Some(raw_fp) = token else {
            continue;
        };

        let normalized = normalize_hostkey_sha256_b64(raw_fp);
        if line.to_ascii_lowercase().contains("ed25519") {
            return Some(normalized);
        }

        if fallback.is_none() {
            fallback = Some(normalized);
        }
    }

    fallback
}

pub(crate) fn classify_ssh_stage_error(
    output: &CommandRunOutput,
    stage: &str,
) -> RegisterNewTargetError {
    let merged = format!("{}\n{}", output.stderr, output.stdout);
    let merged_lower = merged.to_ascii_lowercase();
    let detail = Some(format!(
        "stage={stage}; exit_code={:?}; stderr={}; stdout={}",
        output.status.code(),
        truncate_for_error(output.stderr.trim(), 500),
        truncate_for_error(output.stdout.trim(), 500),
    ));

    if merged_lower.contains("remote host identification has changed")
        || merged_lower.contains("host key verification failed")
    {
        return RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::HostKeyChanged,
            "Host key changed while enrolling target",
            detail,
        );
    }

    if merged_lower.contains("permission denied")
        || merged_lower.contains("authentication failed")
        || merged_lower.contains("publickey")
    {
        return RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::AuthenticationFailed,
            "SSH authentication failed for the provided user/key",
            detail,
        );
    }

    if merged_lower.contains("connection refused")
        || merged_lower.contains("connection timed out")
        || merged_lower.contains("operation timed out")
        || merged_lower.contains("no route to host")
        || merged_lower.contains("could not resolve hostname")
        || merged_lower.contains("name or service not known")
    {
        return RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::ConnectionRefused,
            "Connection refused or host unreachable during enrollment",
            detail,
        );
    }

    RegisterNewTargetError::new(
        RegisterNewTargetErrorCode::BootstrapFailed,
        "Failed to complete SSH bootstrap for target enrollment",
        detail,
    )
}

pub(crate) fn build_ssh_base_args(
    request: &NormalizedNewTargetRequest,
    known_hosts_path: &Path,
) -> Vec<String> {
    vec![
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "ConnectTimeout=10".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=yes".to_string(),
        "-o".to_string(),
        format!("UserKnownHostsFile={}", known_hosts_path.to_string_lossy()),
        "-o".to_string(),
        "GlobalKnownHostsFile=/dev/null".to_string(),
        "-o".to_string(),
        "IdentitiesOnly=yes".to_string(),
        "-o".to_string(),
        "PreferredAuthentications=publickey".to_string(),
        "-o".to_string(),
        "PasswordAuthentication=no".to_string(),
        "-i".to_string(),
        request.ssh_private_key_path.to_string_lossy().to_string(),
        "-p".to_string(),
        request.port.to_string(),
        format!("{}@{}", request.username, request.host),
    ]
}

pub(crate) async fn ensure_local_ssh_keypair(
    private_key_path: &Path,
) -> Result<(String, PathBuf, bool), RegisterNewTargetError> {
    let Some(parent) = private_key_path.parent() else {
        return Err(RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::ValidationFailed,
            "Invalid SSH private key path",
            Some(private_key_path.to_string_lossy().to_string()),
        ));
    };

    std::fs::create_dir_all(parent).map_err(|error| {
        RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::PersistenceFailed,
            "Failed to create directory for SSH key",
            Some(error.to_string()),
        )
    })?;

    let public_key_path = PathBuf::from(format!("{}.pub", private_key_path.to_string_lossy()));
    let mut created_key = false;

    if !private_key_path.exists() {
        let args = vec![
            "-q".to_string(),
            "-t".to_string(),
            "ed25519".to_string(),
            "-a".to_string(),
            "64".to_string(),
            "-N".to_string(),
            "".to_string(),
            "-f".to_string(),
            private_key_path.to_string_lossy().to_string(),
            "-C".to_string(),
            "kria-enrollment".to_string(),
        ];
        let generated =
            run_external_command("ssh-keygen", &args, TARGET_ENROLLMENT_KEYSCAN_TIMEOUT_SECS)
                .await?;
        if !generated.status.success() {
            return Err(RegisterNewTargetError::new(
                RegisterNewTargetErrorCode::BootstrapFailed,
                "Failed to generate local SSH key pair",
                Some(format!(
                    "stderr={}; stdout={}",
                    truncate_for_error(generated.stderr.trim(), 500),
                    truncate_for_error(generated.stdout.trim(), 500),
                )),
            ));
        }
        created_key = true;
    }

    if !public_key_path.exists() {
        let args = vec![
            "-y".to_string(),
            "-f".to_string(),
            private_key_path.to_string_lossy().to_string(),
        ];
        let derived =
            run_external_command("ssh-keygen", &args, TARGET_ENROLLMENT_KEYSCAN_TIMEOUT_SECS)
                .await?;
        if !derived.status.success() {
            return Err(RegisterNewTargetError::new(
                RegisterNewTargetErrorCode::BootstrapFailed,
                "Failed to derive local SSH public key",
                Some(format!(
                    "stderr={}; stdout={}",
                    truncate_for_error(derived.stderr.trim(), 500),
                    truncate_for_error(derived.stdout.trim(), 500),
                )),
            ));
        }
        std::fs::write(&public_key_path, format!("{}\n", derived.stdout.trim())).map_err(
            |error| {
                RegisterNewTargetError::new(
                    RegisterNewTargetErrorCode::PersistenceFailed,
                    "Failed to persist derived SSH public key",
                    Some(error.to_string()),
                )
            },
        )?;
    }

    let public_key_raw = std::fs::read_to_string(&public_key_path).map_err(|error| {
        RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::PersistenceFailed,
            "Failed to read local SSH public key",
            Some(error.to_string()),
        )
    })?;
    let public_key = public_key_raw.trim().to_string();

    if public_key.is_empty() {
        return Err(RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::BootstrapFailed,
            "Local SSH public key is empty",
            Some(public_key_path.to_string_lossy().to_string()),
        ));
    }

    Ok((public_key, public_key_path, created_key))
}

pub(crate) async fn fetch_ssh_hostkey_fingerprint(
    host: &str,
    port: u16,
) -> Result<(String, String), RegisterNewTargetError> {
    let keyscan_args = vec![
        "-T".to_string(),
        "8".to_string(),
        "-p".to_string(),
        port.to_string(),
        host.to_string(),
    ];
    let keyscan = run_external_command(
        "ssh-keyscan",
        &keyscan_args,
        TARGET_ENROLLMENT_KEYSCAN_TIMEOUT_SECS,
    )
    .await?;

    if keyscan.stdout.trim().is_empty() {
        return Err(classify_ssh_stage_error(&keyscan, "ssh_keyscan"));
    }

    let temp_scan_path = std::env::temp_dir().join(format!(
        "kria_target_keyscan_{}_{}.tmp",
        std::process::id(),
        Uuid::new_v4()
    ));
    std::fs::write(&temp_scan_path, keyscan.stdout.as_bytes()).map_err(|error| {
        RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::PersistenceFailed,
            "Failed to persist temporary keyscan output",
            Some(error.to_string()),
        )
    })?;
    let temp_scan_guard = TempFileGuard::new(temp_scan_path);

    let keygen_args = vec![
        "-lf".to_string(),
        temp_scan_guard.path().to_string_lossy().to_string(),
        "-E".to_string(),
        "sha256".to_string(),
    ];
    let keygen = run_external_command(
        "ssh-keygen",
        &keygen_args,
        TARGET_ENROLLMENT_KEYSCAN_TIMEOUT_SECS,
    )
    .await?;
    if !keygen.status.success() {
        return Err(RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::BootstrapFailed,
            "Failed to compute SSH host key fingerprint",
            Some(format!(
                "stderr={}; stdout={}",
                truncate_for_error(keygen.stderr.trim(), 500),
                truncate_for_error(keygen.stdout.trim(), 500),
            )),
        ));
    }

    let fingerprint = parse_ssh_hostkey_fingerprint(&keygen.stdout).ok_or_else(|| {
        RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::BootstrapFailed,
            "Unable to parse SSH host key fingerprint",
            Some(truncate_for_error(keygen.stdout.trim(), 500)),
        )
    })?;

    Ok((keyscan.stdout, fingerprint))
}

pub(crate) fn load_fleet_enrollment_registry(
    path: &Path,
) -> Result<FleetEnrollmentRegistry, RegisterNewTargetError> {
    if !path.exists() {
        return Ok(FleetEnrollmentRegistry::default());
    }

    let bytes = std::fs::read(path).map_err(|error| {
        RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::PersistenceFailed,
            "Failed to read enrollment registry",
            Some(error.to_string()),
        )
    })?;

    if bytes.is_empty() {
        return Ok(FleetEnrollmentRegistry::default());
    }

    serde_json::from_slice::<FleetEnrollmentRegistry>(&bytes).map_err(|error| {
        RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::PersistenceFailed,
            "Enrollment registry is corrupted",
            Some(error.to_string()),
        )
    })
}

pub(crate) fn save_fleet_enrollment_registry(
    path: &Path,
    registry: &FleetEnrollmentRegistry,
) -> Result<(), RegisterNewTargetError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            RegisterNewTargetError::new(
                RegisterNewTargetErrorCode::PersistenceFailed,
                "Failed to create enrollment registry directory",
                Some(error.to_string()),
            )
        })?;
    }

    let payload = serde_json::to_vec_pretty(registry).map_err(|error| {
        RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::PersistenceFailed,
            "Failed to serialize enrollment registry",
            Some(error.to_string()),
        )
    })?;

    let temp_path = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    std::fs::write(&temp_path, payload).map_err(|error| {
        RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::PersistenceFailed,
            "Failed to write temporary enrollment registry",
            Some(error.to_string()),
        )
    })?;

    if let Err(error) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::PersistenceFailed,
            "Failed to persist enrollment registry",
            Some(error.to_string()),
        ));
    }

    Ok(())
}

pub(crate) async fn resolve_target_registry_path(
    state: &AppState,
) -> Result<PathBuf, RegisterNewTargetError> {
    let config = state.config.read().await;
    let paths = config.resolve_paths().map_err(|error| {
        RegisterNewTargetError::new(
            RegisterNewTargetErrorCode::PersistenceFailed,
            "Failed to resolve KRIA data paths",
            Some(error.to_string()),
        )
    })?;

    Ok(target_registry_path_from_data_dir(paths.data_dir.as_path()))
}

fn target_registry_path_from_data_dir(data_dir: &Path) -> PathBuf {
    data_dir
        .join(TARGET_ENROLLMENT_REGISTRY_DIR)
        .join(TARGET_ENROLLMENT_REGISTRY_FILE)
}

fn default_target_registry_path() -> PathBuf {
    let paths = kria_core::platform::paths::KriaPaths::resolve();
    target_registry_path_from_data_dir(paths.data_dir.as_path())
}

pub(crate) fn load_enrolled_target_status_snapshots() -> (Vec<EnrolledTargetStatusSnapshot>, PathBuf)
{
    let registry_path = default_target_registry_path();
    let snapshots = match load_fleet_enrollment_registry(registry_path.as_path()) {
        Ok(registry) => registry
            .targets
            .into_iter()
            .map(|target| EnrolledTargetStatusSnapshot {
                target_id: target.target_id,
                display_name: target.display_name,
                host: target.host,
                port: target.port,
                username: target.username,
                mode: target.mode,
                ssh_hostkey_sha256_b64: target.ssh_hostkey_sha256_b64,
                controller_epoch: target.controller_epoch,
                enrolled_at_unix_ms: target.enrolled_at_unix_ms,
                last_verified_unix_ms: target.last_verified_unix_ms,
            })
            .collect(),
        Err(error) => {
            tracing::warn!(
                code = ?error.code,
                message = %error.message,
                detail = ?error.detail,
                path = %registry_path.to_string_lossy(),
                "fleet status: failed to load enrollment registry snapshot"
            );
            Vec::new()
        }
    };

    (snapshots, registry_path)
}

pub(crate) fn fleet_runtime_root_from_data_dir(data_dir: &Path) -> PathBuf {
    data_dir
        .join(TARGET_ENROLLMENT_REGISTRY_DIR)
        .join(TARGET_ENROLLMENT_RUNTIME_DIR)
}

fn current_host_platform() -> HostPlatform {
    if cfg!(target_os = "windows") {
        HostPlatform::Windows
    } else if cfg!(target_os = "macos") {
        HostPlatform::MacOs
    } else {
        HostPlatform::Linux
    }
}

fn build_remote_config_for_enrolled_target(
    target: &EnrolledTargetRecord,
    runtime_root: &Path,
    system_config: &KriaSystemConfig,
) -> Result<RemoteConfig, String> {
    let target_root = runtime_root.join(&target.target_id);
    let control_root = target_root.join("control");
    let staging_root = target_root.join("staging");
    let workspace_root = target_root.join("workspace");
    let helper_cache_root = target_root.join("helper_cache");
    let helper_root = target_root.join("helper");
    let helper_lock_root = target_root.join("helper_lock");
    let state_root = target_root.join("state");
    let mux_root = target_root.join("mux");

    for directory in [
        target_root.as_path(),
        control_root.as_path(),
        staging_root.as_path(),
        workspace_root.as_path(),
        helper_cache_root.as_path(),
        helper_root.as_path(),
        helper_lock_root.as_path(),
        state_root.as_path(),
        mux_root.as_path(),
    ] {
        std::fs::create_dir_all(directory).map_err(|error| {
            format!(
                "failed to create runtime directory {}: {error}",
                directory.display()
            )
        })?;
    }

    let ssh_key_path = expand_tilde_path(&target.ssh_private_key_path);
    if !ssh_key_path.exists() {
        return Err(format!(
            "SSH private key is missing for target {}: {}",
            target.target_id,
            ssh_key_path.display()
        ));
    }

    let pinned_host_key = {
        let normalized = normalize_hostkey_sha256_b64(&target.ssh_hostkey_sha256_b64);
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    };

    Ok(RemoteConfig {
        host_platform: current_host_platform(),
        host: target.host.clone(),
        port: target.port,
        username: target.username.clone(),
        ssh_key_path,
        guest_os_family: GuestOsFamily::Posix,
        target_kind: TargetKind::PhysicalRemoteHost,
        qemu_boot_cmd: None,
        qemu_pid_state_file: target_root.join("qemu.pid"),
        instance_id: format!("fleet-target-{}", target.target_id),
        remote_control_dir: control_root,
        transport_backend: SshTransportBackend::OpenSshControlMaster,
        ssh_multiplexing: SshMultiplexingConfig {
            enable_control_master: true,
            control_path_cmd: mux_root.join("cmd.sock"),
            control_path_bulk: mux_root.join("bulk.sock"),
            control_persist_secs: 90,
            establish_timeout_ms: 2_000,
            control_check_timeout_ms: 1_000,
            allow_no_mux_for_test: true,
            rust_ssh_max_parallel_channels: 16,
        },
        helper_provisioning: HelperProvisioning {
            required_helper_version: "kria-remote-helper-v1".to_string(),
            helper_manifest_path: target_root.join("helper.manifest"),
            helper_manifest_sig_path: target_root.join("helper.manifest.sig"),
            helper_public_key_path: target_root.join("helper.pub"),
            host_helper_cache_dir: helper_cache_root,
            remote_helper_dir: helper_root,
            remote_helper_lock_dir: helper_lock_root,
            helper_lock_timeout_ms: 10_000,
            helper_lock_claim_retry_ms: 200,
            supervisor_heartbeat_interval_ms: 1_000,
            supervisor_heartbeat_timeout_ms: 5_000,
            worker_journal_silence_timeout_ms: 5_000,
            emergency_status_buffer_bytes: 512 * 1024,
            last_gasp_packet_timeout_ms: 2_000,
            max_helper_rss_bytes: 128 * 1024 * 1024,
        },
        control_transport: ControlPlaneTransport::EphemeralSftpFile,
        envelope_ttl_ms: system_config.target_pool.lease_ttl_ms.max(5_000),
        max_command_payload_bytes: 1_024 * 1_024,
        file_commit_policy: FileCommitPolicy {
            remote_staging_dir: staging_root,
            privileged_commit_mode: PrivilegedCommitMode::Disabled,
            privileged_commit_helper_path: None,
            staging_sweep_ttl_secs: 300,
            staging_lease_heartbeat_timeout_ms: 2_000,
            staging_sweep_batch_limit: 64,
            enforce_linux_openat2: cfg!(target_os = "linux"),
            privileged_probe_timeout_ms: 1_500,
            privileged_commit_timeout_ms: 2_000,
            disable_privileged_on_probe_failure: true,
        },
        guest_filesystem_policy: GuestFilesystemPolicy {
            require_control_dir_writable: true,
            require_staging_dir_writable: true,
            require_non_readonly_mount: true,
            min_free_bytes_floor: 64 * 1024 * 1024,
        },
        reset_policy: ResetPolicy {
            admission_freeze_timeout_ms: 750,
            zombie_reap_timeout_ms: 500,
            lock_acquire_timeout_ms: 1_500,
            network_call_timeout_ms: 5_000,
            total_reset_deadline_ms: 30_000,
        },
        replay_cache_policy: ReplayCachePolicy {
            retained_epoch_buckets: 2,
            max_nonces_per_epoch: 512,
        },
        ssh_pool: SshPoolConfig {
            max_active_targets_hard_cap: 128,
            idle_ttl_secs: 120,
            sweep_interval_secs: 30,
            fd_soft_limit: 16_384,
            fd_reserve: 256,
            fd_per_command_budget: 8,
            fd_telemetry_sample_ms: 1_000,
        },
        host_artifact_gc: HostArtifactGcConfig {
            enable_gc: true,
            gc_ttl_secs: 3_600,
            state_root_dir: state_root,
            host_binary_sha256_or_build_id: format!("kria-desktop-{}", env!("CARGO_PKG_VERSION")),
        },
        infrastructure_runtime: InfrastructureRuntimeConfig {
            infra_worker_threads: 2,
            high_priority_queue_capacity: 64,
            medium_priority_queue_capacity: 64,
            low_priority_queue_capacity: 64,
            infra_spawn_timeout_ms: 2_000,
        },
        ssh_connect_timeout_ms: 15_000,
        command_timeout_ms: 60_000,
        boot_wait_timeout_ms: 20_000,
        poll_interval_ms: 250,
        shutdown_timeout_ms: 10_000,
        soft_reset_grace_ms: 1_000,
        soft_reset_kill_timeout_ms: 5_000,
        max_soft_reset_attempts: 2,
        inflight_drain_timeout_ms: 10_000,
        local_cancel_kill_timeout_ms: 5_000,
        max_stdout_bytes: 2 * 1024 * 1024,
        max_stderr_bytes: 2 * 1024 * 1024,
        max_read_file_bytes: 2 * 1024 * 1024,
        command_timeout_requires_reset: true,
        known_hosts_path: None,
        strict_host_key_checking: pinned_host_key.is_some(),
        pinned_host_key_sha256: pinned_host_key,
        remote_workspace_root: Some(workspace_root),
    })
}

fn build_placeholder_bridge_remote_config(
    runtime_root: &Path,
    system_config: &KriaSystemConfig,
) -> Result<RemoteConfig, String> {
    let bridge_root = runtime_root.join("_bridge_fallback");
    std::fs::create_dir_all(&bridge_root).map_err(|error| {
        format!(
            "failed to create bridge fallback runtime directory {}: {error}",
            bridge_root.display()
        )
    })?;

    Ok(RemoteConfig {
        host_platform: current_host_platform(),
        host: "bridge-placeholder.local".to_string(),
        port: 22,
        username: "bridge".to_string(),
        ssh_key_path: bridge_root.join("placeholder.key"),
        guest_os_family: GuestOsFamily::Posix,
        target_kind: TargetKind::PhysicalRemoteHost,
        qemu_boot_cmd: None,
        qemu_pid_state_file: bridge_root.join("qemu.pid"),
        instance_id: format!("fleet-bridge-{}", Uuid::new_v4()),
        remote_control_dir: bridge_root.join("control"),
        transport_backend: SshTransportBackend::OpenSshControlMaster,
        ssh_multiplexing: SshMultiplexingConfig {
            enable_control_master: false,
            control_path_cmd: bridge_root.join("cmd.sock"),
            control_path_bulk: bridge_root.join("bulk.sock"),
            control_persist_secs: 15,
            establish_timeout_ms: 500,
            control_check_timeout_ms: 500,
            allow_no_mux_for_test: true,
            rust_ssh_max_parallel_channels: 4,
        },
        helper_provisioning: HelperProvisioning {
            required_helper_version: "bridge-placeholder".to_string(),
            helper_manifest_path: bridge_root.join("helper.manifest"),
            helper_manifest_sig_path: bridge_root.join("helper.manifest.sig"),
            helper_public_key_path: bridge_root.join("helper.pub"),
            host_helper_cache_dir: bridge_root.join("helper_cache"),
            remote_helper_dir: bridge_root.join("helper"),
            remote_helper_lock_dir: bridge_root.join("helper_lock"),
            helper_lock_timeout_ms: 500,
            helper_lock_claim_retry_ms: 50,
            supervisor_heartbeat_interval_ms: 500,
            supervisor_heartbeat_timeout_ms: 2_000,
            worker_journal_silence_timeout_ms: 2_000,
            emergency_status_buffer_bytes: 512 * 1024,
            last_gasp_packet_timeout_ms: 500,
            max_helper_rss_bytes: 64 * 1024 * 1024,
        },
        control_transport: ControlPlaneTransport::EphemeralSftpFile,
        envelope_ttl_ms: system_config.target_pool.lease_ttl_ms.max(1_000),
        max_command_payload_bytes: 64 * 1024,
        file_commit_policy: FileCommitPolicy {
            remote_staging_dir: bridge_root.join("staging"),
            privileged_commit_mode: PrivilegedCommitMode::Disabled,
            privileged_commit_helper_path: None,
            staging_sweep_ttl_secs: 60,
            staging_lease_heartbeat_timeout_ms: 500,
            staging_sweep_batch_limit: 16,
            enforce_linux_openat2: cfg!(target_os = "linux"),
            privileged_probe_timeout_ms: 500,
            privileged_commit_timeout_ms: 500,
            disable_privileged_on_probe_failure: true,
        },
        guest_filesystem_policy: GuestFilesystemPolicy {
            require_control_dir_writable: true,
            require_staging_dir_writable: true,
            require_non_readonly_mount: true,
            min_free_bytes_floor: 1,
        },
        reset_policy: ResetPolicy {
            admission_freeze_timeout_ms: 100,
            zombie_reap_timeout_ms: 100,
            lock_acquire_timeout_ms: 250,
            network_call_timeout_ms: 500,
            total_reset_deadline_ms: 5_000,
        },
        replay_cache_policy: ReplayCachePolicy {
            retained_epoch_buckets: 2,
            max_nonces_per_epoch: 64,
        },
        ssh_pool: SshPoolConfig {
            max_active_targets_hard_cap: 8,
            idle_ttl_secs: 30,
            sweep_interval_secs: 30,
            fd_soft_limit: 4_096,
            fd_reserve: 64,
            fd_per_command_budget: 4,
            fd_telemetry_sample_ms: 100,
        },
        host_artifact_gc: HostArtifactGcConfig {
            enable_gc: true,
            gc_ttl_secs: 60,
            state_root_dir: bridge_root.join("state"),
            host_binary_sha256_or_build_id: "bridge-placeholder".to_string(),
        },
        infrastructure_runtime: InfrastructureRuntimeConfig {
            infra_worker_threads: 1,
            high_priority_queue_capacity: 8,
            medium_priority_queue_capacity: 8,
            low_priority_queue_capacity: 8,
            infra_spawn_timeout_ms: 500,
        },
        ssh_connect_timeout_ms: 500,
        command_timeout_ms: 500,
        boot_wait_timeout_ms: 500,
        poll_interval_ms: 50,
        shutdown_timeout_ms: 500,
        soft_reset_grace_ms: 100,
        soft_reset_kill_timeout_ms: 100,
        max_soft_reset_attempts: 1,
        inflight_drain_timeout_ms: 500,
        local_cancel_kill_timeout_ms: 500,
        max_stdout_bytes: 128 * 1024,
        max_stderr_bytes: 128 * 1024,
        max_read_file_bytes: 128 * 1024,
        command_timeout_requires_reset: true,
        known_hosts_path: None,
        strict_host_key_checking: false,
        pinned_host_key_sha256: None,
        remote_workspace_root: Some(bridge_root.join("workspace")),
    })
}

pub(crate) async fn admit_enrolled_target_to_fleet_runtime(
    fleet_runtime: &Arc<FleetRuntimeState>,
    target: &EnrolledTargetRecord,
) -> Result<bool, String> {
    let _admission_guard = fleet_runtime.admission_lock.lock().await;

    let parsed_target_id = Uuid::parse_str(target.target_id.trim()).map_err(|error| {
        format!(
            "target_id '{}' is not a valid UUID: {error}",
            target.target_id
        )
    })?;
    let target_id = TargetId(parsed_target_id);

    if fleet_runtime
        .target_pool
        .inventory_state(&target_id)
        .await
        .is_some()
    {
        return Ok(false);
    }

    let remote_config = build_remote_config_for_enrolled_target(
        target,
        fleet_runtime.runtime_root.as_path(),
        &fleet_runtime.system_config,
    )?;

    let runtime_handle = tokio::runtime::Handle::current();
    let environment = Arc::new(
        QemuSshEnvironment::new(remote_config, runtime_handle.clone(), runtime_handle)
            .map_err(|error| format!("failed to construct QemuSshEnvironment: {error}"))?,
    );

    fleet_runtime
        .target_pool
        .add_target(target_id, environment, TargetHealthTelemetry::default())
        .await;

    Ok(true)
}

pub(crate) fn configure_orchestrator_fleet_bridge(
    orchestrator: &Arc<Orchestrator>,
    fleet_runtime: &Arc<FleetRuntimeState>,
) -> Result<(), String> {
    let fallback_config = build_placeholder_bridge_remote_config(
        fleet_runtime.runtime_root.as_path(),
        &fleet_runtime.system_config,
    )?;

    let runtime_handle = tokio::runtime::Handle::current();
    let fallback_environment = Arc::new(
        QemuSshEnvironment::new(fallback_config, runtime_handle.clone(), runtime_handle).map_err(
            |error| {
                format!(
                    "failed to build fallback remote environment for orchestrator bridge: {error}"
                )
            },
        )?,
    );

    let bridge = RemoteQemuToolBridge::new(fallback_environment)
        .with_target_pool(fleet_runtime.target_pool.clone());
    orchestrator.set_remote_tool_bridge(bridge);
    Ok(())
}

pub(crate) async fn pulse_target_pool_telemetry(target_pool: &Arc<TargetPool>) {
    match target_pool.acquire_lease().await {
        Ok(lease) => {
            if let Err(error) = target_pool.release_lease(&lease.lease_id).await {
                tracing::warn!(
                    error = %error,
                    lease_id = %lease.lease_id.0,
                    "fleet runtime: failed to release telemetry pulse lease"
                );
            }
        }
        Err(error) => {
            tracing::debug!(
                error = %error,
                "fleet runtime: telemetry pulse skipped"
            );
        }
    }
}
