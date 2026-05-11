use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::net::lookup_host;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::{interval_at, sleep, timeout, Instant};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::signer::{
    now_mono_ms, now_unix_ms, DualKeyHmacEnvelopeSigner, SignedEnvelope, SignedEnvelopeInput,
    SignerError, VerificationMetadata, DEFAULT_DRIFT_BUFFER_MS, MAX_DRIFT_BUFFER_MS,
};

const DEFAULT_MAX_DISPATCH_ATTEMPTS: usize = 5;
const DEFAULT_DISPATCH_BACKOFF_BASE_MS: u64 = 200;
const DEFAULT_DISPATCH_BACKOFF_CAP_MS: u64 = 5_000;
const DEFAULT_DISPATCH_BACKOFF_JITTER_MS: u64 = 120;
const TERMINAL_RECONNECT_BASE_MS: u64 = 500;
const TERMINAL_RECONNECT_CAP_MS: u64 = 5_000;
const TERMINAL_RECONNECT_MAX_ATTEMPTS: u32 = 10;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TargetMode {
    SshBootstrap,
    ReverseWs,
    UnixSocket,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TargetState {
    Ready,
    Leased,
    Quarantine,
    Tainted,
    Disabled,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LeaseState {
    Pending,
    Active,
    Released,
    Expired,
    Tainted,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DockerHealthStatus {
    Unknown,
    Running,
    Pass,
    Fail,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ControllerRole {
    Primary,
    WarmStandby,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DispatchStage {
    Resolve,
    Connect,
    VerifyIdentity,
    Auth,
    Dispatch,
}

#[derive(Clone, Debug)]
pub struct TargetIdentity {
    pub target_id: Uuid,
    pub display_name: String,
    pub mode: TargetMode,
    pub dns_name: Option<String>,
    pub ip_addr: Option<IpAddr>,
    pub ssh_hostkey_sha256_b64: Option<String>,
    pub mtls_cert_sha256_b64: Option<String>,
    pub unix_socket_path: Option<String>,
    pub state: TargetState,
    pub tainted: bool,
    pub taint_reason: Option<String>,
    pub health_score: f64,
    pub latency_ewma_ms: f64,
    pub recent_failure_rate: f64,
    pub cooldown_until: Option<Instant>,
    pub docker_health: DockerHealthStatus,
    pub docker_last_run_id: Option<Uuid>,
    pub docker_last_run_at_unix_ms: Option<i64>,
    pub docker_pass_count: u32,
    pub docker_fail_count: u32,
}

#[derive(Clone, Debug)]
pub struct LeaseRecord {
    pub lease_id: Uuid,
    pub target_id: Uuid,
    pub state: LeaseState,
    pub heartbeat_ttl: Duration,
    pub grace: Duration,
    pub expires_at: Instant,
    pub sequence_high_watermark: u64,
    pub last_heartbeat_at: Instant,
    pub owner_controller_id: Uuid,
    pub owner_controller_epoch: i64,
    pub lease_fence_token: i64,
}

#[derive(Clone, Debug)]
pub struct LeaseGrant {
    pub lease_id: Uuid,
    pub target_id: Uuid,
    pub heartbeat_ttl: Duration,
    pub grace: Duration,
    pub expires_at: Instant,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DispatchResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub response_payload: Option<serde_json::Value>,
}

#[derive(Clone, Debug)]
pub struct CommandInput {
    pub lease_id: Uuid,
    pub operation: String,
    pub payload: serde_json::Value,
    pub max_attempts: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentityProof {
    pub ssh_hostkey_sha256_b64: Option<String>,
    pub mtls_cert_sha256_b64: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DockerEvalCaseResult {
    pub case_name: String,
    pub status: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DockerEvalSummary {
    pub run_id: Uuid,
    pub target_id: Uuid,
    pub lease_id: Uuid,
    pub suite_name: String,
    pub status: DockerHealthStatus,
    pub passed_count: u32,
    pub failed_count: u32,
    pub started_at_unix_ms: i64,
    pub finished_at_unix_ms: i64,
    pub cases: Vec<DockerEvalCaseResult>,
}

#[derive(Clone, Debug)]
pub struct DockerEvalRequest {
    pub lease_id: Uuid,
    pub target_id: Uuid,
    pub suite_name: String,
}

#[derive(Clone, Debug)]
pub struct SecurityAlert {
    pub alert_id: Uuid,
    pub target_id: Option<Uuid>,
    pub lease_id: Option<Uuid>,
    pub severity: String,
    pub category: String,
    pub details: serde_json::Value,
    pub created_at_unix_ms: i64,
}

#[derive(Clone, Debug)]
pub struct TerminalGapMarker {
    pub target_id: Uuid,
    pub session_id: String,
    pub since_offset: Option<u64>,
    pub message: String,
    pub created_at_unix_ms: i64,
}

#[derive(Clone, Debug)]
pub struct ClockDriftAlert {
    pub target_id: Uuid,
    pub previous_buffer_ms: i64,
    pub next_buffer_ms: i64,
    pub rejection_count: u32,
    pub created_at_unix_ms: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TerminalStream {
    Stdout,
    Stderr,
    System,
}

#[derive(Clone, Debug)]
pub enum ControlPlaneEvent {
    TargetStatus {
        target_id: Uuid,
        display_name: String,
        mode: TargetMode,
        state: TargetState,
        tainted: bool,
        reason: Option<String>,
        health_score: f64,
        latency_ewma_ms: f64,
        recent_failure_rate: f64,
        docker_health: DockerHealthStatus,
        docker_pass_count: u32,
        docker_fail_count: u32,
        docker_last_run_at_unix_ms: Option<i64>,
    },
    FleetAlert {
        target_id: Option<Uuid>,
        lease_id: Option<Uuid>,
        category: String,
        message: String,
    },
    DockerEvalUpdate {
        target_id: Uuid,
        run_id: Uuid,
        docker_health: DockerHealthStatus,
        docker_pass_count: u32,
        docker_fail_count: u32,
        updated_at_unix_ms: i64,
    },
    TerminalGap {
        marker: TerminalGapMarker,
    },
    TerminalLine {
        target_id: Uuid,
        lease_id: Option<Uuid>,
        offset: u64,
        stream: TerminalStream,
        text: String,
        ts_unix_ms: i64,
    },
    ClockDrift {
        alert: ClockDriftAlert,
    },
    TargetRemoved {
        target_id: Uuid,
    },
}

#[derive(Clone, Debug)]
pub struct HaControlState {
    pub controller_id: Uuid,
    pub role: ControllerRole,
    pub controller_epoch: i64,
    pub lease_fence_token: i64,
    pub failover_timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct KeyAttestationMaterial {
    pub active_ssh_fingerprint: Option<String>,
    pub active_mtls_fingerprint: Option<String>,
    pub next_ssh_fingerprint: Option<String>,
    pub next_mtls_fingerprint: Option<String>,
    pub active_attestation_pubkey_b64: Option<String>,
    pub next_attestation_pubkey_b64: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyAttestationPayload {
    pub target_id: Uuid,
    pub challenge_nonce: String,
    pub old_key_signature_b64: String,
    pub candidate_key_signature_b64: String,
    pub candidate_ssh_fingerprint: Option<String>,
    pub candidate_mtls_fingerprint: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ConnectionManagerConfig {
    pub ssh_parallel_limit: usize,
    pub reaper_interval: Duration,
    pub controller_heartbeat_interval: Duration,
    pub max_dispatch_attempts: usize,
    pub dispatch_timeout: Duration,
    pub dispatch_backoff_base_ms: u64,
    pub dispatch_backoff_cap_ms: u64,
    pub dispatch_backoff_jitter_ms: u64,
    pub reverse_ws_ping_interval: Duration,
    pub reverse_ws_ping_jitter_pct: f64,
    pub reverse_ws_ping_timeout: Duration,
    pub reverse_ws_dead_after_misses: u8,
    pub clock_drift_rejection_threshold: u32,
    pub clock_drift_bump_ms: i64,
    pub clock_drift_recovery_window: Duration,
}

impl Default for ConnectionManagerConfig {
    fn default() -> Self {
        Self {
            ssh_parallel_limit: 8,
            reaper_interval: Duration::from_secs(1),
            controller_heartbeat_interval: Duration::from_secs(2),
            max_dispatch_attempts: DEFAULT_MAX_DISPATCH_ATTEMPTS,
            dispatch_timeout: Duration::from_secs(180),
            dispatch_backoff_base_ms: DEFAULT_DISPATCH_BACKOFF_BASE_MS,
            dispatch_backoff_cap_ms: DEFAULT_DISPATCH_BACKOFF_CAP_MS,
            dispatch_backoff_jitter_ms: DEFAULT_DISPATCH_BACKOFF_JITTER_MS,
            reverse_ws_ping_interval: Duration::from_secs(15),
            reverse_ws_ping_jitter_pct: 0.20,
            reverse_ws_ping_timeout: Duration::from_secs(4),
            reverse_ws_dead_after_misses: 3,
            clock_drift_rejection_threshold: 5,
            clock_drift_bump_ms: 2_000,
            clock_drift_recovery_window: Duration::from_secs(30),
        }
    }
}

#[async_trait]
pub trait Connector: Send + Sync {
    async fn connect(&self, target: &TargetIdentity) -> Result<()>;
    async fn authenticate(&self, target: &TargetIdentity) -> Result<()>;
    async fn probe_identity(
        &self,
        target: &TargetIdentity,
        endpoint: IpAddr,
    ) -> Result<IdentityProof>;
    async fn dispatch(
        &self,
        target: &TargetIdentity,
        endpoint: IpAddr,
        envelope: SignedEnvelope,
    ) -> Result<DispatchResult>;
}

#[derive(Clone)]
pub struct ConnectorRegistry {
    pub ssh: Arc<dyn Connector>,
    pub reverse_ws: Arc<dyn Connector>,
    pub unix_socket: Arc<dyn Connector>,
}

impl ConnectorRegistry {
    fn for_mode(&self, mode: TargetMode) -> Arc<dyn Connector> {
        match mode {
            TargetMode::SshBootstrap => self.ssh.clone(),
            TargetMode::ReverseWs => self.reverse_ws.clone(),
            TargetMode::UnixSocket => self.unix_socket.clone(),
        }
    }
}

#[async_trait]
pub trait ReverseTunnelTransport: Send + Sync {
    async fn ping(&self, target_id: Uuid) -> Result<()>;
    async fn reconnect(&self, target: &TargetIdentity, endpoint: IpAddr) -> Result<()>;
}

#[async_trait]
pub trait TerminalStreamBridge: Send + Sync {
    async fn reconnect_terminal_stream(
        &self,
        target_id: Uuid,
        session_id: &str,
        since_offset: Option<u64>,
    ) -> Result<Option<u64>>;
}

#[async_trait]
pub trait FleetStore: Send + Sync {
    async fn heartbeat_controller(&self, controller_id: Uuid, epoch: i64) -> Result<()>;
    async fn promote_if_stale(
        &self,
        controller_id: Uuid,
        expected_old_epoch: i64,
        failover_timeout: Duration,
    ) -> Result<(bool, i64, i64)>;
    async fn takeover_active_leases(
        &self,
        controller_id: Uuid,
        controller_epoch: i64,
        fence_token: i64,
    ) -> Result<u64>;
    async fn cas_lease_owner(
        &self,
        lease_id: Uuid,
        expected_epoch: i64,
        next_epoch: i64,
        next_fence_token: i64,
    ) -> Result<bool>;
    async fn update_target_docker_health(
        &self,
        target_id: Uuid,
        status: DockerHealthStatus,
        run_id: Uuid,
    ) -> Result<()>;
    async fn save_docker_eval_summary(&self, summary: &DockerEvalSummary) -> Result<()>;
    async fn load_target_attestation_material(
        &self,
        target_id: Uuid,
    ) -> Result<KeyAttestationMaterial>;
    async fn commit_attested_rotation(
        &self,
        target_id: Uuid,
        new_ssh: Option<String>,
        new_mtls: Option<String>,
    ) -> Result<()>;
    async fn record_security_alert(&self, alert: &SecurityAlert) -> Result<()>;
    async fn record_terminal_gap(&self, marker: &TerminalGapMarker) -> Result<()>;
    async fn record_clock_drift_alert(&self, alert: &ClockDriftAlert) -> Result<()>;
}

pub enum ManagerCommand {
    AcquireLease {
        ttl: Duration,
        grace: Duration,
        reply: oneshot::Sender<Result<LeaseGrant>>,
    },
    AcquireLeaseForTarget {
        target_id: Uuid,
        ttl: Duration,
        grace: Duration,
        reply: oneshot::Sender<Result<LeaseGrant>>,
    },
    Heartbeat {
        lease_id: Uuid,
        now: Instant,
        reply: oneshot::Sender<Result<()>>,
    },
    ReleaseLease {
        lease_id: Uuid,
        reason: String,
        reply: oneshot::Sender<Result<()>>,
    },
    SendCommand {
        cmd: CommandInput,
        reply: oneshot::Sender<Result<DispatchResult>>,
    },
    RunDockerEval {
        req: DockerEvalRequest,
        reply: oneshot::Sender<Result<DockerEvalSummary>>,
    },
    RotateTrustPins {
        lease_id: Uuid,
        target_id: Uuid,
        reply: oneshot::Sender<Result<()>>,
    },
    VerifyInboundEnvelope {
        envelope: SignedEnvelope,
        reply: oneshot::Sender<Result<VerificationMetadata>>,
    },
    RegisterTerminalSession {
        target_id: Uuid,
        session_id: String,
        since_offset: Option<u64>,
    },
    TerminalWsFailed {
        target_id: Uuid,
        session_id: String,
        since_offset: Option<u64>,
        error: String,
        sse_connected: bool,
    },
    PromoteStandby {
        reply: oneshot::Sender<Result<()>>,
    },
    ReverseTunnelKeepaliveTick {
        target_id: Uuid,
    },
    ControllerHeartbeatTick,
    ReapExpired {
        now: Instant,
    },
}

#[derive(Clone)]
pub struct ConnectionManagerHandle {
    tx: mpsc::Sender<ManagerCommand>,
    event_tx: broadcast::Sender<ControlPlaneEvent>,
}

impl ConnectionManagerHandle {
    pub fn subscribe_events(&self) -> broadcast::Receiver<ControlPlaneEvent> {
        self.event_tx.subscribe()
    }

    /// Broadcast a TargetRemoved event so SSE subscribers remove the target from their view.
    pub fn emit_target_removed(&self, target_id: Uuid) {
        let _ = self
            .event_tx
            .send(ControlPlaneEvent::TargetRemoved { target_id });
    }

    pub async fn acquire_lease(&self, ttl: Duration, grace: Duration) -> Result<LeaseGrant> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ManagerCommand::AcquireLease {
                ttl,
                grace,
                reply: reply_tx,
            })
            .await
            .context("manager loop unavailable")?;
        reply_rx.await.context("acquire lease channel closed")?
    }

    pub async fn acquire_lease_for_target(
        &self,
        target_id: Uuid,
        ttl: Duration,
        grace: Duration,
    ) -> Result<LeaseGrant> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ManagerCommand::AcquireLeaseForTarget {
                target_id,
                ttl,
                grace,
                reply: reply_tx,
            })
            .await
            .context("manager loop unavailable")?;
        reply_rx
            .await
            .context("acquire lease for target channel closed")?
    }

    pub async fn heartbeat(&self, lease_id: Uuid) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ManagerCommand::Heartbeat {
                lease_id,
                now: Instant::now(),
                reply: reply_tx,
            })
            .await
            .context("manager loop unavailable")?;
        reply_rx.await.context("heartbeat channel closed")?
    }

    pub async fn release_lease(&self, lease_id: Uuid, reason: impl Into<String>) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ManagerCommand::ReleaseLease {
                lease_id,
                reason: reason.into(),
                reply: reply_tx,
            })
            .await
            .context("manager loop unavailable")?;
        reply_rx.await.context("release channel closed")?
    }

    pub async fn send_command(&self, cmd: CommandInput) -> Result<DispatchResult> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ManagerCommand::SendCommand {
                cmd,
                reply: reply_tx,
            })
            .await
            .context("manager loop unavailable")?;
        reply_rx.await.context("send command channel closed")?
    }

    pub async fn run_docker_eval(&self, req: DockerEvalRequest) -> Result<DockerEvalSummary> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ManagerCommand::RunDockerEval {
                req,
                reply: reply_tx,
            })
            .await
            .context("manager loop unavailable")?;
        reply_rx.await.context("docker eval channel closed")?
    }

    pub async fn rotate_trust_pins(&self, lease_id: Uuid, target_id: Uuid) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ManagerCommand::RotateTrustPins {
                lease_id,
                target_id,
                reply: reply_tx,
            })
            .await
            .context("manager loop unavailable")?;
        reply_rx.await.context("rotation channel closed")?
    }

    pub async fn verify_inbound_envelope(
        &self,
        envelope: SignedEnvelope,
    ) -> Result<VerificationMetadata> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ManagerCommand::VerifyInboundEnvelope {
                envelope,
                reply: reply_tx,
            })
            .await
            .context("manager loop unavailable")?;
        reply_rx.await.context("verify channel closed")?
    }

    pub async fn register_terminal_session(
        &self,
        target_id: Uuid,
        session_id: impl Into<String>,
        since_offset: Option<u64>,
    ) -> Result<()> {
        self.tx
            .send(ManagerCommand::RegisterTerminalSession {
                target_id,
                session_id: session_id.into(),
                since_offset,
            })
            .await
            .context("manager loop unavailable")
    }

    pub async fn report_terminal_ws_failure(
        &self,
        target_id: Uuid,
        session_id: impl Into<String>,
        since_offset: Option<u64>,
        error: impl Into<String>,
        sse_connected: bool,
    ) -> Result<()> {
        self.tx
            .send(ManagerCommand::TerminalWsFailed {
                target_id,
                session_id: session_id.into(),
                since_offset,
                error: error.into(),
                sse_connected,
            })
            .await
            .context("manager loop unavailable")
    }

    pub async fn promote_standby(&self) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ManagerCommand::PromoteStandby { reply: reply_tx })
            .await
            .context("manager loop unavailable")?;
        reply_rx.await.context("promote standby channel closed")?
    }

    pub async fn reverse_tunnel_keepalive_tick(&self, target_id: Uuid) -> Result<()> {
        self.tx
            .send(ManagerCommand::ReverseTunnelKeepaliveTick { target_id })
            .await
            .context("manager loop unavailable")
    }
}

pub fn spawn_jittered_heartbeat_loop(
    manager: ConnectionManagerHandle,
    lease_id: Uuid,
    base_interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let wait_ms = jitter_ms(base_interval.as_millis() as u64, 0.10);
            sleep(Duration::from_millis(wait_ms)).await;
            if manager.heartbeat(lease_id).await.is_err() {
                break;
            }
        }
    })
}

pub fn spawn_reverse_tunnel_keepalive_loop(
    manager: ConnectionManagerHandle,
    target_id: Uuid,
    base_interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let wait_ms = jitter_ms(base_interval.as_millis() as u64, 0.20);
            sleep(Duration::from_millis(wait_ms)).await;
            if manager
                .reverse_tunnel_keepalive_tick(target_id)
                .await
                .is_err()
            {
                break;
            }
        }
    })
}

struct StageFailure {
    stage: DispatchStage,
    message: String,
    identity_mismatch: bool,
}

impl StageFailure {
    fn new(stage: DispatchStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
            identity_mismatch: false,
        }
    }

    fn identity_mismatch(stage: DispatchStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
            identity_mismatch: true,
        }
    }
}

#[derive(Clone, Debug)]
struct TerminalSessionState {
    target_id: Uuid,
    stale: bool,
    last_offset: Option<u64>,
}

pub struct ConnectionManager {
    targets: HashMap<Uuid, TargetIdentity>,
    leases: HashMap<Uuid, LeaseRecord>,
    connectors: ConnectorRegistry,
    signer: Arc<DualKeyHmacEnvelopeSigner>,
    store: Arc<dyn FleetStore>,
    reverse_tunnel: Option<Arc<dyn ReverseTunnelTransport>>,
    terminal_stream_bridge: Option<Arc<dyn TerminalStreamBridge>>,
    ssh_fanout_limit: Arc<tokio::sync::Semaphore>,
    event_tx: broadcast::Sender<ControlPlaneEvent>,
    config: ConnectionManagerConfig,
    ha: HaControlState,
    reverse_tunnel_misses: HashMap<Uuid, u8>,
    reverse_tunnel_dead: HashSet<Uuid>,
    terminal_sessions: HashMap<String, TerminalSessionState>,
    terminal_offsets: HashMap<Uuid, u64>,
    drift_rejections: HashMap<Uuid, u32>,
    drift_stabilize_until_mono_ms: HashMap<Uuid, i64>,
}

impl ConnectionManager {
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        initial_targets: Vec<TargetIdentity>,
        connectors: ConnectorRegistry,
        signer: Arc<DualKeyHmacEnvelopeSigner>,
        store: Arc<dyn FleetStore>,
        reverse_tunnel: Option<Arc<dyn ReverseTunnelTransport>>,
        terminal_stream_bridge: Option<Arc<dyn TerminalStreamBridge>>,
        ha_state: HaControlState,
        config: ConnectionManagerConfig,
    ) -> ConnectionManagerHandle {
        let (tx, mut rx) = mpsc::channel::<ManagerCommand>(2048);
        let (event_tx, _) = broadcast::channel::<ControlPlaneEvent>(2048);

        let reverse_targets: Vec<Uuid> = initial_targets
            .iter()
            .filter(|target| target.mode == TargetMode::ReverseWs)
            .map(|target| target.target_id)
            .collect();

        let mut manager = ConnectionManager {
            targets: initial_targets
                .into_iter()
                .map(|target| (target.target_id, target))
                .collect(),
            leases: HashMap::new(),
            connectors,
            signer,
            store,
            reverse_tunnel,
            terminal_stream_bridge,
            ssh_fanout_limit: Arc::new(tokio::sync::Semaphore::new(
                config.ssh_parallel_limit.max(1),
            )),
            event_tx: event_tx.clone(),
            config: config.clone(),
            ha: ha_state,
            reverse_tunnel_misses: HashMap::new(),
            reverse_tunnel_dead: HashSet::new(),
            terminal_sessions: HashMap::new(),
            terminal_offsets: HashMap::new(),
            drift_rejections: HashMap::new(),
            drift_stabilize_until_mono_ms: HashMap::new(),
        };

        let handle = ConnectionManagerHandle {
            tx: tx.clone(),
            event_tx: event_tx.clone(),
        };

        let reaper_tick = config.reaper_interval;
        let tx_reaper = tx.clone();
        tokio::spawn(async move {
            let mut ticker = interval_at(Instant::now() + reaper_tick, reaper_tick);
            loop {
                ticker.tick().await;
                if tx_reaper
                    .send(ManagerCommand::ReapExpired {
                        now: Instant::now(),
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        let controller_tick = config.controller_heartbeat_interval;
        let tx_controller = tx.clone();
        tokio::spawn(async move {
            let mut ticker = interval_at(Instant::now() + controller_tick, controller_tick);
            loop {
                ticker.tick().await;
                if tx_controller
                    .send(ManagerCommand::ControllerHeartbeatTick)
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    ManagerCommand::AcquireLease { ttl, grace, reply } => {
                        let _ = reply.send(manager.handle_acquire_lease(ttl, grace));
                    }
                    ManagerCommand::AcquireLeaseForTarget {
                        target_id,
                        ttl,
                        grace,
                        reply,
                    } => {
                        let _ = reply
                            .send(manager.handle_acquire_lease_for_target(target_id, ttl, grace));
                    }
                    ManagerCommand::Heartbeat {
                        lease_id,
                        now,
                        reply,
                    } => {
                        let _ = reply.send(manager.handle_heartbeat(lease_id, now).await);
                    }
                    ManagerCommand::ReleaseLease {
                        lease_id,
                        reason,
                        reply,
                    } => {
                        let _ = reply.send(manager.handle_release(lease_id, reason).await);
                    }
                    ManagerCommand::SendCommand { cmd, reply } => {
                        let _ = reply.send(manager.handle_send_command(cmd).await);
                    }
                    ManagerCommand::RunDockerEval { req, reply } => {
                        let _ = reply.send(manager.handle_run_docker_eval(req).await);
                    }
                    ManagerCommand::RotateTrustPins {
                        lease_id,
                        target_id,
                        reply,
                    } => {
                        let _ =
                            reply.send(manager.handle_rotate_trust_pins(lease_id, target_id).await);
                    }
                    ManagerCommand::VerifyInboundEnvelope { envelope, reply } => {
                        let _ = reply.send(manager.handle_verify_inbound_envelope(&envelope).await);
                    }
                    ManagerCommand::RegisterTerminalSession {
                        target_id,
                        session_id,
                        since_offset,
                    } => {
                        manager.handle_register_terminal_session(
                            target_id,
                            session_id,
                            since_offset,
                        );
                    }
                    ManagerCommand::TerminalWsFailed {
                        target_id,
                        session_id,
                        since_offset,
                        error,
                        sse_connected,
                    } => {
                        manager
                            .handle_terminal_ws_failed(
                                target_id,
                                session_id,
                                since_offset,
                                error,
                                sse_connected,
                            )
                            .await;
                    }
                    ManagerCommand::PromoteStandby { reply } => {
                        let _ = reply.send(manager.handle_promote_standby().await);
                    }
                    ManagerCommand::ReverseTunnelKeepaliveTick { target_id } => {
                        if let Err(err) = manager
                            .handle_reverse_tunnel_keepalive_tick(target_id)
                            .await
                        {
                            warn!(target_id = %target_id, error = %err, "reverse tunnel keepalive tick failed");
                        }
                    }
                    ManagerCommand::ControllerHeartbeatTick => {
                        if let Err(err) = manager.handle_controller_heartbeat_tick().await {
                            warn!(error = %err, "controller heartbeat tick failed");
                        }
                    }
                    ManagerCommand::ReapExpired { now } => {
                        manager.handle_reap_expired(now).await;
                    }
                }
            }
        });

        for target_id in reverse_targets {
            let _jh = spawn_reverse_tunnel_keepalive_loop(
                handle.clone(),
                target_id,
                config.reverse_ws_ping_interval,
            );
        }

        handle
    }

    fn score(target: &TargetIdentity) -> f64 {
        let health = target.health_score.clamp(0.0, 1.0);
        let latency_component = 1.0 / (1.0 + target.latency_ewma_ms.max(0.0) / 100.0);
        let failure_component = 1.0 - target.recent_failure_rate.clamp(0.0, 1.0);
        (0.50 * health) + (0.30 * latency_component) + (0.20 * failure_component)
    }

    async fn handle_controller_heartbeat_tick(&self) -> Result<()> {
        if self.ha.role == ControllerRole::Primary {
            self.store
                .heartbeat_controller(self.ha.controller_id, self.ha.controller_epoch)
                .await
                .context("heartbeat_controller failed")?;
        }
        Ok(())
    }

    fn handle_acquire_lease(&mut self, ttl: Duration, grace: Duration) -> Result<LeaseGrant> {
        let target_id = self
            .targets
            .values()
            .filter(|target| target.state == TargetState::Ready)
            .max_by(|a, b| {
                Self::score(a)
                    .partial_cmp(&Self::score(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|target| target.target_id)
            .ok_or_else(|| anyhow!("no ready target available"))?;

        self.acquire_lease_for_target_internal(target_id, ttl, grace)
    }

    fn handle_acquire_lease_for_target(
        &mut self,
        target_id: Uuid,
        ttl: Duration,
        grace: Duration,
    ) -> Result<LeaseGrant> {
        self.acquire_lease_for_target_internal(target_id, ttl, grace)
    }

    fn acquire_lease_for_target_internal(
        &mut self,
        target_id: Uuid,
        ttl: Duration,
        grace: Duration,
    ) -> Result<LeaseGrant> {
        let now = Instant::now();
        let target = self
            .targets
            .get_mut(&target_id)
            .ok_or_else(|| anyhow!("target '{}' is not enrolled", target_id))?;

        if target.state != TargetState::Ready {
            return Err(anyhow!(
                "target '{}' is not ready for lease acquisition (state={:?})",
                target.display_name,
                target.state
            ));
        }

        target.state = TargetState::Leased;
        target.taint_reason = None;

        let lease_id = Uuid::new_v4();
        let expires_at = now + ttl;
        let lease = LeaseRecord {
            lease_id,
            target_id,
            state: LeaseState::Active,
            heartbeat_ttl: ttl,
            grace,
            expires_at,
            sequence_high_watermark: 0,
            last_heartbeat_at: now,
            owner_controller_id: self.ha.controller_id,
            owner_controller_epoch: self.ha.controller_epoch,
            lease_fence_token: self.ha.lease_fence_token,
        };
        self.leases.insert(lease_id, lease);

        self.emit_target_status(target_id);

        Ok(LeaseGrant {
            lease_id,
            target_id,
            heartbeat_ttl: ttl,
            grace,
            expires_at,
        })
    }

    async fn handle_heartbeat(&mut self, lease_id: Uuid, now: Instant) -> Result<()> {
        let mut expired = None;
        if let Some(lease) = self.leases.get_mut(&lease_id) {
            if lease.state != LeaseState::Active {
                return Err(anyhow!("lease not active"));
            }

            if now > lease.expires_at + lease.grace {
                lease.state = LeaseState::Expired;
                expired = Some((lease.target_id, lease.lease_id));
            } else {
                lease.last_heartbeat_at = now;
                lease.expires_at = now + lease.heartbeat_ttl;
            }
        } else {
            return Err(anyhow!("invalid lease"));
        }

        if let Some((target_id, expired_lease_id)) = expired {
            self.quarantine_and_taint(
                target_id,
                expired_lease_id,
                "heartbeat_timeout",
                "heartbeat_timeout",
                serde_json::json!({ "protocol": "12.1" }),
            )
            .await;
            return Err(anyhow!("lease expired and target quarantined"));
        }

        Ok(())
    }

    async fn handle_release(&mut self, lease_id: Uuid, reason: String) -> Result<()> {
        let target_id = {
            let lease = self
                .leases
                .get_mut(&lease_id)
                .ok_or_else(|| anyhow!("invalid lease"))?;

            if lease.state == LeaseState::Active {
                lease.state = LeaseState::Released;
            }

            lease.target_id
        };

        if let Some(target) = self.targets.get_mut(&target_id) {
            let mut should_emit = false;
            if self
                .leases
                .get(&lease_id)
                .map(|lease| lease.state == LeaseState::Released)
                .unwrap_or(false)
                && target.state == TargetState::Leased
            {
                target.state = TargetState::Ready;
                target.taint_reason = Some(reason);
                should_emit = true;
            }

            if should_emit {
                self.emit_target_status(target_id);
            }
        }

        Ok(())
    }

    async fn handle_reap_expired(&mut self, now: Instant) {
        let expired: Vec<(Uuid, Uuid)> = self
            .leases
            .iter()
            .filter_map(|(lease_id, lease)| {
                if lease.state == LeaseState::Active && now > lease.expires_at + lease.grace {
                    Some((*lease_id, lease.target_id))
                } else {
                    None
                }
            })
            .collect();

        for (lease_id, target_id) in expired {
            self.quarantine_and_taint(
                target_id,
                lease_id,
                "lease_reaped_expired",
                "lease_expired",
                serde_json::json!({ "protocol": "12.1", "path": "reaper" }),
            )
            .await;
        }
    }

    async fn handle_send_command(&mut self, cmd: CommandInput) -> Result<DispatchResult> {
        self.dispatch_with_retry(
            cmd.lease_id,
            cmd.operation,
            cmd.payload,
            cmd.max_attempts
                .unwrap_or(self.config.max_dispatch_attempts),
        )
        .await
    }

    async fn dispatch_with_retry(
        &mut self,
        lease_id: Uuid,
        operation: String,
        payload: serde_json::Value,
        max_attempts: usize,
    ) -> Result<DispatchResult> {
        let (target_id, ttl, sequence) = {
            let lease = self
                .leases
                .get_mut(&lease_id)
                .ok_or_else(|| anyhow!("invalid lease"))?;

            if lease.state != LeaseState::Active {
                return Err(anyhow!("lease not active"));
            }

            if Instant::now() > lease.expires_at + lease.grace {
                lease.state = LeaseState::Expired;
                let target_id = lease.target_id;
                let lease_id = lease.lease_id;
                self.quarantine_and_taint(
                    target_id,
                    lease_id,
                    "command_after_expiry",
                    "lease_expired",
                    serde_json::json!({ "protocol": "12.1", "operation": operation }),
                )
                .await;
                return Err(anyhow!("lease expired before command dispatch"));
            }

            lease.sequence_high_watermark += 1;
            (
                lease.target_id,
                lease.heartbeat_ttl,
                lease.sequence_high_watermark,
            )
        };

        let target = self
            .targets
            .get(&target_id)
            .cloned()
            .ok_or_else(|| anyhow!("target missing for active lease"))?;

        if target.mode == TargetMode::ReverseWs
            && self.reverse_tunnel_dead.contains(&target.target_id)
        {
            return Err(anyhow!(
                "reverse tunnel marked dead; command dispatch halted until reconnect"
            ));
        }

        let connector = self.connectors.for_mode(target.mode);
        let mut backoff_ms = self.config.dispatch_backoff_base_ms;
        let attempts = max_attempts.max(1);

        for attempt in 1..=attempts {
            let envelope = self
                .signer
                .sign(SignedEnvelopeInput {
                    target_id: target.target_id,
                    lease_id,
                    nonce: Uuid::new_v4().to_string(),
                    sequence,
                    op: operation.clone(),
                    payload: payload.clone(),
                    ttl,
                    drift_buffer_ms: DEFAULT_DRIFT_BUFFER_MS,
                })
                .await
                .context("failed to sign command envelope")?;

            let endpoint = match self.resolve_and_verify_identity(&target).await {
                Ok(endpoint) => endpoint,
                Err(stage_failure) => {
                    self.mark_dispatch_failure(target.target_id);
                    if stage_failure.identity_mismatch {
                        self.quarantine_and_taint(
                            target.target_id,
                            lease_id,
                            "identity_pin_mismatch",
                            "identity_mismatch",
                            serde_json::json!({
                                "protocol": "12.2",
                                "stage": format!("{:?}", stage_failure.stage),
                                "message": stage_failure.message,
                            }),
                        )
                        .await;
                        return Err(anyhow!(
                            "identity mismatch for target {} during {}",
                            target.display_name,
                            operation
                        ));
                    }

                    if attempt == attempts {
                        self.quarantine_and_taint(
                            target.target_id,
                            lease_id,
                            "identity_resolution_failed",
                            "handshake_timeout",
                            serde_json::json!({
                                "protocol": "12.1",
                                "stage": format!("{:?}", stage_failure.stage),
                                "message": stage_failure.message,
                                "attempt": attempt,
                            }),
                        )
                        .await;
                        return Err(anyhow!("dispatch failed after identity resolution retries"));
                    }

                    self.sleep_with_backoff(&mut backoff_ms).await;
                    continue;
                }
            };

            match self
                .connect_auth_dispatch(connector.clone(), &target, endpoint, envelope)
                .await
            {
                Ok(result) => {
                    self.mark_dispatch_success(target.target_id, result.duration_ms);
                    self.emit_command_terminal_output(target.target_id, Some(lease_id), &result);
                    self.emit_target_status(target.target_id);
                    return Ok(result);
                }
                Err(stage_failure) => {
                    self.mark_dispatch_failure(target.target_id);
                    self.emit_target_status(target.target_id);

                    if attempt == attempts {
                        self.emit_terminal_line(
                            target.target_id,
                            Some(lease_id),
                            TerminalStream::System,
                            format!(
                                "dispatch stage {:?} failed after max retries: {}",
                                stage_failure.stage, stage_failure.message
                            ),
                        );
                        self.quarantine_and_taint(
                            target.target_id,
                            lease_id,
                            "dispatch_failed",
                            "handshake_timeout",
                            serde_json::json!({
                                "protocol": "12.1",
                                "stage": format!("{:?}", stage_failure.stage),
                                "message": stage_failure.message,
                                "attempt": attempt,
                            }),
                        )
                        .await;
                        return Err(anyhow!(
                            "dispatch stage {:?} failed after max retries: {}",
                            stage_failure.stage,
                            stage_failure.message
                        ));
                    }

                    self.sleep_with_backoff(&mut backoff_ms).await;
                }
            }
        }

        Err(anyhow!("dispatch retry loop exhausted"))
    }

    async fn connect_auth_dispatch(
        &self,
        connector: Arc<dyn Connector>,
        target: &TargetIdentity,
        endpoint: IpAddr,
        envelope: SignedEnvelope,
    ) -> std::result::Result<DispatchResult, StageFailure> {
        let maybe_permit = if target.mode == TargetMode::SshBootstrap {
            match self.ssh_fanout_limit.clone().acquire_owned().await {
                Ok(permit) => Some(permit),
                Err(err) => {
                    return Err(StageFailure::new(
                        DispatchStage::Connect,
                        format!("ssh fanout semaphore closed: {err}"),
                    ));
                }
            }
        } else {
            None
        };

        let connect = timeout(Duration::from_secs(4), connector.connect(target)).await;
        match connect {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                drop(maybe_permit);
                return Err(StageFailure::new(
                    DispatchStage::Connect,
                    format!("connect stage failed: {err}"),
                ));
            }
            Err(_) => {
                drop(maybe_permit);
                return Err(StageFailure::new(
                    DispatchStage::Connect,
                    "connect stage timeout",
                ));
            }
        }

        let auth = timeout(Duration::from_secs(4), connector.authenticate(target)).await;
        match auth {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                drop(maybe_permit);
                return Err(StageFailure::new(
                    DispatchStage::Auth,
                    format!("auth stage failed: {err}"),
                ));
            }
            Err(_) => {
                drop(maybe_permit);
                return Err(StageFailure::new(DispatchStage::Auth, "auth stage timeout"));
            }
        }

        let dispatched = timeout(
            self.config.dispatch_timeout,
            connector.dispatch(target, endpoint, envelope),
        )
        .await;
        drop(maybe_permit);

        match dispatched {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(err)) => Err(StageFailure::new(
                DispatchStage::Dispatch,
                format!("dispatch stage failed: {err}"),
            )),
            Err(_) => Err(StageFailure::new(
                DispatchStage::Dispatch,
                "dispatch stage timeout",
            )),
        }
    }

    async fn resolve_and_verify_identity(
        &self,
        target: &TargetIdentity,
    ) -> std::result::Result<IpAddr, StageFailure> {
        match target.mode {
            TargetMode::UnixSocket => {
                if target.unix_socket_path.is_none() {
                    return Err(StageFailure::new(
                        DispatchStage::Resolve,
                        "unix socket path missing",
                    ));
                }
                Ok(target.ip_addr.unwrap_or(IpAddr::from([127, 0, 0, 1])))
            }
            TargetMode::SshBootstrap | TargetMode::ReverseWs => {
                let dns_name = match target.dns_name.as_ref() {
                    Some(value) => value,
                    None => {
                        return Err(StageFailure::new(
                            DispatchStage::Resolve,
                            "dns name missing for remote mode",
                        ));
                    }
                };

                let port = if target.mode == TargetMode::SshBootstrap {
                    22
                } else {
                    443
                };

                let mut resolved = match timeout(
                    Duration::from_secs(2),
                    lookup_host((dns_name.as_str(), port)),
                )
                .await
                {
                    Ok(Ok(addrs)) => addrs,
                    Ok(Err(err)) => {
                        return Err(StageFailure::new(
                            DispatchStage::Resolve,
                            format!("dns lookup failed: {err}"),
                        ));
                    }
                    Err(_) => {
                        return Err(StageFailure::new(
                            DispatchStage::Resolve,
                            "dns lookup timeout",
                        ));
                    }
                };

                let endpoint = match resolved.next() {
                    Some(socket_addr) => socket_addr.ip(),
                    None => {
                        return Err(StageFailure::new(
                            DispatchStage::Resolve,
                            "dns lookup returned no address records",
                        ));
                    }
                };

                let connector = self.connectors.for_mode(target.mode);
                let proof = match timeout(
                    Duration::from_secs(4),
                    connector.probe_identity(target, endpoint),
                )
                .await
                {
                    Ok(Ok(proof)) => proof,
                    Ok(Err(err)) => {
                        return Err(StageFailure::new(
                            DispatchStage::VerifyIdentity,
                            format!("identity probe failed: {err}"),
                        ));
                    }
                    Err(_) => {
                        return Err(StageFailure::new(
                            DispatchStage::VerifyIdentity,
                            "identity probe timeout",
                        ));
                    }
                };

                if target.mode == TargetMode::SshBootstrap {
                    let expected = match target.ssh_hostkey_sha256_b64.as_ref() {
                        Some(value) => value,
                        None => {
                            return Err(StageFailure::new(
                                DispatchStage::VerifyIdentity,
                                "pinned SSH fingerprint missing",
                            ));
                        }
                    };
                    let observed = match proof.ssh_hostkey_sha256_b64 {
                        Some(value) => value,
                        None => {
                            return Err(StageFailure::new(
                                DispatchStage::VerifyIdentity,
                                "SSH proof missing host key fingerprint",
                            ));
                        }
                    };
                    if observed != *expected {
                        return Err(StageFailure::identity_mismatch(
                            DispatchStage::VerifyIdentity,
                            "ssh host key mismatch",
                        ));
                    }
                }

                if target.mode == TargetMode::ReverseWs {
                    let expected = match target.mtls_cert_sha256_b64.as_ref() {
                        Some(value) => value,
                        None => {
                            return Err(StageFailure::new(
                                DispatchStage::VerifyIdentity,
                                "pinned mTLS fingerprint missing",
                            ));
                        }
                    };
                    let observed = match proof.mtls_cert_sha256_b64 {
                        Some(value) => value,
                        None => {
                            return Err(StageFailure::new(
                                DispatchStage::VerifyIdentity,
                                "reverse tunnel proof missing mTLS fingerprint",
                            ));
                        }
                    };
                    if observed != *expected {
                        return Err(StageFailure::identity_mismatch(
                            DispatchStage::VerifyIdentity,
                            "mTLS fingerprint mismatch",
                        ));
                    }
                }

                Ok(endpoint)
            }
        }
    }

    async fn sleep_with_backoff(&self, backoff_ms: &mut u64) {
        let jitter = rand::thread_rng().gen_range(0..=self.config.dispatch_backoff_jitter_ms);
        let sleep_ms = (*backoff_ms + jitter).min(self.config.dispatch_backoff_cap_ms);
        sleep(Duration::from_millis(sleep_ms)).await;
        *backoff_ms = (*backoff_ms * 2).min(self.config.dispatch_backoff_cap_ms);
    }

    fn mark_dispatch_success(&mut self, target_id: Uuid, duration_ms: u64) {
        if let Some(target) = self.targets.get_mut(&target_id) {
            let observed = duration_ms as f64;
            if target.latency_ewma_ms <= 0.0 {
                target.latency_ewma_ms = observed;
            } else {
                target.latency_ewma_ms = (target.latency_ewma_ms * 0.8) + (observed * 0.2);
            }
            target.recent_failure_rate = (target.recent_failure_rate * 0.9).clamp(0.0, 1.0);
        }
    }

    fn mark_dispatch_failure(&mut self, target_id: Uuid) {
        if let Some(target) = self.targets.get_mut(&target_id) {
            target.recent_failure_rate = (target.recent_failure_rate + 0.05).clamp(0.0, 1.0);
        }
    }

    fn next_terminal_offset(&mut self, target_id: Uuid) -> u64 {
        let next = self.terminal_offsets.entry(target_id).or_insert(0);
        let current = *next;
        *next = next.saturating_add(1);
        current
    }

    fn emit_terminal_line(
        &mut self,
        target_id: Uuid,
        lease_id: Option<Uuid>,
        stream: TerminalStream,
        text: String,
    ) {
        let offset = self.next_terminal_offset(target_id);
        let _ = self.event_tx.send(ControlPlaneEvent::TerminalLine {
            target_id,
            lease_id,
            offset,
            stream,
            text,
            ts_unix_ms: now_unix_ms(),
        });
    }

    fn emit_command_terminal_output(
        &mut self,
        target_id: Uuid,
        lease_id: Option<Uuid>,
        result: &DispatchResult,
    ) {
        for line in result.stdout.lines() {
            if line.trim().is_empty() {
                continue;
            }
            self.emit_terminal_line(
                target_id,
                lease_id,
                TerminalStream::Stdout,
                line.to_string(),
            );
        }

        for line in result.stderr.lines() {
            if line.trim().is_empty() {
                continue;
            }
            self.emit_terminal_line(
                target_id,
                lease_id,
                TerminalStream::Stderr,
                line.to_string(),
            );
        }
    }

    async fn handle_run_docker_eval(
        &mut self,
        req: DockerEvalRequest,
    ) -> Result<DockerEvalSummary> {
        let lease = self
            .leases
            .get(&req.lease_id)
            .ok_or_else(|| anyhow!("invalid lease"))?
            .clone();

        if lease.state != LeaseState::Active {
            return Err(anyhow!("lease not active"));
        }

        if lease.target_id != req.target_id {
            return Err(anyhow!(
                "docker eval target does not match active lease owner"
            ));
        }

        let run_id = Uuid::new_v4();
        let started_at_unix_ms = now_unix_ms();

        if let Some(target) = self.targets.get_mut(&req.target_id) {
            target.docker_health = DockerHealthStatus::Running;
            target.docker_last_run_id = Some(run_id);
            target.docker_last_run_at_unix_ms = Some(started_at_unix_ms);
        }

        if let Err(err) = self
            .store
            .update_target_docker_health(req.target_id, DockerHealthStatus::Running, run_id)
            .await
        {
            warn!(target_id = %req.target_id, error = %err, "failed to persist docker health running state");
        }

        let mut cases = Vec::new();
        for (case_name, shell) in docker_eval_suite_commands() {
            let started = Instant::now();
            let command_payload = serde_json::json!({
                "case_name": case_name,
                "shell": shell,
                "timeout_ms": 120_000,
            });

            let result = self
                .dispatch_with_retry(
                    req.lease_id,
                    "docker_eval.run_case".to_string(),
                    command_payload,
                    self.config.max_dispatch_attempts,
                )
                .await;

            match result {
                Ok(value) => {
                    cases.push(DockerEvalCaseResult {
                        case_name: case_name.to_string(),
                        status: if value.exit_code == 0 {
                            "passed".to_string()
                        } else {
                            "failed".to_string()
                        },
                        exit_code: value.exit_code,
                        stdout: value.stdout,
                        stderr: value.stderr,
                        duration_ms: started.elapsed().as_millis() as u64,
                    });
                }
                Err(err) => {
                    cases.push(DockerEvalCaseResult {
                        case_name: case_name.to_string(),
                        status: "failed".to_string(),
                        exit_code: 1,
                        stdout: String::new(),
                        stderr: err.to_string(),
                        duration_ms: started.elapsed().as_millis() as u64,
                    });
                }
            }
        }

        let passed_count = cases.iter().filter(|case| case.status == "passed").count() as u32;
        let failed_count = cases.len() as u32 - passed_count;
        let status = if failed_count == 0 {
            DockerHealthStatus::Pass
        } else {
            DockerHealthStatus::Fail
        };
        let finished_at_unix_ms = now_unix_ms();

        let summary = DockerEvalSummary {
            run_id,
            target_id: req.target_id,
            lease_id: req.lease_id,
            suite_name: req.suite_name,
            status,
            passed_count,
            failed_count,
            started_at_unix_ms,
            finished_at_unix_ms,
            cases,
        };

        if let Some(target) = self.targets.get_mut(&req.target_id) {
            target.docker_health = summary.status;
            target.docker_last_run_id = Some(summary.run_id);
            target.docker_last_run_at_unix_ms = Some(summary.finished_at_unix_ms);
            target.docker_pass_count = summary.passed_count;
            target.docker_fail_count = summary.failed_count;
        }

        if let Err(err) = self.store.save_docker_eval_summary(&summary).await {
            warn!(target_id = %req.target_id, error = %err, "failed to persist docker eval summary");
        }

        if let Err(err) = self
            .store
            .update_target_docker_health(req.target_id, summary.status, run_id)
            .await
        {
            warn!(target_id = %req.target_id, error = %err, "failed to persist docker eval health status");
        }

        let _ = self.event_tx.send(ControlPlaneEvent::DockerEvalUpdate {
            target_id: summary.target_id,
            run_id: summary.run_id,
            docker_health: summary.status,
            docker_pass_count: summary.passed_count,
            docker_fail_count: summary.failed_count,
            updated_at_unix_ms: summary.finished_at_unix_ms,
        });

        Ok(summary)
    }

    async fn handle_rotate_trust_pins(&mut self, lease_id: Uuid, target_id: Uuid) -> Result<()> {
        let material = self
            .store
            .load_target_attestation_material(target_id)
            .await
            .context("load_target_attestation_material failed")?;

        let nonce = Uuid::new_v4().to_string();
        let response = self
            .dispatch_with_retry(
                lease_id,
                "trust.rotate_attest".to_string(),
                serde_json::json!({ "nonce": nonce }),
                self.config.max_dispatch_attempts,
            )
            .await
            .context("trust.rotate_attest dispatch failed")?;

        let attestation_value = if let Some(payload) = response.response_payload {
            payload
        } else {
            serde_json::from_str::<serde_json::Value>(response.stdout.as_str())
                .context("attestation response missing response_payload and valid JSON stdout")?
        };

        let attestation: KeyAttestationPayload = serde_json::from_value(attestation_value)
            .context("failed to parse key attestation payload")?;

        if attestation.challenge_nonce != nonce {
            self.quarantine_and_taint(
                target_id,
                lease_id,
                "attestation_nonce_mismatch",
                "trust_attestation",
                serde_json::json!({ "protocol": "7.1.manual_trust_overhead" }),
            )
            .await;
            return Err(anyhow!("attestation nonce mismatch"));
        }

        if let Err(err) = verify_rotation_attestation(target_id, &material, &attestation) {
            self.quarantine_and_taint(
                target_id,
                lease_id,
                "attestation_signature_invalid",
                "trust_attestation",
                serde_json::json!({ "error": err.to_string() }),
            )
            .await;
            return Err(err);
        }

        self.store
            .commit_attested_rotation(
                target_id,
                attestation.candidate_ssh_fingerprint,
                attestation.candidate_mtls_fingerprint,
            )
            .await
            .context("commit_attested_rotation failed")?;

        Ok(())
    }

    async fn handle_verify_inbound_envelope(
        &mut self,
        envelope: &SignedEnvelope,
    ) -> Result<VerificationMetadata> {
        match self.signer.verify(envelope).await {
            Ok(metadata) => {
                self.maybe_revert_clock_drift_override(envelope.target_id)
                    .await;
                Ok(metadata)
            }
            Err(SignerError::IssuedInFutureWindow) | Err(SignerError::EnvelopeExpired) => {
                self.handle_clock_drift_rejection(envelope.target_id).await;
                Err(anyhow!("clock drift verification rejection"))
            }
            Err(SignerError::ReplayNonceDetected)
            | Err(SignerError::NonMonotonicSequence { .. }) => {
                self.quarantine_and_taint(
                    envelope.target_id,
                    envelope.lease_id,
                    "replay_detected",
                    "replay_detection",
                    serde_json::json!({
                        "protocol": "7.fail_closed",
                        "nonce": envelope.nonce,
                        "sequence": envelope.sequence,
                    }),
                )
                .await;
                Err(anyhow!("replay detection triggered fail-closed quarantine"))
            }
            Err(err) => Err(anyhow!("envelope verification failed: {err}")),
        }
    }

    async fn handle_clock_drift_rejection(&mut self, target_id: Uuid) {
        let counter = self.drift_rejections.entry(target_id).or_insert(0);
        *counter += 1;

        if *counter < self.config.clock_drift_rejection_threshold {
            return;
        }

        let previous_buffer_ms = self.signer.target_drift_buffer_ms(target_id).await;
        let next_buffer_ms = (previous_buffer_ms + self.config.clock_drift_bump_ms)
            .clamp(DEFAULT_DRIFT_BUFFER_MS, MAX_DRIFT_BUFFER_MS);

        self.signer
            .set_target_drift_buffer_ms(target_id, next_buffer_ms)
            .await;
        self.drift_stabilize_until_mono_ms.insert(
            target_id,
            now_mono_ms() + self.config.clock_drift_recovery_window.as_millis() as i64,
        );

        let alert = ClockDriftAlert {
            target_id,
            previous_buffer_ms,
            next_buffer_ms,
            rejection_count: *counter,
            created_at_unix_ms: now_unix_ms(),
        };

        if let Err(err) = self.store.record_clock_drift_alert(&alert).await {
            warn!(target_id = %target_id, error = %err, "failed to persist clock drift alert");
        }

        let _ = self.event_tx.send(ControlPlaneEvent::ClockDrift { alert });
        *counter = 0;
    }

    async fn maybe_revert_clock_drift_override(&mut self, target_id: Uuid) {
        let Some(stabilize_until) = self.drift_stabilize_until_mono_ms.get(&target_id).copied()
        else {
            return;
        };

        if now_mono_ms() < stabilize_until {
            return;
        }

        let current = self.signer.target_drift_buffer_ms(target_id).await;
        if current > DEFAULT_DRIFT_BUFFER_MS {
            self.signer
                .set_target_drift_buffer_ms(target_id, DEFAULT_DRIFT_BUFFER_MS)
                .await;
            let _ = self.event_tx.send(ControlPlaneEvent::FleetAlert {
                target_id: Some(target_id),
                lease_id: None,
                category: "clock_drift_recovered".to_string(),
                message: format!(
                    "drift buffer reverted from {current}ms to {}ms after stability window",
                    DEFAULT_DRIFT_BUFFER_MS
                ),
            });
        }

        self.drift_stabilize_until_mono_ms.remove(&target_id);
        self.drift_rejections.remove(&target_id);
    }

    fn handle_register_terminal_session(
        &mut self,
        target_id: Uuid,
        session_id: String,
        since_offset: Option<u64>,
    ) {
        self.terminal_sessions.insert(
            session_id,
            TerminalSessionState {
                target_id,
                stale: false,
                last_offset: since_offset,
            },
        );
    }

    async fn handle_terminal_ws_failed(
        &mut self,
        target_id: Uuid,
        session_id: String,
        since_offset: Option<u64>,
        error_message: String,
        sse_connected: bool,
    ) {
        if let Some(existing) = self.terminal_sessions.get(session_id.as_str()) {
            if existing.target_id != target_id {
                let _ = self.event_tx.send(ControlPlaneEvent::FleetAlert {
                    target_id: Some(target_id),
                    lease_id: None,
                    category: "terminal_session_target_mismatch".to_string(),
                    message: format!(
                        "session {} was previously registered for different target",
                        session_id
                    ),
                });
            }
        }

        if !sse_connected {
            let _ = self.event_tx.send(ControlPlaneEvent::FleetAlert {
                target_id: Some(target_id),
                lease_id: None,
                category: "terminal_ws_failure".to_string(),
                message: format!(
                    "terminal websocket failed and SSE was also disconnected: {error_message}"
                ),
            });
            return;
        }

        let marker = TerminalGapMarker {
            target_id,
            session_id: session_id.clone(),
            since_offset,
            message: "terminal_gap_detected".to_string(),
            created_at_unix_ms: now_unix_ms(),
        };

        self.terminal_sessions.insert(
            session_id.clone(),
            TerminalSessionState {
                target_id,
                stale: true,
                last_offset: since_offset,
            },
        );

        if let Err(err) = self.store.record_terminal_gap(&marker).await {
            warn!(
                target_id = %target_id,
                session_id = %session_id,
                error = %err,
                "failed to persist terminal gap marker"
            );
        }

        let _ = self.event_tx.send(ControlPlaneEvent::TerminalGap {
            marker: marker.clone(),
        });

        let Some(bridge) = self.terminal_stream_bridge.clone() else {
            let _ = self.event_tx.send(ControlPlaneEvent::FleetAlert {
                target_id: Some(target_id),
                lease_id: None,
                category: "terminal_reconnect_unavailable".to_string(),
                message: "terminal stream bridge unavailable for reconnect".to_string(),
            });
            return;
        };

        let mut backoff_ms = TERMINAL_RECONNECT_BASE_MS;
        let mut last_error = String::from("no reconnect attempt executed");

        for _attempt in 1..=TERMINAL_RECONNECT_MAX_ATTEMPTS {
            let jitter = rand::thread_rng().gen_range(0..=150);
            let wait_ms = (backoff_ms + jitter).min(TERMINAL_RECONNECT_CAP_MS);
            sleep(Duration::from_millis(wait_ms)).await;

            match bridge
                .reconnect_terminal_stream(target_id, session_id.as_str(), since_offset)
                .await
            {
                Ok(replay_offset) => {
                    if let Some(session) = self.terminal_sessions.get_mut(session_id.as_str()) {
                        session.stale = false;
                        session.last_offset = replay_offset.or(since_offset);
                    }

                    let reconnect_message = if replay_offset.is_some() {
                        "terminal stream reconnected with replay window".to_string()
                    } else {
                        "terminal stream reconnected without replay support; gap preserved"
                            .to_string()
                    };

                    let _ = self.event_tx.send(ControlPlaneEvent::FleetAlert {
                        target_id: Some(target_id),
                        lease_id: None,
                        category: "terminal_reconnected".to_string(),
                        message: reconnect_message,
                    });
                    return;
                }
                Err(err) => {
                    last_error = err.to_string();
                    backoff_ms = (backoff_ms * 2).min(TERMINAL_RECONNECT_CAP_MS);
                }
            }
        }

        let _ = self.event_tx.send(ControlPlaneEvent::FleetAlert {
            target_id: Some(target_id),
            lease_id: None,
            category: "terminal_reconnect_failed".to_string(),
            message: format!("terminal reconnect exhausted after max attempts: {last_error}"),
        });
    }

    async fn handle_promote_standby(&mut self) -> Result<()> {
        if self.ha.role == ControllerRole::Primary {
            self.store
                .heartbeat_controller(self.ha.controller_id, self.ha.controller_epoch)
                .await
                .context("heartbeat_controller failed")?;
            return Ok(());
        }

        let (promoted, next_epoch, next_fence_token) = self
            .store
            .promote_if_stale(
                self.ha.controller_id,
                self.ha.controller_epoch,
                self.ha.failover_timeout,
            )
            .await
            .context("promote_if_stale failed")?;

        if !promoted {
            return Ok(());
        }

        let stolen = self
            .store
            .takeover_active_leases(self.ha.controller_id, next_epoch, next_fence_token)
            .await
            .context("takeover_active_leases failed")?;

        self.ha.role = ControllerRole::Primary;
        self.ha.controller_epoch = next_epoch;
        self.ha.lease_fence_token = next_fence_token;

        for lease in self.leases.values_mut() {
            if lease.state == LeaseState::Active {
                lease.owner_controller_id = self.ha.controller_id;
                lease.owner_controller_epoch = self.ha.controller_epoch;
                lease.lease_fence_token = self.ha.lease_fence_token;
            }
        }

        let _ = self.event_tx.send(ControlPlaneEvent::FleetAlert {
            target_id: None,
            lease_id: None,
            category: "standby_promoted".to_string(),
            message: format!(
                "warm standby promoted to primary; epoch={} fence={} stolen_leases={}",
                self.ha.controller_epoch, self.ha.lease_fence_token, stolen
            ),
        });

        info!(
            controller_id = %self.ha.controller_id,
            epoch = self.ha.controller_epoch,
            fence = self.ha.lease_fence_token,
            stolen_leases = stolen,
            "warm standby promoted to primary"
        );

        Ok(())
    }

    async fn handle_reverse_tunnel_keepalive_tick(&mut self, target_id: Uuid) -> Result<()> {
        let target = self
            .targets
            .get(&target_id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown target for keepalive tick"))?;

        if target.mode != TargetMode::ReverseWs {
            return Ok(());
        }

        let Some(transport) = self.reverse_tunnel.clone() else {
            return Err(anyhow!("reverse tunnel transport not configured"));
        };

        let ping_result = timeout(
            self.config.reverse_ws_ping_timeout,
            transport.ping(target_id),
        )
        .await;

        match ping_result {
            Ok(Ok(())) => {
                self.reverse_tunnel_misses.insert(target_id, 0);
                if self.reverse_tunnel_dead.remove(&target_id) {
                    let _ = self.event_tx.send(ControlPlaneEvent::FleetAlert {
                        target_id: Some(target_id),
                        lease_id: None,
                        category: "reverse_tunnel_recovered".to_string(),
                        message: "reverse tunnel heartbeat recovered".to_string(),
                    });
                }
                return Ok(());
            }
            _ => {
                let misses = self.reverse_tunnel_misses.entry(target_id).or_insert(0);
                *misses = misses.saturating_add(1);

                warn!(
                    target_id = %target_id,
                    misses = *misses,
                    "reverse tunnel ping miss"
                );

                if *misses < self.config.reverse_ws_dead_after_misses {
                    return Ok(());
                }
            }
        }

        self.reverse_tunnel_dead.insert(target_id);
        self.quarantine_target(
            target_id,
            "reverse_tunnel_dead",
            "reverse tunnel considered dead; dispatch halted pending reconnect",
        )
        .await;

        let endpoint = match self.resolve_and_verify_identity(&target).await {
            Ok(endpoint) => endpoint,
            Err(failure) => {
                if failure.identity_mismatch {
                    if let Some(lease_id) = self.active_lease_for_target(target_id) {
                        self.quarantine_and_taint(
                            target_id,
                            lease_id,
                            "identity_pin_mismatch",
                            "identity_mismatch",
                            serde_json::json!({
                                "protocol": "12.6",
                                "stage": format!("{:?}", failure.stage),
                                "message": failure.message,
                            }),
                        )
                        .await;
                    }
                }
                return Err(anyhow!(
                    "reverse tunnel reconnect failed during identity verification: {}",
                    failure.message
                ));
            }
        };

        match timeout(
            Duration::from_secs(6),
            transport.reconnect(&target, endpoint),
        )
        .await
        {
            Ok(Ok(())) => {
                self.reverse_tunnel_dead.remove(&target_id);
                self.reverse_tunnel_misses.insert(target_id, 0);
                let mut should_emit = false;
                if let Some(record) = self.targets.get_mut(&target_id) {
                    if record.state == TargetState::Quarantine && !record.tainted {
                        record.state = TargetState::Ready;
                        record.taint_reason = None;
                        should_emit = true;
                    }
                }

                if should_emit {
                    self.emit_target_status(target_id);
                }
                let _ = self.event_tx.send(ControlPlaneEvent::FleetAlert {
                    target_id: Some(target_id),
                    lease_id: None,
                    category: "reverse_tunnel_reconnected".to_string(),
                    message: "reverse tunnel reconnected after DNS+identity verification"
                        .to_string(),
                });
                Ok(())
            }
            Ok(Err(err)) => {
                error!(target_id = %target_id, error = %err, "reverse tunnel reconnect failed");
                Err(anyhow!("reverse tunnel reconnect failed: {err}"))
            }
            Err(_) => Err(anyhow!("reverse tunnel reconnect timeout")),
        }
    }

    async fn quarantine_target(&mut self, target_id: Uuid, reason: &str, message: &str) {
        if let Some(target) = self.targets.get_mut(&target_id) {
            target.state = TargetState::Quarantine;
            target.taint_reason = Some(reason.to_string());
            self.emit_target_status(target_id);
        }

        let _ = self.event_tx.send(ControlPlaneEvent::FleetAlert {
            target_id: Some(target_id),
            lease_id: None,
            category: reason.to_string(),
            message: message.to_string(),
        });
    }

    async fn quarantine_and_taint(
        &mut self,
        target_id: Uuid,
        lease_id: Uuid,
        reason: &str,
        category: &str,
        details: serde_json::Value,
    ) {
        if let Some(lease) = self.leases.get_mut(&lease_id) {
            lease.state = LeaseState::Tainted;
        }

        if let Some(target) = self.targets.get_mut(&target_id) {
            target.state = TargetState::Quarantine;
            target.tainted = true;
            target.taint_reason = Some(reason.to_string());
            target.recent_failure_rate = (target.recent_failure_rate + 0.2).clamp(0.0, 1.0);
            self.emit_target_status(target_id);
        }

        let alert = SecurityAlert {
            alert_id: Uuid::new_v4(),
            target_id: Some(target_id),
            lease_id: Some(lease_id),
            severity: "high".to_string(),
            category: category.to_string(),
            details,
            created_at_unix_ms: now_unix_ms(),
        };

        if let Err(err) = self.store.record_security_alert(&alert).await {
            warn!(
                target_id = %target_id,
                lease_id = %lease_id,
                error = %err,
                "failed to persist security alert"
            );
        }

        let _ = self.event_tx.send(ControlPlaneEvent::FleetAlert {
            target_id: Some(target_id),
            lease_id: Some(lease_id),
            category: category.to_string(),
            message: reason.to_string(),
        });
    }

    fn active_lease_for_target(&self, target_id: Uuid) -> Option<Uuid> {
        self.leases
            .values()
            .find(|lease| lease.target_id == target_id && lease.state == LeaseState::Active)
            .map(|lease| lease.lease_id)
    }

    fn emit_target_status(&self, target_id: Uuid) {
        if let Some(target) = self.targets.get(&target_id) {
            let _ = self.event_tx.send(ControlPlaneEvent::TargetStatus {
                target_id,
                display_name: target.display_name.clone(),
                mode: target.mode,
                state: target.state,
                tainted: target.tainted,
                reason: target.taint_reason.clone(),
                health_score: target.health_score,
                latency_ewma_ms: target.latency_ewma_ms,
                recent_failure_rate: target.recent_failure_rate,
                docker_health: target.docker_health,
                docker_pass_count: target.docker_pass_count,
                docker_fail_count: target.docker_fail_count,
                docker_last_run_at_unix_ms: target.docker_last_run_at_unix_ms,
            });
        }
    }
}

pub fn docker_eval_suite_commands() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "docker_basic_execution_test",
            "cargo test -p kria-core --test environment_docker_tests docker_basic_execution_test -- --nocapture",
        ),
        (
            "docker_env_var_injection_test",
            "cargo test -p kria-core --test environment_docker_tests docker_env_var_injection_test -- --nocapture",
        ),
        (
            "docker_archive_io_tmpfs_test",
            "cargo test -p kria-core --test environment_docker_tests docker_archive_io_tmpfs_test -- --nocapture",
        ),
        (
            "docker_output_flood_control_test",
            "cargo test -p kria-core --test environment_docker_tests docker_output_flood_control_test -- --nocapture",
        ),
        (
            "docker_memory_limit_oom_test",
            "cargo test -p kria-core --test environment_docker_tests docker_memory_limit_oom_test -- --nocapture",
        ),
        (
            "docker_pid_limit_exhaustion_test",
            "cargo test -p kria-core --test environment_docker_tests docker_pid_limit_exhaustion_test -- --nocapture",
        ),
    ]
}

fn verify_rotation_attestation(
    expected_target_id: Uuid,
    mat: &KeyAttestationMaterial,
    proof: &KeyAttestationPayload,
) -> Result<()> {
    if proof.target_id != expected_target_id {
        return Err(anyhow!("attestation target mismatch"));
    }

    if proof.candidate_ssh_fingerprint.is_none() && proof.candidate_mtls_fingerprint.is_none() {
        return Err(anyhow!(
            "candidate key material missing in attestation payload"
        ));
    }

    if mat.next_ssh_fingerprint.is_some()
        && proof.candidate_ssh_fingerprint != mat.next_ssh_fingerprint
    {
        return Err(anyhow!(
            "candidate ssh fingerprint does not match staged next fingerprint"
        ));
    }

    if mat.next_mtls_fingerprint.is_some()
        && proof.candidate_mtls_fingerprint != mat.next_mtls_fingerprint
    {
        return Err(anyhow!(
            "candidate mtls fingerprint does not match staged next fingerprint"
        ));
    }

    if proof.old_key_signature_b64.is_empty() || proof.candidate_key_signature_b64.is_empty() {
        return Err(anyhow!("attestation signatures missing"));
    }

    let old_pub = mat
        .active_attestation_pubkey_b64
        .as_ref()
        .ok_or_else(|| anyhow!("active attestation pubkey not available"))?;
    let next_pub = mat
        .next_attestation_pubkey_b64
        .as_ref()
        .ok_or_else(|| anyhow!("candidate attestation pubkey not available"))?;

    let old_msg = canonical_attestation_message("old", proof)?;
    let next_msg = canonical_attestation_message("candidate", proof)?;

    if !verify_ed25519_b64(
        old_pub.as_str(),
        old_msg.as_slice(),
        proof.old_key_signature_b64.as_str(),
    )? {
        return Err(anyhow!("old-key attestation signature invalid"));
    }

    if !verify_ed25519_b64(
        next_pub.as_str(),
        next_msg.as_slice(),
        proof.candidate_key_signature_b64.as_str(),
    )? {
        return Err(anyhow!("candidate-key attestation signature invalid"));
    }

    Ok(())
}

fn canonical_attestation_message(role: &str, proof: &KeyAttestationPayload) -> Result<Vec<u8>> {
    serde_json::to_vec(&serde_json::json!({
        "domain": "kria.trust_rotation.attestation.v1",
        "role": role,
        "target_id": proof.target_id,
        "challenge_nonce": proof.challenge_nonce,
        "candidate_ssh_fingerprint": proof.candidate_ssh_fingerprint,
        "candidate_mtls_fingerprint": proof.candidate_mtls_fingerprint,
    }))
    .map_err(|err| anyhow!("failed to serialize canonical attestation message: {err}"))
}

fn verify_ed25519_b64(pubkey_b64: &str, message: &[u8], signature_b64: &str) -> Result<bool> {
    let pubkey_raw = B64
        .decode(pubkey_b64.as_bytes())
        .map_err(|err| anyhow!("invalid attestation public key encoding: {err}"))?;
    let sig_raw = B64
        .decode(signature_b64.as_bytes())
        .map_err(|err| anyhow!("invalid attestation signature encoding: {err}"))?;

    let pubkey_bytes: [u8; 32] = pubkey_raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("attestation public key must be 32 bytes"))?;
    let sig_bytes: [u8; 64] = sig_raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("attestation signature must be 64 bytes"))?;

    let verifying_key = VerifyingKey::from_bytes(&pubkey_bytes)
        .map_err(|err| anyhow!("invalid attestation public key: {err}"))?;
    let signature = Signature::from_bytes(&sig_bytes);

    Ok(verifying_key.verify(message, &signature).is_ok())
}

fn jitter_ms(base_ms: u64, pct: f64) -> u64 {
    let swing = ((base_ms as f64) * pct).round() as i64;
    let delta = rand::thread_rng().gen_range(-swing..=swing);
    (base_ms as i64 + delta).max(100) as u64
}
