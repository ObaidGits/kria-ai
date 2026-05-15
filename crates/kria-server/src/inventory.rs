use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use kria_connection_control::manager::{
    ClockDriftAlert, ConnectionManager, ConnectionManagerConfig, ConnectionManagerHandle,
    Connector, ConnectorRegistry, ControlPlaneEvent, ControllerRole, DispatchResult,
    DockerEvalSummary, DockerHealthStatus, FleetStore, HaControlState, IdentityProof,
    KeyAttestationMaterial, SecurityAlert, TargetIdentity, TargetMode, TargetState,
    TerminalGapMarker, TerminalStream,
};
use kria_connection_control::signer::{DualKeyHmacEnvelopeSigner, KeyMaterial, SignedEnvelope};
use kria_core::config::KriaConfig;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

const TARGET_ENROLLMENT_REGISTRY_DIR: &str = "fleet";
const TARGET_ENROLLMENT_REGISTRY_FILE: &str = "targets.json";
const SSH_DEFAULT_PORT: u16 = 22;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FleetEnrollmentRegistry {
    #[serde(default)]
    targets: Vec<EnrolledTargetRecord>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnrolledTargetRecord {
    target_id: String,
    display_name: String,
    host: String,
    #[serde(default = "default_ssh_port")]
    port: u16,
    username: String,
    #[serde(default = "default_mode")]
    mode: String,
    ssh_private_key_path: String,
    #[serde(default)]
    ssh_hostkey_sha256_b64: String,
    #[serde(default, alias = "commanderEpoch")]
    _controller_epoch: i64,
}

#[derive(Debug, Clone)]
struct SshTargetProfile {
    host: String,
    port: u16,
    username: String,
    ssh_private_key_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct FleetTargetProjection {
    pub target_id: String,
    pub display_name: String,
    pub mode: String,
    pub state: String,
    pub tainted: bool,
    pub reason: Option<String>,
    pub health_score: f64,
    pub latency_ewma_ms: f64,
    pub recent_failure_rate: f64,
    pub docker_health: String,
    pub docker_pass_count: u32,
    pub docker_fail_count: u32,
    pub docker_last_run_at_unix_ms: Option<i64>,
    pub updated_at_unix_ms: i64,
}

impl FleetTargetProjection {
    fn from_target(target: &TargetIdentity) -> Self {
        Self {
            target_id: target.target_id.to_string(),
            display_name: target.display_name.clone(),
            mode: target_mode_label(target.mode).to_string(),
            state: target_state_label(target.state).to_string(),
            tainted: target.tainted,
            reason: target.taint_reason.clone(),
            health_score: target.health_score,
            latency_ewma_ms: target.latency_ewma_ms,
            recent_failure_rate: target.recent_failure_rate,
            docker_health: docker_health_label(target.docker_health).to_string(),
            docker_pass_count: target.docker_pass_count,
            docker_fail_count: target.docker_fail_count,
            docker_last_run_at_unix_ms: target.docker_last_run_at_unix_ms,
            updated_at_unix_ms: now_unix_ms(),
        }
    }
}

#[derive(Clone)]
pub struct FleetRuntime {
    pub manager: ConnectionManagerHandle,
    projections: Arc<RwLock<HashMap<Uuid, FleetTargetProjection>>>,
    pub registry_path: PathBuf,
}

impl FleetRuntime {
    pub async fn initialize(config: &KriaConfig) -> Result<Self> {
        let paths = config.resolve_paths().context("resolve_paths failed")?;
        let registry_path = paths
            .data_dir
            .join(TARGET_ENROLLMENT_REGISTRY_DIR)
            .join(TARGET_ENROLLMENT_REGISTRY_FILE);

        let registry = load_registry(registry_path.as_path())
            .with_context(|| format!("failed to load {}", registry_path.display()))?;

        let mut initial_targets = Vec::new();
        let mut profiles = HashMap::new();
        for record in registry.targets {
            match map_record_to_target(record) {
                Ok((target, profile)) => {
                    profiles.insert(target.target_id, profile);
                    initial_targets.push(target);
                }
                Err(error) => {
                    tracing::warn!(error = %error, "skipping invalid enrolled target during fleet hydration");
                }
            }
        }

        let signer = Arc::new(build_signer());
        let store = Arc::new(InMemoryFleetStore::new(&initial_targets));
        let connector = Arc::new(SshConnector::new(profiles));
        let connectors = ConnectorRegistry {
            ssh: connector.clone(),
            reverse_ws: connector.clone(),
            unix_socket: connector,
        };

        let controller_epoch = initial_targets.iter().map(|_| 1_i64).max().unwrap_or(1_i64);

        let manager = ConnectionManager::spawn(
            initial_targets.clone(),
            connectors,
            signer,
            store,
            None,
            None,
            HaControlState {
                controller_id: Uuid::new_v4(),
                role: ControllerRole::Primary,
                controller_epoch,
                lease_fence_token: 1,
                failover_timeout: Duration::from_secs(8),
            },
            ConnectionManagerConfig::default(),
        );

        let projections = Arc::new(RwLock::new(
            initial_targets
                .iter()
                .map(|target| (target.target_id, FleetTargetProjection::from_target(target)))
                .collect::<HashMap<_, _>>(),
        ));

        let runtime = Self {
            manager,
            projections,
            registry_path,
        };
        runtime.spawn_projection_loop();
        Ok(runtime)
    }

    fn spawn_projection_loop(&self) {
        let projections = self.projections.clone();
        let mut rx = self.manager.subscribe_events();

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => apply_event_to_projection(&projections, &event).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "fleet projection receiver lagged; continuing");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    pub async fn snapshot_targets(&self) -> Vec<FleetTargetProjection> {
        let guard = self.projections.read().await;
        let mut rows = guard.values().cloned().collect::<Vec<_>>();
        rows.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        rows
    }

    pub async fn snapshot_event_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "snapshot",
            "targets": self.snapshot_targets().await,
        })
    }
}

pub fn control_plane_event_json(event: &ControlPlaneEvent) -> serde_json::Value {
    match event {
        ControlPlaneEvent::TargetStatus {
            target_id,
            display_name,
            mode,
            state,
            tainted,
            reason,
            health_score,
            latency_ewma_ms,
            recent_failure_rate,
            docker_health,
            docker_pass_count,
            docker_fail_count,
            docker_last_run_at_unix_ms,
        } => serde_json::json!({
            "type": "target_status",
            "target_id": target_id,
            "display_name": display_name,
            "mode": target_mode_label(*mode),
            "state": target_state_label(*state),
            "tainted": tainted,
            "reason": reason,
            "health_score": health_score,
            "latency_ewma_ms": latency_ewma_ms,
            "recent_failure_rate": recent_failure_rate,
            "docker_health": docker_health_label(*docker_health),
            "docker_pass_count": docker_pass_count,
            "docker_fail_count": docker_fail_count,
            "docker_last_run_at_unix_ms": docker_last_run_at_unix_ms,
            "updated_at_unix_ms": now_unix_ms(),
        }),
        ControlPlaneEvent::FleetAlert {
            target_id,
            lease_id,
            category,
            message,
        } => serde_json::json!({
            "type": "fleet_alert",
            "target_id": target_id,
            "lease_id": lease_id,
            "category": category,
            "message": message,
            "created_at_unix_ms": now_unix_ms(),
        }),
        ControlPlaneEvent::DockerEvalUpdate {
            target_id,
            run_id,
            docker_health,
            docker_pass_count,
            docker_fail_count,
            updated_at_unix_ms,
        } => serde_json::json!({
            "type": "docker_eval_update",
            "target_id": target_id,
            "run_id": run_id,
            "docker_health": docker_health_label(*docker_health),
            "docker_pass_count": docker_pass_count,
            "docker_fail_count": docker_fail_count,
            "docker_last_run_at_unix_ms": updated_at_unix_ms,
            "updated_at_unix_ms": updated_at_unix_ms,
        }),
        ControlPlaneEvent::TerminalGap { marker } => serde_json::json!({
            "type": "terminal_gap",
            "target_id": marker.target_id,
            "session_id": marker.session_id,
            "since_offset": marker.since_offset,
            "message": marker.message,
            "created_at_unix_ms": marker.created_at_unix_ms,
        }),
        ControlPlaneEvent::TerminalLine {
            target_id,
            lease_id,
            offset,
            stream,
            text,
            ts_unix_ms,
        } => serde_json::json!({
            "type": "terminal_line",
            "target_id": target_id,
            "lease_id": lease_id,
            "offset": offset,
            "stream": terminal_stream_label(*stream),
            "text": text,
            "ts_unix_ms": ts_unix_ms,
        }),
        ControlPlaneEvent::ClockDrift { alert } => serde_json::json!({
            "type": "clock_drift",
            "alert": {
                "target_id": alert.target_id,
                "previous_buffer_ms": alert.previous_buffer_ms,
                "next_buffer_ms": alert.next_buffer_ms,
                "rejection_count": alert.rejection_count,
                "created_at_unix_ms": alert.created_at_unix_ms,
            }
        }),
        ControlPlaneEvent::TargetRemoved { target_id } => serde_json::json!({
            "type": "target_removed",
            "target_id": target_id,
        }),
    }
}

async fn apply_event_to_projection(
    projections: &Arc<RwLock<HashMap<Uuid, FleetTargetProjection>>>,
    event: &ControlPlaneEvent,
) {
    match event {
        ControlPlaneEvent::TargetStatus {
            target_id,
            display_name,
            mode,
            state,
            tainted,
            reason,
            health_score,
            latency_ewma_ms,
            recent_failure_rate,
            docker_health,
            docker_pass_count,
            docker_fail_count,
            docker_last_run_at_unix_ms,
        } => {
            let mut guard = projections.write().await;
            guard.insert(
                *target_id,
                FleetTargetProjection {
                    target_id: target_id.to_string(),
                    display_name: display_name.clone(),
                    mode: target_mode_label(*mode).to_string(),
                    state: target_state_label(*state).to_string(),
                    tainted: *tainted,
                    reason: reason.clone(),
                    health_score: *health_score,
                    latency_ewma_ms: *latency_ewma_ms,
                    recent_failure_rate: *recent_failure_rate,
                    docker_health: docker_health_label(*docker_health).to_string(),
                    docker_pass_count: *docker_pass_count,
                    docker_fail_count: *docker_fail_count,
                    docker_last_run_at_unix_ms: *docker_last_run_at_unix_ms,
                    updated_at_unix_ms: now_unix_ms(),
                },
            );
        }
        ControlPlaneEvent::DockerEvalUpdate {
            target_id,
            docker_health,
            docker_pass_count,
            docker_fail_count,
            updated_at_unix_ms,
            ..
        } => {
            let mut guard = projections.write().await;
            if let Some(row) = guard.get_mut(target_id) {
                row.docker_health = docker_health_label(*docker_health).to_string();
                row.docker_pass_count = *docker_pass_count;
                row.docker_fail_count = *docker_fail_count;
                row.docker_last_run_at_unix_ms = Some(*updated_at_unix_ms);
                row.updated_at_unix_ms = *updated_at_unix_ms;
            }
        }
        ControlPlaneEvent::TargetRemoved { target_id } => {
            let mut guard = projections.write().await;
            guard.remove(target_id);
        }
        _ => {}
    }
}

#[derive(Default)]
struct InMemoryFleetStore {
    controller_heartbeat: Mutex<HashMap<Uuid, (i64, Instant)>>,
    lease_owners: Mutex<HashMap<Uuid, (i64, i64)>>,
    docker_summaries: Mutex<HashMap<Uuid, DockerEvalSummary>>,
    docker_health: Mutex<HashMap<Uuid, (DockerHealthStatus, Uuid)>>,
    attestation: Mutex<HashMap<Uuid, KeyAttestationMaterial>>,
    security_alerts: Mutex<Vec<SecurityAlert>>,
    terminal_gaps: Mutex<Vec<TerminalGapMarker>>,
    clock_drift_alerts: Mutex<Vec<ClockDriftAlert>>,
}

impl InMemoryFleetStore {
    fn new(initial_targets: &[TargetIdentity]) -> Self {
        let mut attestation = HashMap::new();
        for target in initial_targets {
            attestation.insert(
                target.target_id,
                KeyAttestationMaterial {
                    active_ssh_fingerprint: target.ssh_hostkey_sha256_b64.clone(),
                    active_mtls_fingerprint: target.mtls_cert_sha256_b64.clone(),
                    next_ssh_fingerprint: None,
                    next_mtls_fingerprint: None,
                    active_attestation_pubkey_b64: None,
                    next_attestation_pubkey_b64: None,
                },
            );
        }

        Self {
            attestation: Mutex::new(attestation),
            ..Self::default()
        }
    }
}

#[async_trait]
impl FleetStore for InMemoryFleetStore {
    async fn heartbeat_controller(&self, controller_id: Uuid, epoch: i64) -> Result<()> {
        self.controller_heartbeat
            .lock()
            .await
            .insert(controller_id, (epoch, Instant::now()));
        Ok(())
    }

    async fn promote_if_stale(
        &self,
        controller_id: Uuid,
        expected_old_epoch: i64,
        failover_timeout: Duration,
    ) -> Result<(bool, i64, i64)> {
        let now = Instant::now();
        let mut guard = self.controller_heartbeat.lock().await;

        let mut promoted = true;
        if let Some((existing_epoch, last_seen)) = guard.get(&controller_id).copied() {
            if existing_epoch > expected_old_epoch {
                return Ok((false, existing_epoch, existing_epoch + 1));
            }
            promoted = now.duration_since(last_seen) >= failover_timeout;
        }

        if promoted {
            let next_epoch = expected_old_epoch.saturating_add(1);
            let next_fence = next_epoch;
            guard.insert(controller_id, (next_epoch, now));
            Ok((true, next_epoch, next_fence))
        } else {
            Ok((false, expected_old_epoch, expected_old_epoch + 1))
        }
    }

    async fn takeover_active_leases(
        &self,
        _controller_id: Uuid,
        _controller_epoch: i64,
        _fence_token: i64,
    ) -> Result<u64> {
        Ok(0)
    }

    async fn cas_lease_owner(
        &self,
        lease_id: Uuid,
        _expected_epoch: i64,
        next_epoch: i64,
        next_fence_token: i64,
    ) -> Result<bool> {
        self.lease_owners
            .lock()
            .await
            .insert(lease_id, (next_epoch, next_fence_token));
        Ok(true)
    }

    async fn update_target_docker_health(
        &self,
        target_id: Uuid,
        status: DockerHealthStatus,
        run_id: Uuid,
    ) -> Result<()> {
        self.docker_health
            .lock()
            .await
            .insert(target_id, (status, run_id));
        Ok(())
    }

    async fn save_docker_eval_summary(&self, summary: &DockerEvalSummary) -> Result<()> {
        self.docker_summaries
            .lock()
            .await
            .insert(summary.run_id, summary.clone());
        Ok(())
    }

    async fn load_target_attestation_material(
        &self,
        target_id: Uuid,
    ) -> Result<KeyAttestationMaterial> {
        let guard = self.attestation.lock().await;
        Ok(guard
            .get(&target_id)
            .cloned()
            .unwrap_or(KeyAttestationMaterial {
                active_ssh_fingerprint: None,
                active_mtls_fingerprint: None,
                next_ssh_fingerprint: None,
                next_mtls_fingerprint: None,
                active_attestation_pubkey_b64: None,
                next_attestation_pubkey_b64: None,
            }))
    }

    async fn commit_attested_rotation(
        &self,
        target_id: Uuid,
        new_ssh: Option<String>,
        new_mtls: Option<String>,
    ) -> Result<()> {
        let mut guard = self.attestation.lock().await;
        let current = guard.entry(target_id).or_insert(KeyAttestationMaterial {
            active_ssh_fingerprint: None,
            active_mtls_fingerprint: None,
            next_ssh_fingerprint: None,
            next_mtls_fingerprint: None,
            active_attestation_pubkey_b64: None,
            next_attestation_pubkey_b64: None,
        });
        if new_ssh.is_some() {
            current.active_ssh_fingerprint = new_ssh;
        }
        if new_mtls.is_some() {
            current.active_mtls_fingerprint = new_mtls;
        }
        Ok(())
    }

    async fn record_security_alert(&self, alert: &SecurityAlert) -> Result<()> {
        self.security_alerts.lock().await.push(alert.clone());
        Ok(())
    }

    async fn record_terminal_gap(&self, marker: &TerminalGapMarker) -> Result<()> {
        self.terminal_gaps.lock().await.push(marker.clone());
        Ok(())
    }

    async fn record_clock_drift_alert(&self, alert: &ClockDriftAlert) -> Result<()> {
        self.clock_drift_alerts.lock().await.push(alert.clone());
        Ok(())
    }
}

#[derive(Clone)]
struct SshConnector {
    profiles: Arc<RwLock<HashMap<Uuid, SshTargetProfile>>>,
}

impl SshConnector {
    fn new(profiles: HashMap<Uuid, SshTargetProfile>) -> Self {
        Self {
            profiles: Arc::new(RwLock::new(profiles)),
        }
    }

    async fn profile_for(&self, target_id: Uuid) -> Result<SshTargetProfile> {
        let guard = self.profiles.read().await;
        guard
            .get(&target_id)
            .cloned()
            .ok_or_else(|| anyhow!("missing ssh profile for target {}", target_id))
    }

    async fn keyscan_entries(&self, profile: &SshTargetProfile) -> Result<String> {
        let args = vec![
            "-T".to_string(),
            "8".to_string(),
            "-p".to_string(),
            profile.port.to_string(),
            profile.host.clone(),
        ];
        let output = run_external("ssh-keyscan", args).await?;
        if output.stdout.trim().is_empty() {
            return Err(anyhow!(
                "ssh-keyscan returned no keys for {}:{}",
                profile.host,
                profile.port
            ));
        }
        Ok(output.stdout)
    }

    async fn verify_connectivity(&self, profile: &SshTargetProfile) -> Result<()> {
        let output = self.run_ssh_shell(profile, "true").await?;
        if output.status_code != 0 {
            return Err(anyhow!(
                "ssh connectivity probe failed for {}@{}:{} (exit={})",
                profile.username,
                profile.host,
                profile.port,
                output.status_code
            ));
        }
        Ok(())
    }

    async fn run_ssh_shell(
        &self,
        profile: &SshTargetProfile,
        shell: &str,
    ) -> Result<CommandOutput> {
        let known_hosts = self.keyscan_entries(profile).await?;
        let known_hosts_path = std::env::temp_dir().join(format!(
            "kria_fleet_known_hosts_{}_{}.tmp",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::write(&known_hosts_path, known_hosts.as_bytes())
            .with_context(|| format!("failed to write {}", known_hosts_path.display()))?;

        let remote = format!("bash -lc {}", shell_quote_single(shell));
        let mut args = vec![
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
            profile.ssh_private_key_path.to_string_lossy().to_string(),
            "-p".to_string(),
            profile.port.to_string(),
            format!("{}@{}", profile.username, profile.host),
            remote,
        ];

        let result = run_external("ssh", std::mem::take(&mut args)).await;
        let _ = std::fs::remove_file(&known_hosts_path);
        result
    }
}

#[async_trait]
impl Connector for SshConnector {
    async fn connect(&self, target: &TargetIdentity) -> Result<()> {
        let profile = self.profile_for(target.target_id).await?;
        self.verify_connectivity(&profile).await
    }

    async fn authenticate(&self, target: &TargetIdentity) -> Result<()> {
        let profile = self.profile_for(target.target_id).await?;
        self.verify_connectivity(&profile).await
    }

    async fn probe_identity(
        &self,
        target: &TargetIdentity,
        _endpoint: IpAddr,
    ) -> Result<IdentityProof> {
        let profile = self.profile_for(target.target_id).await?;
        let keyscan = self.keyscan_entries(&profile).await?;

        let temp_path = std::env::temp_dir().join(format!(
            "kria_fleet_probe_{}_{}.tmp",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::write(&temp_path, keyscan.as_bytes())
            .with_context(|| format!("failed to write {}", temp_path.display()))?;

        let keygen_out = run_external(
            "ssh-keygen",
            vec![
                "-lf".to_string(),
                temp_path.to_string_lossy().to_string(),
                "-E".to_string(),
                "sha256".to_string(),
            ],
        )
        .await;
        let _ = std::fs::remove_file(&temp_path);
        let keygen_out = keygen_out?;

        if keygen_out.status_code != 0 {
            return Err(anyhow!(
                "ssh-keygen fingerprint probe failed for {}:{}",
                profile.host,
                profile.port
            ));
        }

        let fingerprint = parse_ssh_hostkey_fingerprint(&keygen_out.stdout)
            .ok_or_else(|| anyhow!("unable to parse SSH host key fingerprint"))?;

        Ok(IdentityProof {
            ssh_hostkey_sha256_b64: Some(fingerprint),
            mtls_cert_sha256_b64: None,
        })
    }

    async fn dispatch(
        &self,
        target: &TargetIdentity,
        _endpoint: IpAddr,
        envelope: SignedEnvelope,
    ) -> Result<DispatchResult> {
        let profile = self.profile_for(target.target_id).await?;

        let command = if envelope.op == "docker_eval.run_case" {
            envelope
                .payload
                .get("shell")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("docker_eval.run_case payload missing shell"))?
                .to_string()
        } else if envelope.op == "trust.rotate_attest" {
            return Err(anyhow!(
                "trust.rotate_attest is not supported by OpenSSH connector"
            ));
        } else if let Some(shell) = envelope.payload.get("shell").and_then(|v| v.as_str()) {
            shell.to_string()
        } else if let Some(cmd) = envelope.payload.get("command").and_then(|v| v.as_str()) {
            cmd.to_string()
        } else {
            return Err(anyhow!(
                "unsupported operation {}: payload missing shell/command",
                envelope.op
            ));
        };

        let started = Instant::now();
        let output = self.run_ssh_shell(&profile, command.as_str()).await?;

        Ok(DispatchResult {
            exit_code: output.status_code,
            stdout: output.stdout,
            stderr: output.stderr,
            duration_ms: started.elapsed().as_millis() as u64,
            response_payload: None,
        })
    }
}

#[derive(Debug)]
struct CommandOutput {
    status_code: i32,
    stdout: String,
    stderr: String,
}

async fn run_external(binary: &str, args: Vec<String>) -> Result<CommandOutput> {
    let output = Command::new(binary)
        .args(args)
        .output()
        .await
        .with_context(|| format!("failed to launch {}", binary))?;

    Ok(CommandOutput {
        status_code: output.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn default_ssh_port() -> u16 {
    SSH_DEFAULT_PORT
}

fn default_mode() -> String {
    "ssh_bootstrap".to_string()
}

fn load_registry(path: &Path) -> Result<FleetEnrollmentRegistry> {
    if !path.exists() {
        return Ok(FleetEnrollmentRegistry {
            targets: Vec::new(),
        });
    }

    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.is_empty() {
        return Ok(FleetEnrollmentRegistry {
            targets: Vec::new(),
        });
    }

    serde_json::from_slice::<FleetEnrollmentRegistry>(&bytes)
        .with_context(|| format!("invalid JSON in {}", path.display()))
}

fn map_record_to_target(
    record: EnrolledTargetRecord,
) -> Result<(TargetIdentity, SshTargetProfile)> {
    let target_id = Uuid::parse_str(record.target_id.trim())
        .with_context(|| format!("invalid target_id {}", record.target_id))?;

    let host = record.host.trim().to_string();
    if host.is_empty() {
        return Err(anyhow!("host is empty for target {}", target_id));
    }

    let username = record.username.trim().to_string();
    if username.is_empty() {
        return Err(anyhow!("username is empty for target {}", target_id));
    }

    let mode = match record.mode.trim().to_ascii_lowercase().as_str() {
        "ssh_bootstrap" => TargetMode::SshBootstrap,
        "reverse_ws" => TargetMode::ReverseWs,
        "unix_socket" => TargetMode::UnixSocket,
        _ => TargetMode::SshBootstrap,
    };

    let ip_addr = host.parse::<IpAddr>().ok();

    let target = TargetIdentity {
        target_id,
        display_name: record.display_name,
        mode,
        dns_name: if mode == TargetMode::UnixSocket {
            None
        } else {
            Some(host.clone())
        },
        ip_addr,
        ssh_hostkey_sha256_b64: normalize_hostkey_sha256_b64(
            record.ssh_hostkey_sha256_b64.as_str(),
        ),
        mtls_cert_sha256_b64: None,
        unix_socket_path: None,
        state: TargetState::Ready,
        tainted: false,
        taint_reason: None,
        health_score: 1.0,
        latency_ewma_ms: 0.0,
        recent_failure_rate: 0.0,
        cooldown_until: None,
        docker_health: DockerHealthStatus::Unknown,
        docker_last_run_id: None,
        docker_last_run_at_unix_ms: None,
        docker_pass_count: 0,
        docker_fail_count: 0,
    };

    let profile = SshTargetProfile {
        host,
        port: if record.port == 0 {
            SSH_DEFAULT_PORT
        } else {
            record.port
        },
        username,
        ssh_private_key_path: expand_tilde_path(record.ssh_private_key_path.as_str()),
    };

    Ok((target, profile))
}

fn build_signer() -> DualKeyHmacEnvelopeSigner {
    let current_key_id = std::env::var("KRIA_FLEET_HMAC_KEY_ID_CURRENT")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "fleet-current".to_string());
    let current_secret = std::env::var("KRIA_FLEET_HMAC_KEY_CURRENT")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "kria-fleet-hmac-key-current-change-me".to_string())
        .into_bytes();

    let previous = std::env::var("KRIA_FLEET_HMAC_KEY_PREVIOUS")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|secret| KeyMaterial {
            key_id: std::env::var("KRIA_FLEET_HMAC_KEY_ID_PREVIOUS")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "fleet-previous".to_string()),
            secret: secret.into_bytes(),
        });

    let grace_secs = std::env::var("KRIA_FLEET_HMAC_PREVIOUS_GRACE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(300);

    DualKeyHmacEnvelopeSigner::new(
        KeyMaterial {
            key_id: current_key_id,
            secret: current_secret,
        },
        previous,
        Duration::from_secs(grace_secs.max(1)),
    )
}

fn normalize_hostkey_sha256_b64(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let value = trimmed
        .strip_prefix("SHA256:")
        .unwrap_or(trimmed)
        .trim()
        .to_string();

    if value.is_empty() {
        None
    } else {
        Some(value)
    }
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

        let normalized = normalize_hostkey_sha256_b64(raw_fp)?;
        if line.to_ascii_lowercase().contains("ed25519") {
            return Some(normalized);
        }

        if fallback.is_none() {
            fallback = Some(normalized);
        }
    }

    fallback
}

fn expand_tilde_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return PathBuf::from("~/.ssh/kria_id");
    }

    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }

    PathBuf::from(trimmed)
}

fn shell_quote_single(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{}'", escaped)
}

fn target_mode_label(mode: TargetMode) -> &'static str {
    match mode {
        TargetMode::SshBootstrap => "ssh_bootstrap",
        TargetMode::ReverseWs => "reverse_ws",
        TargetMode::UnixSocket => "unix_socket",
    }
}

fn target_state_label(state: TargetState) -> &'static str {
    match state {
        TargetState::Ready => "ready",
        TargetState::Leased => "leased",
        TargetState::Quarantine => "quarantine",
        TargetState::Tainted => "tainted",
        TargetState::Disabled => "disabled",
    }
}

fn docker_health_label(status: DockerHealthStatus) -> &'static str {
    match status {
        DockerHealthStatus::Unknown => "unknown",
        DockerHealthStatus::Running => "running",
        DockerHealthStatus::Pass => "pass",
        DockerHealthStatus::Fail => "fail",
    }
}

fn terminal_stream_label(stream: TerminalStream) -> &'static str {
    match stream {
        TerminalStream::Stdout => "stdout",
        TerminalStream::Stderr => "stderr",
        TerminalStream::System => "system",
    }
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or(0)
}
