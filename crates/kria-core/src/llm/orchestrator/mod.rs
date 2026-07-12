//! Hardware Orchestrator — manages llama-server lifecycle and dynamic GPU
//! layer offloading based on real-time VRAM/RAM telemetry.
//!
//! Cross-platform: NVML on Linux/Windows, RAM-based on macOS, disabled when
//! no GPU is present.

pub mod child_guard;
pub mod gpu_policy;
pub mod gpu_watchdog;
pub mod ra_adapter;
pub mod runtime;
pub mod server_manager;
pub mod strategy;
pub mod telemetry;
pub mod threshold;
pub mod tier_strategy;
pub mod vision_strategy;
pub mod vram_budget;

pub use crate::resource::L1Residency;
pub use runtime::L1Runtime;

use crate::config::OrchestratorConfig;
use crate::infra::environment::remote_qemu::QemuSshEnvironment;
use crate::infra::environment::{
    CommandExecutor, CommandRequest, CommandResult, EnvironmentError, EnvironmentLifecycle,
    FileSystemOps, ListDirRequest, ListDirResult, ReadFileRequest, ReadFileResult, ResetReason,
    ShellState, WriteFileRequest, WriteFileResult,
};
use crate::infra::event_bus::EventBus;
use crate::infra::health::HealthRegistry;
use crate::infra::pool::{LeaseHandle, PoolTelemetryPacket, TargetPool};
use crate::infra::qos::{QosAdaptationDecision, QosAdaptationPacket};
use crate::resource::{
    GpuLeaseManager, GpuOwner, ImageRuntimeSnapshot, L1RuntimeSnapshot, LeaseToken, RamSnapshot,
    RecoveryReason, ResourceSnapshot, VramSnapshot,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

fn is_router_mode_unavailable_error(err: &str) -> bool {
    err.contains("Router Mode not supported")
        || err.contains("Router Mode not active")
        || err.contains("cached HTTP 404/501")
}

// ── Safe-ngl persistence (startup backoff seed) ───────────────────────────────────────────────
//
// Remembers the highest `n-gpu-layers` that actually LOADED for a given model, so subsequent boots
// start straight at the known-good value instead of re-probing a too-high ngl that hangs (the
// Vulkan-laptop quirk). Best-effort: any IO/parse error is ignored and the ladder falls back to its
// full→down sweep. Keyed by model filename so different models don't cross-contaminate.

fn safe_ngl_cache_path() -> Option<std::path::PathBuf> {
    let paths = crate::platform::paths::KriaPaths::resolve();
    Some(paths.data_dir.join("llm_safe_ngl.json"))
}

fn safe_ngl_model_key(model_path: &str) -> String {
    std::path::Path::new(model_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(model_path)
        .to_string()
}

fn read_cached_safe_ngl(model_path: &str) -> Option<u32> {
    let path = safe_ngl_cache_path()?;
    let bytes = std::fs::read(&path).ok()?;
    let map: std::collections::HashMap<String, u32> = serde_json::from_slice(&bytes).ok()?;
    map.get(&safe_ngl_model_key(model_path)).copied()
}

fn write_cached_safe_ngl(model_path: &str, ngl: u32) {
    let Some(path) = safe_ngl_cache_path() else {
        return;
    };
    let mut map: std::collections::HashMap<String, u32> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let key = safe_ngl_model_key(model_path);
    if map.get(&key).copied() == Some(ngl) {
        return; // unchanged
    }
    map.insert(key, ngl);
    if let Ok(json) = serde_json::to_vec_pretty(&map) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, json);
    }
}

/// Build the descending `n-gpu-layers` backoff ladder used for the startup spawn probe.
///
/// `full` is the freshly VRAM-computed target ngl; `cached` is the last known-good ngl persisted
/// from a previous successful boot (if any).
///
/// Root-cause fix: a previous version seeded the ladder with `cached` FIRST and then only
/// generated the `full`+fraction rungs inside an `if !ladder.contains(&full)` guard. Whenever the
/// cache held a value equal to `full` (the common case right after a successful boot at full
/// offload), that guard was already false, so the fraction rungs (3/4, 1/2, 1/4 of `full`) were
/// never generated — the ladder silently collapsed to `[full, 0]`. If `full` then failed to load
/// on a later boot (e.g. the RTX 4050 quirk where ngl≥30 can hang even though ngl≤28 loads,
/// or transient VRAM contention from another process), the orchestrator had no mid-range rung
/// to fall back to and jumped straight to CPU-only — the exact 15+ minute stuck-on-CPU symptom
/// observed in GUI validation.
///
/// This version always builds the complete `full → 3/4 → 1/2 → 1/4 → 0` ladder first, then moves
/// the cached value to the front (as a fast-path hint) without removing any rung. A stale or
/// optimistic cache entry can no longer shrink the fallback ladder.
fn build_ngl_backoff_ladder(full: u32, cached: Option<u32>) -> Vec<u32> {
    let mut ladder: Vec<u32> = Vec::new();
    if full > 0 {
        ladder.push(full);
        for frac in [3u32, 2, 1] {
            let n = full * frac / 4;
            if n > 0 && !ladder.contains(&n) {
                ladder.push(n);
            }
        }
    }
    ladder.push(0); // CPU fallback — always fits.

    // Move the cached known-good value to the front as a fast-path hint, without dropping any
    // rung already in the ladder.
    if let Some(c) = cached.filter(|&n| n > 0 && (full == 0 || n <= full)) {
        ladder.retain(|&n| n != c);
        ladder.insert(0, c);
    }
    ladder
}

/// Which GPU backend is available on this platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuBackend {
    /// NVIDIA GPU (Linux/Windows) — full VRAM-based orchestration.
    Cuda,
    /// Apple Silicon (macOS) — unified memory, RAM-based telemetry, static ngl.
    Metal,
    /// No discrete GPU — CPU-only inference, orchestrator mostly static.
    CpuOnly,
}

impl GpuBackend {
    /// Detect the GPU backend for the current platform.
    pub fn detect() -> Self {
        #[cfg(target_os = "macos")]
        {
            return GpuBackend::Metal;
        }

        #[cfg(not(target_os = "macos"))]
        {
            // Check NVML availability first, then nvidia-smi CLI fallback
            if Self::has_nvidia_gpu() {
                GpuBackend::Cuda
            } else {
                GpuBackend::CpuOnly
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn has_nvidia_gpu() -> bool {
        // Try NVML feature first
        #[cfg(feature = "nvidia")]
        {
            if nvml_wrapper::Nvml::init().is_ok() {
                return true;
            }
        }
        // CLI fallback: check if nvidia-smi exists and works
        std::process::Command::new("nvidia-smi")
            .arg("--query-gpu=name")
            .arg("--format=csv,noheader")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Whether VRAM-based dynamic orchestration is supported.
    pub fn supports_vram_orchestration(&self) -> bool {
        matches!(self, GpuBackend::Cuda)
    }
}

/// Snapshot of the current orchestrator state exposed to other components.
#[derive(Debug, Clone)]
pub struct OrchestratorSnapshot {
    pub backend: GpuBackend,
    pub current_ngl: u32,
    pub current_context: u32,
    pub degradation: strategy::DegradationLevel,
    pub server_healthy: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct L1ResidencyMetrics {
    pub unload_latency_ms: u64,
    pub load_latency_ms: u64,
    pub slot_save_ok: bool,
    pub slot_restore_ok: bool,
}

/// Intent surface for remote tool-calls backed by an environment provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteToolCallIntent {
    ExecuteCommand {
        request: CommandRequest,
        shell_state: ShellState,
    },
    ReadFile {
        request: ReadFileRequest,
    },
    WriteFile {
        request: WriteFileRequest,
    },
    ListDir {
        request: ListDirRequest,
    },
}

/// Result payload for a remote tool-call intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteToolCallOutcome {
    Command(CommandResult),
    ReadFile(ReadFileResult),
    WriteFile(WriteFileResult),
    ListDir(ListDirResult),
}

/// Stages emitted while handling EnvironmentResetRequired recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteResetLifecycleStage {
    AgentPaused,
    ResetStarted,
    ResetHealthy,
    AgentResumed,
}

pub type RemoteResetLifecycleCallback = Arc<dyn Fn(RemoteResetLifecycleStage, &str) + Send + Sync>;

pub type RemoteInfraObservabilityCallback =
    Arc<dyn Fn(RemoteInfraObservabilityEvent) + Send + Sync>;

#[derive(Debug, Clone)]
pub enum RemoteInfraObservabilityEvent {
    PoolTelemetry(PoolTelemetryPacket),
    QosAdaptation(QosAdaptationPacket),
}

#[derive(Debug, Clone, Default)]
pub struct RemoteInfraObservabilityState {
    pub latest_pool_packet: Option<PoolTelemetryPacket>,
    pub latest_qos_adaptation: Option<QosAdaptationPacket>,
    pub target_health_degraded: bool,
    pub qos_pressure_active: bool,
}

impl RemoteInfraObservabilityState {
    fn apply_event(&mut self, event: &RemoteInfraObservabilityEvent) {
        match event {
            RemoteInfraObservabilityEvent::PoolTelemetry(packet) => {
                self.target_health_degraded =
                    packet.tainted_targets > 0 || packet.quarantined_targets > 0;
                self.latest_pool_packet = Some(packet.clone());
            }
            RemoteInfraObservabilityEvent::QosAdaptation(packet) => {
                match packet.decision {
                    QosAdaptationDecision::ThrottleLowMaintenance
                    | QosAdaptationDecision::RejectLowMaintenance => {
                        self.qos_pressure_active = true;
                    }
                    QosAdaptationDecision::ReleaseLowMaintenanceThrottle => {
                        self.qos_pressure_active = false;
                    }
                    QosAdaptationDecision::PromoteMediumReconnect => {}
                }
                self.latest_qos_adaptation = Some(packet.clone());
            }
        }
    }
}

pub type RemoteQemuToolBridge = RemoteEnvironmentToolBridge<QemuSshEnvironment>;

/// Adapter that maps remote intents onto CommandExecutor/FileSystemOps and
/// performs a single reset-and-retry cycle when EnvironmentResetRequired occurs.
#[derive(Clone)]
pub struct RemoteEnvironmentToolBridge<E>
where
    E: CommandExecutor + FileSystemOps + EnvironmentLifecycle + Send + Sync + 'static,
{
    environment: Arc<E>,
    target_pool: Option<Arc<TargetPool>>,
    on_reset_lifecycle: Arc<StdMutex<Option<RemoteResetLifecycleCallback>>>,
    on_observability: Arc<StdMutex<Option<RemoteInfraObservabilityCallback>>>,
    observability_state: Arc<StdMutex<RemoteInfraObservabilityState>>,
}

impl<E> RemoteEnvironmentToolBridge<E>
where
    E: CommandExecutor + FileSystemOps + EnvironmentLifecycle + Send + Sync + 'static,
{
    pub fn new(environment: Arc<E>) -> Self {
        Self {
            environment,
            target_pool: None,
            on_reset_lifecycle: Arc::new(StdMutex::new(None)),
            on_observability: Arc::new(StdMutex::new(None)),
            observability_state: Arc::new(StdMutex::new(RemoteInfraObservabilityState::default())),
        }
    }

    pub fn with_reset_lifecycle_callback(mut self, callback: RemoteResetLifecycleCallback) -> Self {
        self.set_reset_lifecycle_callback(Some(callback));
        self
    }

    pub fn with_target_pool(mut self, target_pool: Arc<TargetPool>) -> Self {
        self.target_pool = Some(Arc::clone(&target_pool));
        self.register_pool_observability_callback(&target_pool);
        self
    }

    pub fn with_observability_callback(
        mut self,
        callback: RemoteInfraObservabilityCallback,
    ) -> Self {
        self.set_observability_callback(Some(callback));
        self
    }

    pub fn set_reset_lifecycle_callback(&mut self, callback: Option<RemoteResetLifecycleCallback>) {
        *self
            .on_reset_lifecycle
            .lock()
            .expect("remote bridge reset lifecycle lock poisoned") = callback;
    }

    pub fn set_observability_callback(
        &mut self,
        callback: Option<RemoteInfraObservabilityCallback>,
    ) {
        *self
            .on_observability
            .lock()
            .expect("remote bridge observability callback lock poisoned") = callback;
    }

    pub fn observability_snapshot(&self) -> RemoteInfraObservabilityState {
        self.observability_state
            .lock()
            .expect("remote bridge observability state lock poisoned")
            .clone()
    }

    pub async fn dispatch_tool_call(
        &self,
        intent: RemoteToolCallIntent,
    ) -> Result<RemoteToolCallOutcome, EnvironmentError> {
        let retry_intent = intent.clone();
        let mut active_lease = self.acquire_verified_pool_lease().await?;

        let outcome = match self.dispatch_once(intent, active_lease.as_ref()).await {
            Ok(outcome) => Ok(outcome),
            Err(EnvironmentError::EnvironmentResetRequired { reason }) => {
                self.emit_reset_stage(RemoteResetLifecycleStage::AgentPaused, &reason);
                self.emit_reset_stage(RemoteResetLifecycleStage::ResetStarted, &reason);

                let reset_reason = Self::classify_reset_reason(&reason);
                self.handle_reset_recovery(reset_reason, &mut active_lease)
                    .await?;

                self.emit_reset_stage(RemoteResetLifecycleStage::ResetHealthy, &reason);
                let retry = self
                    .dispatch_once(retry_intent, active_lease.as_ref())
                    .await;
                self.emit_reset_stage(RemoteResetLifecycleStage::AgentResumed, &reason);
                retry
            }
            Err(error) => Err(error),
        };

        self.release_pool_lease(active_lease).await;
        outcome
    }

    async fn dispatch_once(
        &self,
        intent: RemoteToolCallIntent,
        active_lease: Option<&LeaseHandle>,
    ) -> Result<RemoteToolCallOutcome, EnvironmentError> {
        if let Some(pool) = &self.target_pool {
            let lease = active_lease.ok_or_else(|| EnvironmentError::EnvironmentResetRequired {
                reason: "no lease, no dispatch invariant violation".to_string(),
            })?;
            let environment = pool.environment_for_lease(lease).await?;
            let outcome = Self::dispatch_intent_to_environment(environment.as_ref(), intent).await;
            self.sync_pool_observability(pool);
            return outcome;
        }

        Self::dispatch_intent_to_environment(self.environment.as_ref(), intent).await
    }

    async fn dispatch_intent_to_environment<T>(
        environment: &T,
        intent: RemoteToolCallIntent,
    ) -> Result<RemoteToolCallOutcome, EnvironmentError>
    where
        T: CommandExecutor + FileSystemOps + Send + Sync + ?Sized,
    {
        match intent {
            RemoteToolCallIntent::ExecuteCommand {
                request,
                shell_state,
            } => environment
                .execute_command(request, shell_state)
                .await
                .map(RemoteToolCallOutcome::Command),
            RemoteToolCallIntent::ReadFile { request } => environment
                .read_file(request)
                .await
                .map(RemoteToolCallOutcome::ReadFile),
            RemoteToolCallIntent::WriteFile { request } => environment
                .write_file(request)
                .await
                .map(RemoteToolCallOutcome::WriteFile),
            RemoteToolCallIntent::ListDir { request } => environment
                .list_dir(request)
                .await
                .map(RemoteToolCallOutcome::ListDir),
        }
    }

    async fn acquire_verified_pool_lease(&self) -> Result<Option<LeaseHandle>, EnvironmentError> {
        let Some(pool) = &self.target_pool else {
            return Ok(None);
        };

        let lease = pool.acquire_lease().await?;
        let renewed = pool.heartbeat(&lease.lease_id).await?;
        self.sync_pool_observability(pool);
        Ok(Some(renewed))
    }

    async fn release_pool_lease(&self, active_lease: Option<LeaseHandle>) {
        let Some(pool) = &self.target_pool else {
            return;
        };

        if let Some(lease) = active_lease {
            if let Err(error) = pool.release_lease(&lease.lease_id).await {
                tracing::warn!(
                    error = %error,
                    lease_id = %lease.lease_id.0,
                    "remote bridge: failed to release lease after dispatch"
                );
            }
            self.sync_pool_observability(pool);
        }
    }

    async fn handle_reset_recovery(
        &self,
        reset_reason: ResetReason,
        active_lease: &mut Option<LeaseHandle>,
    ) -> Result<(), EnvironmentError> {
        if let Some(pool) = &self.target_pool {
            let lease =
                active_lease
                    .clone()
                    .ok_or_else(|| EnvironmentError::EnvironmentResetRequired {
                        reason: "recovery requested without an active pool lease".to_string(),
                    })?;
            let environment = pool.environment_for_lease(&lease).await?;

            Self::run_fail_closed_reset(environment.as_ref(), reset_reason).await?;
            environment.ensure_ready().await?;

            // RFC-005 recovery: renew lease after reset before replaying the tool call.
            let renewed = match pool.heartbeat(&lease.lease_id).await {
                Ok(renewed) => renewed,
                Err(_) => {
                    let lease = pool.acquire_lease().await?;
                    pool.heartbeat(&lease.lease_id).await?
                }
            };

            *active_lease = Some(renewed);
            self.sync_pool_observability(pool);
            return Ok(());
        }

        Self::run_fail_closed_reset(self.environment.as_ref(), reset_reason).await?;
        self.environment.ensure_ready().await
    }

    async fn run_fail_closed_reset<T>(
        environment: &T,
        reset_reason: ResetReason,
    ) -> Result<(), EnvironmentError>
    where
        T: EnvironmentLifecycle + ?Sized,
    {
        match environment.reset_environment(reset_reason).await {
            Ok(()) => Ok(()),
            Err(primary_error) => {
                tracing::warn!(
                    error = %primary_error,
                    "remote bridge: reset failed; enforcing fail-closed hard reprovision"
                );

                environment
                    .reset_environment(ResetReason::RuntimeFailure)
                    .await
                    .map_err(|fallback_error| EnvironmentError::EnvironmentResetFailed {
                        reason: "orchestrator_fail_closed_recovery".to_string(),
                        details: format!(
                            "primary_reset_error={primary_error}; hard_reprovision_error={fallback_error}"
                        ),
                    })
            }
        }
    }

    fn register_pool_observability_callback(&self, pool: &Arc<TargetPool>) {
        let state = Arc::clone(&self.observability_state);
        let callback = Arc::clone(&self.on_observability);

        pool.register_telemetry_callback(Arc::new(move |packet| {
            Self::publish_observability_event(
                &state,
                &callback,
                RemoteInfraObservabilityEvent::PoolTelemetry(packet),
            );
        }));
    }

    fn sync_pool_observability(&self, pool: &Arc<TargetPool>) {
        if let Some(packet) = pool.latest_telemetry_packet() {
            self.emit_observability_event(RemoteInfraObservabilityEvent::PoolTelemetry(packet));
        }

        if let Some(adaptation) = pool.qos_adaptation_snapshot(1).into_iter().next() {
            self.emit_observability_event(RemoteInfraObservabilityEvent::QosAdaptation(adaptation));
        }
    }

    fn emit_observability_event(&self, event: RemoteInfraObservabilityEvent) {
        Self::publish_observability_event(&self.observability_state, &self.on_observability, event);
    }

    fn publish_observability_event(
        state: &Arc<StdMutex<RemoteInfraObservabilityState>>,
        callback_store: &Arc<StdMutex<Option<RemoteInfraObservabilityCallback>>>,
        event: RemoteInfraObservabilityEvent,
    ) {
        {
            let mut guard = state
                .lock()
                .expect("remote bridge observability state lock poisoned");
            guard.apply_event(&event);
        }

        let callback = callback_store
            .lock()
            .expect("remote bridge observability callback lock poisoned")
            .clone();
        if let Some(callback) = callback {
            callback(event.clone());
        }

        match &event {
            RemoteInfraObservabilityEvent::PoolTelemetry(packet) => {
                let payload = serde_json::to_string(packet)
                    .unwrap_or_else(|error| format!("telemetry_error:{error}"));
                tracing::debug!(
                    target: "kria_orchestrator",
                    packet = %payload,
                    "orchestrator_pool_telemetry"
                );
            }
            RemoteInfraObservabilityEvent::QosAdaptation(packet) => {
                let payload = serde_json::to_string(packet)
                    .unwrap_or_else(|error| format!("telemetry_error:{error}"));
                tracing::info!(
                    target: "kria_orchestrator",
                    packet = %payload,
                    "orchestrator_qos_adaptation"
                );
            }
        }
    }

    fn emit_reset_stage(&self, stage: RemoteResetLifecycleStage, reason: &str) {
        let callback = self
            .on_reset_lifecycle
            .lock()
            .expect("remote bridge reset lifecycle lock poisoned")
            .clone();

        if let Some(callback) = callback {
            callback(stage, reason);
        }
    }

    fn classify_reset_reason(reason: &str) -> ResetReason {
        let lower = reason.to_ascii_lowercase();
        if lower.contains("resource") || lower.contains("fd") || lower.contains("disk") {
            ResetReason::ResourceExhaustion
        } else if lower.contains("policy") {
            ResetReason::Policy
        } else if lower.contains("manual") {
            ResetReason::Manual
        } else {
            ResetReason::RuntimeFailure
        }
    }
}

/// Top-level orchestrator that wires telemetry → watchdog → server_manager.
pub struct Orchestrator {
    pub config: OrchestratorConfig,
    pub backend: GpuBackend,
    pub server_manager: Arc<server_manager::LlamaServerManager>,
    gpu_lease: Arc<GpuLeaseManager>,
    l1_lease_token: StdMutex<Option<LeaseToken>>,
    /// HRA cutover: when enforcing, the LLM holds an HRA co-residency admission while GPU-resident
    /// (acquired/released in `reconcile_l1_lease`). `None` in shadow mode — the legacy private lease
    /// (`gpu_lease` + `l1_lease_token`) remains the executor. This makes HRA the owner of the LLM's
    /// GPU residency decision under enforce.
    l1_hra_admission: tokio::sync::Mutex<Option<crate::resource::authority::AdmissionGuard>>,
    telemetry: Arc<dyn telemetry::GpuTelemetry>,
    event_bus: Arc<EventBus>,
    health: Arc<HealthRegistry>,
    watchdog_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    lifecycle_lock: Mutex<()>,
    last_restart_at: Mutex<Option<Instant>>,
    /// Total VRAM detected at boot. Used by the watchdog for dynamic
    /// threshold scaling (Phase 1). Zero on CPU-only backends.
    total_vram_mb: u64,
    last_unload_latency_ms: AtomicU64,
    last_load_latency_ms: AtomicU64,
    last_slot_save_ok: AtomicBool,
    last_slot_restore_ok: AtomicBool,
    remote_tool_bridge: StdMutex<Option<Arc<RemoteQemuToolBridge>>>,
    remote_infra_observability: Arc<StdMutex<RemoteInfraObservabilityState>>,
    /// Keeps the TelemetryActor OS thread alive for the duration of the
    /// orchestrator's lifetime. Drop order: actor is dropped after watchdog.
    _telemetry_actor: Option<telemetry::TelemetryActor>,
}

impl Orchestrator {
    /// Create and start the orchestrator.
    ///
    /// - Detects GPU backend
    /// - Spawns llama-server with optimal initial parameters
    /// - Starts the GPU watchdog telemetry loop
    pub async fn start(
        config: OrchestratorConfig,
        model_path: String,
        mmproj_path: Option<String>,
        event_bus: Arc<EventBus>,
        health: Arc<HealthRegistry>,
    ) -> anyhow::Result<Arc<Self>> {
        health.register("llama-server");
        health.register("orchestrator");
        health.update(
            "orchestrator",
            crate::infra::health::ServiceStatus::Starting,
            Some("detecting GPU backend".into()),
        );
        health.update(
            "llama-server",
            crate::infra::health::ServiceStatus::Starting,
            Some("awaiting spawn parameters".into()),
        );

        // GpuBackend::detect() calls nvidia-smi (a subprocess) when NVML is
        // unavailable. Wrap in spawn_blocking so we never block a Tokio worker.
        let backend = tokio::task::spawn_blocking(GpuBackend::detect)
            .await
            .unwrap_or(GpuBackend::CpuOnly);
        tracing::info!(?backend, "orchestrator: detected GPU backend");
        health.update(
            "orchestrator",
            crate::infra::health::ServiceStatus::Starting,
            Some(format!("GPU backend detected: {:?}", backend)),
        );

        // Start the TelemetryActor: a dedicated OS thread that owns NVML/sysinfo (with an
        // nvidia-smi CLI fallback that works even without the `nvidia` feature compiled). The
        // orchestrator MUST use this rather than the TelemetryHub's `build_profiler`, because the
        // hub profiler is NVML-feature-gated and returns a Null/0-VRAM reading under
        // `--no-default-features` — which would make sizing read RAM-as-VRAM and over-allocate ngl.
        let poll_interval = Duration::from_secs(config.poll_interval_secs.max(1));
        let (telemetry_actor, telemetry) = tokio::task::spawn_blocking(move || {
            telemetry::create_telemetry_actor(backend, poll_interval)
        })
        .await
        .expect("telemetry actor thread spawn failed");
        health.update(
            "orchestrator",
            crate::infra::health::ServiceStatus::Starting,
            Some(format!("Telemetry online ({})", telemetry.source_name())),
        );

        // Calculate initial parameters from pre-spawn telemetry.
        let mut initial_snapshot = telemetry.snapshot().await;
        // COLD-START GUARD (root-cause fix for the first-prompt "LLM not reachable" flap):
        // the telemetry actor polls on an interval, so the VERY FIRST snapshot here can still be the
        // pre-sample default (free=0, total=0). Sizing on that wrongly lands the model on CPU
        // (ngl=0). The watchdog then sees real free VRAM and tries to scale the model onto the GPU
        // by restarting llama-server — over and over — which is exactly the between-/first-turn
        // "Optimizing GPU layers / LLM server not reachable" flapping. On a GPU backend, force a
        // FRESH synchronous VRAM read (shared hub → else one-shot CLI profiler) before sizing so the
        // model is sized onto the GPU once, at startup, and never needs a scale-up restart.
        if backend != GpuBackend::CpuOnly && initial_snapshot.total_vram_mb == 0 {
            let fresh = if let Some(hub) = crate::resource::global_telemetry_hub() {
                let s = hub.sample_now().await;
                s.gpus.first().map(|g| (g.free_vram_mb, g.total_vram_mb))
            } else {
                None
            };
            let (free, total) = match fresh {
                Some((f, t)) if t > 0 => (f, t),
                _ => {
                    let snap = crate::platform::vram::build_profiler().snapshot().await;
                    (snap.free_mb, snap.total_mb)
                }
            };
            if total > 0 {
                tracing::info!(
                    free_vram_mb = free,
                    total_vram_mb = total,
                    "orchestrator: cold-start fresh VRAM read (telemetry actor not warm yet) — sizing on GPU, not CPU"
                );
                initial_snapshot.free_vram_mb = free;
                initial_snapshot.total_vram_mb = total;
            } else {
                tracing::warn!(
                    "orchestrator: fresh VRAM read still 0 on a GPU backend — sizing may fall back to CPU"
                );
            }
        }
        let total_vram_mb = initial_snapshot.total_vram_mb;
        let initial_params = strategy::calculate_target_params_prod(
            &config.model_profile,
            initial_snapshot.free_vram_mb,
            config.safety_margin_mb,
            backend,
        );

        tracing::info!(
            ngl = initial_params.ngl,
            ctx = initial_params.context,
            degradation = ?initial_params.degradation,
            "orchestrator: initial parameters"
        );

        // Create and spawn llama-server
        let model_path_for_cache = model_path.clone();
        let server_manager = Arc::new(server_manager::LlamaServerManager::new(
            config.clone(),
            model_path,
            mmproj_path,
        ));

        health.update(
            "llama-server",
            crate::infra::health::ServiceStatus::Starting,
            Some(format!(
                "Spawning llama-server (ngl={}, ctx={})",
                initial_params.ngl, initial_params.context
            )),
        );

        // STARTUP SPAWN with ngl BACKOFF (root-cause fix for "LLM won't start").
        //
        // Some llama-server builds (e.g. the Vulkan laptop build that enumerates both an Intel iGPU
        // and the NVIDIA dGPU) HANG indefinitely during model load at a high `n-gpu-layers` even
        // though VRAM is plentiful — verified on an RTX 4050 where ngl≥30 stalls but ngl≤28 loads in
        // ~2 s. A single spawn at the computed full-offload ngl then trips the port-discovery
        // timeout and the orchestrator never comes up. Instead we try a descending ngl ladder with a
        // SHORT per-attempt timeout (a healthy load is ~2 s, a hang is forever, so a short probe is
        // safe), backing off until the server reports a listening port. The final rung is CPU
        // (ngl=0) which always loads — so the LLM is ALWAYS available, just slower in the worst case.
        let full = initial_params.ngl;
        let cached = read_cached_safe_ngl(&model_path_for_cache);
        let ladder = build_ngl_backoff_ladder(full, cached);

        // Short probe for GPU attempts so a hang costs ~20 s, not the full discovery timeout.
        const GPU_PROBE_SECS: u64 = 20;
        let mut loaded_ngl: Option<u32> = None;
        let mut last_err: Option<anyhow::Error> = None;
        for (idx, &cand_ngl) in ladder.iter().enumerate() {
            // GPU attempts get the short probe; the CPU fallback gets the full configured timeout
            // (clear override) because a cold CPU load can legitimately take longer.
            server_manager.set_spawn_timeout_override(if cand_ngl == 0 {
                0
            } else {
                GPU_PROBE_SECS
            });
            // On retries, cap context to a conservative 4096 to reduce load time/footprint.
            let cand_ctx = if idx == 0 {
                initial_params.context
            } else {
                initial_params.context.min(4096)
            };
            health.update(
                "llama-server",
                crate::infra::health::ServiceStatus::Starting,
                Some(format!(
                    "Spawning llama-server (attempt {}/{}, ngl={}, ctx={})",
                    idx + 1,
                    ladder.len(),
                    cand_ngl,
                    cand_ctx
                )),
            );
            tracing::info!(
                attempt = idx + 1,
                of = ladder.len(),
                ngl = cand_ngl,
                ctx = cand_ctx,
                "orchestrator: startup spawn attempt"
            );
            match server_manager
                .spawn(
                    cand_ngl,
                    cand_ctx,
                    initial_params.vision_mode,
                    event_bus.clone(),
                )
                .await
            {
                Ok(()) => {
                    loaded_ngl = Some(cand_ngl);
                    // Persist the working GPU ngl so the next boot starts here (skip the hang probe).
                    if cand_ngl > 0 {
                        write_cached_safe_ngl(&model_path_for_cache, cand_ngl);
                    }
                    if cand_ngl != full {
                        tracing::warn!(
                            requested_ngl = full,
                            loaded_ngl = cand_ngl,
                            "orchestrator: backed off to a lower ngl that loads (higher ngl hung/failed)"
                        );
                    }
                    break;
                }
                Err(e) => {
                    tracing::warn!(ngl = cand_ngl, error = %e, "orchestrator: spawn attempt failed — backing off");
                    last_err = Some(e);
                }
            }
        }
        server_manager.set_spawn_timeout_override(0); // restore config defaults for later swaps

        let loaded_ngl = match loaded_ngl {
            Some(n) => n,
            None => {
                let e = last_err
                    .unwrap_or_else(|| anyhow::anyhow!("llama-server failed to start at any ngl"));
                health.update(
                    "llama-server",
                    crate::infra::health::ServiceStatus::Degraded,
                    Some(format!("startup spawn failed at every ngl: {e}")),
                );
                health.update(
                    "orchestrator",
                    crate::infra::health::ServiceStatus::Degraded,
                    Some("startup aborted".into()),
                );
                return Err(e);
            }
        };

        health.update(
            "llama-server",
            crate::infra::health::ServiceStatus::Healthy,
            Some("Server ready (warming kernels)".into()),
        );
        health.update(
            "orchestrator",
            crate::infra::health::ServiceStatus::Starting,
            Some("Starting GPU watchdog".into()),
        );

        // Start the watchdog loop
        let watchdog = gpu_watchdog::GpuWatchdog::new(
            config.clone(),
            backend,
            telemetry.clone(),
            server_manager.clone(),
            event_bus.clone(),
            total_vram_mb,
        );

        let watchdog_handle = tokio::spawn(async move {
            watchdog.run().await;
        });
        health.update(
            "orchestrator",
            crate::infra::health::ServiceStatus::Healthy,
            Some("Watchdog active".into()),
        );

        {
            let warmup_manager = server_manager.clone();
            let warmup_health = health.clone();
            tokio::spawn(async move {
                if let Err(e) = warmup_manager.run_warmup_completion().await {
                    tracing::debug!(?e, "orchestrator: warmup prompt failed");
                } else {
                    warmup_health.update(
                        "llama-server",
                        crate::infra::health::ServiceStatus::Healthy,
                        Some("Warmup completed".into()),
                    );
                }
            });
        }

        let gpu_lease = Arc::new(GpuLeaseManager::default());

        let orchestrator = Arc::new(Self {
            config,
            backend,
            server_manager,
            gpu_lease,
            l1_lease_token: StdMutex::new(None),
            l1_hra_admission: tokio::sync::Mutex::new(None),
            telemetry,
            event_bus,
            health,
            watchdog_handle: Mutex::new(Some(watchdog_handle)),
            lifecycle_lock: Mutex::new(()),
            last_restart_at: Mutex::new(None),
            total_vram_mb,
            last_unload_latency_ms: AtomicU64::new(0),
            last_load_latency_ms: AtomicU64::new(0),
            last_slot_save_ok: AtomicBool::new(false),
            last_slot_restore_ok: AtomicBool::new(false),
            remote_tool_bridge: StdMutex::new(None),
            remote_infra_observability: Arc::new(StdMutex::new(
                RemoteInfraObservabilityState::default(),
            )),
            _telemetry_actor: Some(telemetry_actor),
        });

        if loaded_ngl > 0 {
            orchestrator.claim_l1_lease("startup_initial_spawn");
        }
        orchestrator.reconcile_l1_lease(loaded_ngl > 0).await;

        Ok(orchestrator)
    }

    /// Get a snapshot of the current orchestrator state.
    pub fn snapshot(&self) -> OrchestratorSnapshot {
        let (ngl, ctx) = self.server_manager.current_params();
        let degradation = strategy::degradation_level(ngl, ctx, &self.config.model_profile);
        OrchestratorSnapshot {
            backend: self.backend,
            current_ngl: ngl,
            current_context: ctx,
            degradation,
            server_healthy: self.server_manager.is_healthy(),
        }
    }

    pub fn set_remote_tool_bridge(&self, bridge: RemoteQemuToolBridge) {
        let mut slot = self
            .remote_tool_bridge
            .lock()
            .expect("remote tool bridge lock poisoned");
        *slot = Some(Arc::new(bridge));
    }

    pub fn clear_remote_tool_bridge(&self) {
        let mut slot = self
            .remote_tool_bridge
            .lock()
            .expect("remote tool bridge lock poisoned");
        *slot = None;
    }

    pub fn remote_infra_observability_snapshot(&self) -> RemoteInfraObservabilityState {
        self.remote_infra_observability
            .lock()
            .expect("remote infra observability lock poisoned")
            .clone()
    }

    pub async fn dispatch_remote_tool_call(
        &self,
        intent: RemoteToolCallIntent,
    ) -> Result<RemoteToolCallOutcome, EnvironmentError> {
        let bridge = self
            .remote_tool_bridge
            .lock()
            .expect("remote tool bridge lock poisoned")
            .clone()
            .ok_or_else(|| EnvironmentError::ProviderUnavailable {
                provider: "remote_tool_bridge".to_string(),
                details: "RemoteEnvironmentToolBridge not configured".to_string(),
            })?;

        let outcome = bridge.dispatch_tool_call(intent).await;
        self.update_remote_infra_observability(bridge.observability_snapshot());
        outcome
    }

    fn update_remote_infra_observability(&self, snapshot: RemoteInfraObservabilityState) {
        {
            let mut state = self
                .remote_infra_observability
                .lock()
                .expect("remote infra observability lock poisoned");
            *state = snapshot.clone();
        }

        let degraded = snapshot.target_health_degraded || snapshot.qos_pressure_active;
        let status = if degraded {
            crate::infra::health::ServiceStatus::Degraded
        } else {
            crate::infra::health::ServiceStatus::Healthy
        };

        self.health.update(
            "orchestrator",
            status,
            Some(Self::format_remote_infra_observability_message(&snapshot)),
        );
    }

    fn format_remote_infra_observability_message(
        snapshot: &RemoteInfraObservabilityState,
    ) -> String {
        let pool = snapshot.latest_pool_packet.as_ref().map_or_else(
            || "pool=unavailable".to_string(),
            |packet| {
                format!(
                    "pool[event={}, ready={}, leased={}, tainted={}, quarantined={}]",
                    packet.event,
                    packet.ready_targets,
                    packet.leased_targets,
                    packet.tainted_targets,
                    packet.quarantined_targets,
                )
            },
        );

        let qos = snapshot.latest_qos_adaptation.as_ref().map_or_else(
            || "qos=unavailable".to_string(),
            |packet| {
                format!(
                    "qos[decision={:?}, high_wait_p95_ms={}, slo_ms={}]",
                    packet.decision, packet.high_recovery_wait_p95_ms, packet.high_recovery_slo_ms
                )
            },
        );

        format!(
            "{}; {}; target_health_degraded={}; qos_pressure_active={}",
            pool, qos, snapshot.target_health_degraded, snapshot.qos_pressure_active,
        )
    }

    pub fn l1_residency(&self) -> L1Residency {
        let state = self.server_manager.state();
        let (ngl, _) = self.server_manager.current_params();

        match state {
            server_manager::STATE_STOPPED => L1Residency::Stopped,
            server_manager::STATE_STARTING => L1Residency::Starting,
            server_manager::STATE_SWAPPING => {
                if ngl == 0 {
                    L1Residency::RamHotVramCold
                } else {
                    L1Residency::ReloadingGpu
                }
            }
            server_manager::STATE_READY => {
                if ngl > 0 {
                    L1Residency::GpuHot
                } else if self.backend == GpuBackend::Cuda {
                    L1Residency::RamHotVramCold
                } else {
                    L1Residency::CpuResidentLegacy
                }
            }
            server_manager::STATE_ERROR => L1Residency::Error,
            _ => L1Residency::Error,
        }
    }

    pub fn residency_metrics(&self) -> L1ResidencyMetrics {
        L1ResidencyMetrics {
            unload_latency_ms: self.last_unload_latency_ms.load(Ordering::Acquire),
            load_latency_ms: self.last_load_latency_ms.load(Ordering::Acquire),
            slot_save_ok: self.last_slot_save_ok.load(Ordering::Acquire),
            slot_restore_ok: self.last_slot_restore_ok.load(Ordering::Acquire),
        }
    }

    fn record_slot_save(&self, ok: bool) {
        self.last_slot_save_ok.store(ok, Ordering::Release);
    }

    fn record_slot_restore(&self, ok: bool) {
        self.last_slot_restore_ok.store(ok, Ordering::Release);
    }

    fn claim_l1_lease(&self, turn_label: &str) {
        let mut lock = self
            .l1_lease_token
            .lock()
            .expect("l1 lease token lock poisoned");

        if let Some(token) = lock.as_ref() {
            if self
                .gpu_lease
                .refresh(token, Some(Duration::from_secs(300)))
            {
                return;
            }
        }

        match self.gpu_lease.acquire_token(
            GpuOwner::L1Worker,
            turn_label,
            Some(Duration::from_secs(300)),
        ) {
            Ok(token) => {
                *lock = Some(token);
            }
            Err(e) => {
                tracing::warn!(error = %e, "orchestrator: failed to claim L1 GPU lease");
            }
        }
    }

    fn release_l1_lease(&self, reason: RecoveryReason) {
        let mut lock = self
            .l1_lease_token
            .lock()
            .expect("l1 lease token lock poisoned");
        if let Some(token) = lock.take() {
            let _ = self.gpu_lease.release_token(&token, reason);
        }
    }

    async fn build_resource_snapshot(&self, l1_gpu_resident_hint: bool) -> ResourceSnapshot {
        let telemetry = self.telemetry.snapshot().await;
        let live_process = self.server_manager.has_live_process().await;
        let l1_gpu_resident = live_process && l1_gpu_resident_hint;

        let mut sys = sysinfo::System::new();
        sys.refresh_memory();

        ResourceSnapshot {
            vram: VramSnapshot {
                free_mb: telemetry.free_vram_mb,
                total_mb: telemetry.total_vram_mb,
                used_mb: telemetry
                    .total_vram_mb
                    .saturating_sub(telemetry.free_vram_mb),
            },
            ram: RamSnapshot {
                total_mb: sys.total_memory() / (1024 * 1024),
                free_mb: sys.available_memory() / (1024 * 1024),
            },
            l1: L1RuntimeSnapshot {
                residency: self.l1_residency(),
                process_id: None,
            },
            image: ImageRuntimeSnapshot {
                backend_id: "comfy_ui".to_string(),
                is_generating: l1_gpu_resident,
                process_id: None,
            },
            processes: Vec::new(),
            sampled_at: Instant::now(),
        }
    }

    async fn reconcile_l1_lease(&self, l1_gpu_resident: bool) {
        let snapshot = self.build_resource_snapshot(l1_gpu_resident).await;
        self.gpu_lease.reconcile(&snapshot);

        // HRA cutover (enforce-only): tie the LLM's HRA co-residency admission to its GPU residency.
        // When the LLM becomes GPU-resident we acquire an InteractiveFg admission (so it co-resides
        // with / preempts background image/vision under the VRAM budget); when it leaves the GPU we
        // release it. In shadow mode this is a no-op and the legacy private lease is unchanged.
        if let Some(hra) = crate::resource::authority::global_hra() {
            if !hra.is_shadow_only() {
                let mut held = self.l1_hra_admission.lock().await;
                if l1_gpu_resident {
                    if held.is_none() {
                        let (_, total) = self.server_manager.current_params();
                        let vram_hint = self.config.model_profile.per_layer_vram_mb as u64
                            * self.server_manager.current_params().0.max(1) as u64
                            + self.config.model_profile.base_vram_overhead_mb as u64;
                        let _ = total;
                        let req = crate::resource::authority::ResourceRequest {
                            consumer: crate::resource::authority::ConsumerId::Llm,
                            class: crate::resource::authority::PriorityClass::InteractiveFg,
                            need: crate::resource::authority::ResourceNeed {
                                vram_mb: vram_hint.max(512),
                                ram_mb: 0,
                                cpu_threads: 0,
                                exclusivity: false,
                                model_id: None,
                                est_ms: 0,
                            },
                            constraints: Default::default(),
                            turn_id: crate::resource::authority::TurnId("l1_residency".into()),
                        };
                        match hra
                            .admit_gpu(&req, crate::resource::authority::ResidencyTarget::Hot)
                            .await
                        {
                            Ok(g) => {
                                tracing::info!(target: "hra", "[HRA][LLM] GPU Residency admitted (enforce)");
                                *held = Some(g);
                            }
                            Err(e) => {
                                tracing::warn!(target: "hra", reason = ?e, "[HRA][LLM] residency admission denied");
                            }
                        }
                    }
                } else if held.take().is_some() {
                    tracing::info!(target: "hra", "[HRA][LLM] GPU Residency released (left VRAM)");
                }
            }
        }
    }

    /// Get the current API URL of the running llama-server.
    pub fn api_url(&self) -> String {
        self.server_manager.api_url()
    }

    /// Graceful shutdown: stop watchdog, then kill server.
    pub async fn shutdown(&self) {
        let _lifecycle_guard = self.lifecycle_lock.lock().await;
        tracing::info!("orchestrator: shutting down");
        self.stop_watchdog().await;
        self.release_l1_lease(RecoveryReason::ShutdownRequested);

        self.server_manager
            .graceful_stop_with_timeout(Duration::from_secs(
                self.config.graceful_stop_timeout_secs.max(1),
            ))
            .await;

        if self.backend == GpuBackend::Cuda {
            self.wait_for_vram_release_bounded(Duration::from_secs(
                self.config.vram_release_timeout_secs.max(1),
            ))
            .await;
        }

        self.reconcile_l1_lease(false).await;

        self.health.update(
            "llama-server",
            crate::infra::health::ServiceStatus::Stopped,
            Some("orchestrator shutdown completed".into()),
        );
        self.health.update(
            "orchestrator",
            crate::infra::health::ServiceStatus::Stopped,
            Some("orchestrator stopped".into()),
        );
    }

    /// Restart the managed llama-server and re-arm watchdog monitoring.
    pub async fn restart(&self, reason: &str) -> anyhow::Result<()> {
        let _lifecycle_guard = self.lifecycle_lock.lock().await;

        let cooldown = Duration::from_secs(self.config.restart_cooldown_secs.max(1));
        {
            let mut last = self.last_restart_at.lock().await;
            if let Some(previous) = *last {
                if previous.elapsed() < cooldown {
                    let remaining_ms = (cooldown - previous.elapsed()).as_millis() as u64;
                    anyhow::bail!(
                        "orchestrator restart cooldown active (remaining {} ms)",
                        remaining_ms
                    );
                }
            }
            *last = Some(Instant::now());
        }

        tracing::warn!(reason, "orchestrator: restart requested");

        self.stop_watchdog().await;

        self.health.update(
            "orchestrator",
            crate::infra::health::ServiceStatus::Starting,
            Some(format!("restarting ({reason})")),
        );
        self.health.update(
            "llama-server",
            crate::infra::health::ServiceStatus::Starting,
            Some("restarting local LLM runtime".into()),
        );

        let (previous_ngl, previous_ctx) = self.server_manager.current_params();

        self.server_manager
            .graceful_stop_with_timeout(Duration::from_secs(
                self.config.graceful_stop_timeout_secs.max(1),
            ))
            .await;

        if self.backend == GpuBackend::Cuda {
            self.wait_for_vram_release_bounded(Duration::from_secs(
                self.config.vram_release_timeout_secs.max(1),
            ))
            .await;
        }

        let snapshot = self.telemetry.snapshot().await;
        let target = strategy::calculate_target_params_prod(
            &self.config.model_profile,
            snapshot.free_vram_mb,
            self.config.safety_margin_mb,
            self.backend,
        );

        let primary = self
            .server_manager
            .spawn(
                target.ngl,
                target.context,
                target.vision_mode,
                self.event_bus.clone(),
            )
            .await;

        let restart_result = match primary {
            Ok(()) => Ok(()),
            Err(primary_error) => {
                tracing::warn!(
                    ?primary_error,
                    "orchestrator: primary restart spawn failed; attempting fallback"
                );

                if self.config.restart_backoff_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(self.config.restart_backoff_ms)).await;
                }

                let fallback_ngl = previous_ngl;
                let fallback_ctx = if previous_ctx > 0 {
                    previous_ctx
                } else {
                    self.config.model_profile.min_context
                };
                let fallback_ram = {
                    let mut sys = sysinfo::System::new();
                    sys.refresh_memory();
                    sys.available_memory() / (1024 * 1024)
                };
                let fallback_vm = vision_strategy::determine_vision_mode(
                    &self.config.model_profile,
                    fallback_ngl,
                    fallback_ram,
                );

                self.server_manager
                    .spawn(
                        fallback_ngl,
                        fallback_ctx,
                        fallback_vm,
                        self.event_bus.clone(),
                    )
                    .await
                    .map_err(|fallback_error| {
                        anyhow::anyhow!(
                            "restart failed: primary error: {}; fallback error: {}",
                            primary_error,
                            fallback_error
                        )
                    })
            }
        };

        match restart_result {
            Ok(()) => {
                let (ngl, _) = self.server_manager.current_params();
                if ngl > 0 {
                    self.claim_l1_lease("restart");
                } else {
                    self.release_l1_lease(RecoveryReason::OwnerReleaseRequested);
                }
                self.reconcile_l1_lease(ngl > 0).await;

                self.health.update(
                    "llama-server",
                    crate::infra::health::ServiceStatus::Healthy,
                    None,
                );
                self.health.update(
                    "orchestrator",
                    crate::infra::health::ServiceStatus::Healthy,
                    None,
                );
                self.ensure_watchdog_running().await;
                tracing::info!("orchestrator: restart completed");
                Ok(())
            }
            Err(e) => {
                self.health.update(
                    "llama-server",
                    crate::infra::health::ServiceStatus::Degraded,
                    Some(format!("restart failed: {e}")),
                );
                self.health.update(
                    "orchestrator",
                    crate::infra::health::ServiceStatus::Degraded,
                    Some(format!("restart failed: {e}")),
                );
                Err(e)
            }
        }
    }

    /// Ensure llama-server is running and healthy.
    ///
    /// This is used by desktop preflight checks before dispatching a turn,
    /// especially when idle-release has intentionally stopped the runtime.
    pub async fn ensure_ready(&self, reason: &str) -> anyhow::Result<()> {
        let _lifecycle_guard = self.lifecycle_lock.lock().await;

        let has_live_process = self.server_manager.has_live_process().await;
        let (current_ngl, current_ctx) = self.server_manager.current_params();

        if has_live_process && self.server_manager.is_healthy() && current_ngl > 0 {
            self.claim_l1_lease("ensure_ready_fast_path");
            self.reconcile_l1_lease(true).await;
            self.ensure_watchdog_running().await;
            return Ok(());
        }

        let suspended = has_live_process && current_ngl == 0;

        if suspended {
            tracing::info!(
                reason,
                state = self.server_manager.state(),
                "orchestrator: restoring idle-suspended runtime via API reload"
            );
            self.health.update(
                "llama-server",
                crate::infra::health::ServiceStatus::Starting,
                Some("restoring idle-suspended model".into()),
            );
            let load_started = Instant::now();
            match self.server_manager.api_load_model().await {
                Ok(()) => {
                    self.last_load_latency_ms
                        .store(load_started.elapsed().as_millis() as u64, Ordering::Release);
                    let slot_restore_ok = self.server_manager.restore_active_slot().await;
                    self.record_slot_restore(slot_restore_ok);
                    tracing::debug!(
                        slot_restore_ok,
                        load_latency_ms = self.last_load_latency_ms.load(Ordering::Acquire),
                        "orchestrator: idle resume slot restore status"
                    );
                    self.claim_l1_lease("ensure_ready_idle_resume");
                    self.reconcile_l1_lease(true).await;
                    self.health.update(
                        "llama-server",
                        crate::infra::health::ServiceStatus::Healthy,
                        Some("idle resume complete".into()),
                    );
                    self.health.update(
                        "orchestrator",
                        crate::infra::health::ServiceStatus::Healthy,
                        None,
                    );
                    self.ensure_watchdog_running().await;
                    return Ok(());
                }
                Err(e) => {
                    let e_text = e.to_string();
                    if is_router_mode_unavailable_error(&e_text) {
                        tracing::info!(
                            error = %e,
                            "orchestrator: API idle resume unavailable on this llama-server; using legacy restart path"
                        );
                    } else {
                        tracing::warn!(
                            error = %e,
                            "orchestrator: API idle resume failed; falling back to restart"
                        );
                    }
                }
            }
        }

        tracing::info!(
            reason,
            had_live_process = has_live_process,
            state = self.server_manager.state(),
            "orchestrator: ensure_ready starting local runtime"
        );

        // Mark the restart as an intended, bounded transition so reachability probes
        // report "warming" instead of flipping the banner to "LLM server not reachable"
        // while the SAME model is being brought back (crash/idle recovery).
        self.server_manager.begin_restart();

        self.health.update(
            "orchestrator",
            crate::infra::health::ServiceStatus::Starting,
            Some(format!("ensuring local runtime ({reason})")),
        );
        self.health.update(
            "llama-server",
            crate::infra::health::ServiceStatus::Starting,
            Some("starting local LLM runtime".into()),
        );

        let previous_ngl = current_ngl;
        let previous_ctx = current_ctx;
        if has_live_process || self.server_manager.state() != server_manager::STATE_STOPPED {
            self.server_manager
                .graceful_stop_with_timeout(Duration::from_secs(
                    self.config.graceful_stop_timeout_secs.max(1),
                ))
                .await;

            if self.backend == GpuBackend::Cuda {
                self.wait_for_vram_release_bounded(Duration::from_secs(
                    self.config.vram_release_timeout_secs.max(1),
                ))
                .await;
            }
        }

        let snapshot = self.telemetry.snapshot().await;
        let target = strategy::calculate_target_params_prod(
            &self.config.model_profile,
            snapshot.free_vram_mb,
            self.config.safety_margin_mb,
            self.backend,
        );

        let primary = self
            .server_manager
            .spawn(
                target.ngl,
                target.context,
                target.vision_mode,
                self.event_bus.clone(),
            )
            .await;

        let ensure_result = match primary {
            Ok(()) => Ok(()),
            Err(primary_error) => {
                tracing::warn!(
                    ?primary_error,
                    "orchestrator: ensure_ready primary spawn failed; attempting fallback"
                );

                if self.config.restart_backoff_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(self.config.restart_backoff_ms)).await;
                }

                let fallback_ngl = previous_ngl;
                let fallback_ctx = if previous_ctx > 0 {
                    previous_ctx
                } else {
                    self.config.model_profile.min_context
                };
                let fallback_ram = {
                    let mut sys = sysinfo::System::new();
                    sys.refresh_memory();
                    sys.available_memory() / (1024 * 1024)
                };
                let fallback_vm = vision_strategy::determine_vision_mode(
                    &self.config.model_profile,
                    fallback_ngl,
                    fallback_ram,
                );

                self.server_manager
                    .spawn(
                        fallback_ngl,
                        fallback_ctx,
                        fallback_vm,
                        self.event_bus.clone(),
                    )
                    .await
                    .map_err(|fallback_error| {
                        anyhow::anyhow!(
                            "ensure_ready failed: primary error: {}; fallback error: {}",
                            primary_error,
                            fallback_error
                        )
                    })
            }
        };

        // Restart window over (success or failure) — resume normal reachability
        // reporting so a genuinely down server is surfaced accurately.
        self.server_manager.end_restart();

        match ensure_result {
            Ok(()) => {
                let (ngl, _) = self.server_manager.current_params();
                if ngl > 0 {
                    self.claim_l1_lease("ensure_ready");
                } else {
                    self.release_l1_lease(RecoveryReason::OwnerReleaseRequested);
                }
                self.reconcile_l1_lease(ngl > 0).await;

                self.health.update(
                    "llama-server",
                    crate::infra::health::ServiceStatus::Healthy,
                    None,
                );
                self.health.update(
                    "orchestrator",
                    crate::infra::health::ServiceStatus::Healthy,
                    None,
                );
                self.ensure_watchdog_running().await;
                Ok(())
            }
            Err(e) => {
                self.health.update(
                    "llama-server",
                    crate::infra::health::ServiceStatus::Degraded,
                    Some(format!("ensure_ready failed: {e}")),
                );
                self.health.update(
                    "orchestrator",
                    crate::infra::health::ServiceStatus::Degraded,
                    Some(format!("ensure_ready failed: {e}")),
                );
                Err(e)
            }
        }
    }

    /// Release llama-server when the desktop runtime is idle.
    ///
    /// Returns true if a running process was released, false when there was
    /// nothing to release.
    pub async fn release_if_idle(&self, reason: &str) -> anyhow::Result<bool> {
        let _lifecycle_guard = self.lifecycle_lock.lock().await;

        if !self.server_manager.has_live_process().await {
            self.release_l1_lease(RecoveryReason::OwnerReleaseRequested);
            self.reconcile_l1_lease(false).await;
            return Ok(false);
        }

        // If this llama-server build cannot unload the model without a full process
        // restart, idle-release is net-negative: freeing VRAM now GUARANTEES a slow
        // cold restart — and a user-visible "LLM server not reachable" window — on
        // the very next turn. This is the exact production symptom. Keep the model
        // resident and let the GPU watchdog handle genuine memory pressure instead.
        if matches!(
            self.server_manager.zero_downtime_swap_supported(),
            Some(false)
        ) {
            tracing::debug!(
                reason,
                "orchestrator: skipping idle release — llama-server lacks zero-downtime \
                 model unload; keeping model resident to avoid a cold-restart on the next turn"
            );
            return Ok(false);
        }

        tracing::info!(reason, "orchestrator: idle release requested");
        self.stop_watchdog().await;

        let slot_save_ok = self.server_manager.save_active_slot().await;
        self.record_slot_save(slot_save_ok);

        let unload_started = Instant::now();

        match self.server_manager.api_unload_model().await {
            Ok(()) => {
                self.last_unload_latency_ms.store(
                    unload_started.elapsed().as_millis() as u64,
                    Ordering::Release,
                );
                self.release_l1_lease(RecoveryReason::OwnerReleaseRequested);
                self.reconcile_l1_lease(false).await;
                tracing::debug!(
                    slot_save_ok,
                    unload_latency_ms = self.last_unload_latency_ms.load(Ordering::Acquire),
                    "orchestrator: idle release metrics"
                );
                self.health.update(
                    "llama-server",
                    crate::infra::health::ServiceStatus::Degraded,
                    Some("idle suspended (model unloaded)".into()),
                );
                self.health.update(
                    "orchestrator",
                    crate::infra::health::ServiceStatus::Healthy,
                    Some("idle release active (process alive)".into()),
                );
                return Ok(true);
            }
            Err(e) => {
                // Router-mode unload unsupported (the common case on stock llama.cpp
                // builds): a process kill here forces a slow cold restart on the next
                // turn. Keep the model resident instead — the very reason we probe the
                // capability. The watchdog still relieves genuine VRAM pressure.
                if is_router_mode_unavailable_error(&e.to_string()) {
                    tracing::debug!(
                        ?e,
                        reason,
                        "orchestrator: idle release skipped — zero-downtime unload unavailable; \
                         keeping model resident to avoid a cold-restart on the next turn"
                    );
                    self.ensure_watchdog_running().await;
                    return Ok(false);
                }
                tracing::warn!(
                    ?e,
                    "orchestrator: idle API unload failed, falling back to process stop"
                );
            }
        }

        self.server_manager
            .graceful_stop_with_timeout(Duration::from_secs(
                self.config.graceful_stop_timeout_secs.max(1),
            ))
            .await;
        self.release_l1_lease(RecoveryReason::OwnerReleaseRequested);

        if self.backend == GpuBackend::Cuda {
            self.wait_for_vram_release_bounded(Duration::from_secs(
                self.config.vram_release_timeout_secs.max(1),
            ))
            .await;
        }
        self.reconcile_l1_lease(false).await;

        self.health.update(
            "llama-server",
            crate::infra::health::ServiceStatus::Stopped,
            Some("released while idle; will warm on next turn".into()),
        );
        self.health.update(
            "orchestrator",
            crate::infra::health::ServiceStatus::Healthy,
            Some("idle release active".into()),
        );

        Ok(true)
    }

    async fn stop_watchdog(&self) {
        if let Some(handle) = self.watchdog_handle.lock().await.take() {
            handle.abort();
        }
    }

    async fn ensure_watchdog_running(&self) {
        let mut lock = self.watchdog_handle.lock().await;
        let should_restart = lock.as_ref().map(|h| h.is_finished()).unwrap_or(true);
        if !should_restart {
            return;
        }

        let watchdog = gpu_watchdog::GpuWatchdog::new(
            self.config.clone(),
            self.backend,
            self.telemetry.clone(),
            self.server_manager.clone(),
            self.event_bus.clone(),
            self.total_vram_mb,
        );

        *lock = Some(tokio::spawn(async move {
            watchdog.run().await;
        }));
    }

    async fn wait_for_vram_release_bounded(&self, timeout: Duration) {
        let start = Instant::now();

        loop {
            if start.elapsed() > timeout {
                tracing::warn!(
                    timeout_secs = timeout.as_secs(),
                    "orchestrator: VRAM release wait timed out"
                );
                break;
            }

            let snap = self.telemetry.snapshot().await;
            if snap.free_vram_mb > self.config.yield_threshold_mb {
                tracing::debug!(
                    free_mb = snap.free_vram_mb,
                    "orchestrator: VRAM release observed"
                );
                break;
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    // ─── Tier B drop-and-swap support ────────────────────────────────────────
    //
    // The image-generation path needs to free ~4 GB of VRAM occupied by the
    // local LLM so ComfyUI can fit. Older revisions tried to do this via
    // `POST /props {"n_gpu_layers": 0}`; modern llama.cpp builds reject that
    // request with HTTP 501 (the layer split is fixed at process start).
    //
    // The supported approach is a hard process restart with the appropriate
    // `--n-gpu-layers` flag, with conversational context preserved best-effort
    // through the slot KV-cache save/restore endpoints.
    //
    // Slot 0 is used as the canonical "current conversation" slot. When the
    // server is started with `--parallel 1` (the orchestrator's default for
    // single-user assistants) this is the only slot that exists.

    /// Evict the LLM model from VRAM to free GPU for ComfyUI.
    ///
    /// **Preferred path (zero-downtime):** API-level unload via Router Mode.
    /// Server stays alive, port stays stable, no process restart needed.
    ///
    /// **Fallback path (legacy):** SIGTERM/SIGKILL + respawn with ngl=0.
    /// Used when Router Mode is unavailable (HTTP 404/501).
    pub async fn evict_to_ram(&self) -> anyhow::Result<()> {
        let _lifecycle_guard = self.lifecycle_lock.lock().await;

        let (prior_ngl, prior_ctx) = self.server_manager.current_params();
        if prior_ngl == 0 {
            tracing::info!("orchestrator: evict_to_cpu no-op (already CPU-resident)");
            self.release_l1_lease(RecoveryReason::OwnerReleaseRequested);
            self.reconcile_l1_lease(false).await;
            return Ok(());
        }

        tracing::info!(
            prior_ngl,
            prior_ctx,
            "orchestrator: Tier B eviction starting"
        );

        // Stop the watchdog *first* so it can't re-spawn behind our back when
        // it sees VRAM pressure during the swap.
        self.stop_watchdog().await;

        // Best-effort: snapshot conversational context.
        let slot_save_ok = self.server_manager.save_active_slot().await;
        self.record_slot_save(slot_save_ok);

        self.health.update(
            "llama-server",
            crate::infra::health::ServiceStatus::Starting,
            Some("Tier B eviction: freeing VRAM".into()),
        );

        // ── Try API-level unload first (zero-downtime path) ──────────────
        let unload_started = Instant::now();
        match self.server_manager.api_unload_model().await {
            Ok(()) => {
                self.last_unload_latency_ms.store(
                    unload_started.elapsed().as_millis() as u64,
                    Ordering::Release,
                );
                self.release_l1_lease(RecoveryReason::OwnerReleaseRequested);
                self.reconcile_l1_lease(false).await;
                tracing::info!(
                    prior_ngl,
                    "orchestrator: Tier B eviction complete via API unload \
                     (process alive, VRAM freed, zero-downtime)"
                );
                tracing::debug!(
                    slot_save_ok,
                    unload_latency_ms = self.last_unload_latency_ms.load(Ordering::Acquire),
                    "orchestrator: Tier B eviction metrics"
                );
                self.health.update(
                    "llama-server",
                    crate::infra::health::ServiceStatus::Degraded,
                    Some("Tier B: model unloaded via API".into()),
                );
                return Ok(());
            }
            Err(api_err) => {
                let api_err_text = api_err.to_string();
                if is_router_mode_unavailable_error(&api_err_text) {
                    tracing::info!(
                        error = %api_err,
                        "orchestrator: API-level model unload unavailable on this llama-server; using legacy restart path"
                    );
                } else {
                    tracing::error!(
                        error = %api_err,
                        "orchestrator: API-level model unload FAILED — \
                         falling back to legacy SIGTERM/SIGKILL process restart."
                    );
                }
            }
        }

        // ── Legacy fallback: SIGTERM → wait → SIGKILL ────────────────────
        self.server_manager
            .graceful_stop_with_timeout(Duration::from_secs(
                self.config.graceful_stop_timeout_secs.max(1),
            ))
            .await;

        // Wait for NVML to confirm VRAM was actually reclaimed by the
        // driver. Without this, the next ComfyUI allocation can race the
        // CUDA cleanup and OOM.
        if self.backend == GpuBackend::Cuda {
            self.wait_for_vram_release_bounded(Duration::from_secs(
                self.config.vram_release_timeout_secs.max(1),
            ))
            .await;
        }

        // Respawn purely on CPU. We keep the same context window so the
        // restored KV cache is byte-compatible.
        let cpu_ctx = if prior_ctx > 0 {
            prior_ctx
        } else {
            self.config.model_profile.min_context
        };

        // FIX: Determine vision mode dynamically instead of hardcoding Disabled.
        // The mmproj weights can live in system RAM (CpuVision tier) even at
        // ngl=0. Hardcoding Disabled here was the root cause of the LLM going
        // blind during Tier B eviction despite having 8+ GB of free RAM.
        let eviction_free_ram_mb = {
            let mut sys = sysinfo::System::new();
            sys.refresh_memory();
            sys.available_memory() / (1024 * 1024)
        };
        let eviction_vision_mode = vision_strategy::determine_vision_mode(
            &self.config.model_profile,
            0, // ngl = 0 for CPU eviction
            eviction_free_ram_mb,
        );
        tracing::info!(
            free_ram_mb = eviction_free_ram_mb,
            ?eviction_vision_mode,
            "orchestrator: evict_to_cpu vision mode determined"
        );

        self.server_manager
            .spawn(0, cpu_ctx, eviction_vision_mode, self.event_bus.clone())
            .await
            .map_err(|e| anyhow::anyhow!("CPU-mode spawn failed: {e}"))?;

        // Best-effort: rehydrate the conversation.
        let slot_restore_ok = self.server_manager.restore_active_slot().await;
        self.record_slot_restore(slot_restore_ok);
        self.release_l1_lease(RecoveryReason::OwnerReleaseRequested);
        self.reconcile_l1_lease(false).await;
        tracing::debug!(
            slot_restore_ok,
            "orchestrator: Tier B eviction restore status"
        );

        self.health.update(
            "llama-server",
            crate::infra::health::ServiceStatus::Healthy,
            Some("Tier B: CPU-resident (legacy restart)".into()),
        );

        tracing::info!(
            prior_ngl,
            prior_ctx,
            "orchestrator: Tier B eviction complete (legacy process restart)"
        );
        Ok(())
    }

    pub async fn evict_to_cpu(&self) -> anyhow::Result<()> {
        self.evict_to_ram().await
    }

    /// Inverse of `evict_to_cpu`: hard-restart llama-server back onto the GPU
    /// using the strategy calculator's freshly-recomputed `(ngl, ctx)` based
    /// on currently-free VRAM. Best-effort restores the swap slot.
    pub async fn reload_to_vram(&self) -> anyhow::Result<()> {
        let _lifecycle_guard = self.lifecycle_lock.lock().await;

        let (current_ngl, current_ctx) = self.server_manager.current_params();
        if current_ngl > 0 {
            tracing::debug!(
                current_ngl,
                "orchestrator: restore_from_cpu no-op (already on GPU)"
            );
            self.claim_l1_lease("tier_b_restore_noop");
            self.reconcile_l1_lease(true).await;
            return Ok(());
        }

        tracing::info!("orchestrator: Tier B restore starting");

        self.stop_watchdog().await;

        self.health.update(
            "llama-server",
            crate::infra::health::ServiceStatus::Starting,
            Some("Tier B restore: reloading model (API-first)".into()),
        );

        // Preferred path: API-level reload (zero-downtime, no process restart).
        let load_started = Instant::now();
        match self.server_manager.api_load_model().await {
            Ok(()) => {
                self.last_load_latency_ms
                    .store(load_started.elapsed().as_millis() as u64, Ordering::Release);
                let slot_restore_ok = self.server_manager.restore_active_slot().await;
                self.record_slot_restore(slot_restore_ok);
                self.claim_l1_lease("tier_b_restore_api");
                self.reconcile_l1_lease(true).await;

                self.health.update(
                    "llama-server",
                    crate::infra::health::ServiceStatus::Healthy,
                    Some("Tier B: GPU-resident (API reload)".into()),
                );
                self.ensure_watchdog_running().await;

                tracing::debug!(
                    slot_restore_ok,
                    load_latency_ms = self.last_load_latency_ms.load(Ordering::Acquire),
                    "orchestrator: Tier B restore API metrics"
                );
                tracing::info!("orchestrator: Tier B restore complete via API load");
                return Ok(());
            }
            Err(api_err) => {
                let api_err_text = api_err.to_string();
                if is_router_mode_unavailable_error(&api_err_text) {
                    tracing::info!(
                        error = %api_err,
                        "orchestrator: API-level model load unavailable on this llama-server; using legacy respawn path"
                    );
                } else {
                    tracing::warn!(
                        error = %api_err,
                        "orchestrator: API-level model load failed; falling back to legacy respawn"
                    );
                }
            }
        }

        // Legacy fallback path: graceful stop + fresh GPU spawn.
        let slot_save_ok = self.server_manager.save_active_slot().await;
        self.record_slot_save(slot_save_ok);

        self.server_manager
            .graceful_stop_with_timeout(Duration::from_secs(
                self.config.graceful_stop_timeout_secs.max(1),
            ))
            .await;

        if self.backend == GpuBackend::Cuda {
            self.wait_for_vram_release_bounded(Duration::from_secs(
                self.config.vram_release_timeout_secs.max(1),
            ))
            .await;
        }

        // Recompute target params from the *current* free VRAM so we don't
        // demand more layers than the freshly-released GPU can hold.
        let snapshot = self.telemetry.snapshot().await;
        let target = strategy::calculate_target_params_prod(
            &self.config.model_profile,
            snapshot.free_vram_mb,
            self.config.safety_margin_mb,
            self.backend,
        );

        let target_ctx = if current_ctx > 0 {
            current_ctx
        } else {
            target.context
        };

        self.server_manager
            .spawn(
                target.ngl,
                target_ctx,
                target.vision_mode,
                self.event_bus.clone(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("GPU-mode respawn failed: {e}"))?;

        let slot_restore_ok = self.server_manager.restore_active_slot().await;
        self.record_slot_restore(slot_restore_ok);

        if target.ngl > 0 {
            self.claim_l1_lease("tier_b_restore_legacy");
        } else {
            self.release_l1_lease(RecoveryReason::OwnerReleaseRequested);
        }
        self.reconcile_l1_lease(target.ngl > 0).await;

        self.health.update(
            "llama-server",
            crate::infra::health::ServiceStatus::Healthy,
            None,
        );
        self.ensure_watchdog_running().await;

        tracing::info!(
            ngl = target.ngl,
            ctx = target_ctx,
            "orchestrator: Tier B restore complete"
        );
        Ok(())
    }

    pub async fn restore_from_cpu(&self) -> anyhow::Result<()> {
        self.reload_to_vram().await
    }
}

#[async_trait::async_trait]
impl runtime::L1Runtime for Orchestrator {
    fn snapshot(&self) -> OrchestratorSnapshot {
        Orchestrator::snapshot(self)
    }

    fn residency(&self) -> L1Residency {
        Orchestrator::l1_residency(self)
    }

    fn residency_metrics(&self) -> L1ResidencyMetrics {
        Orchestrator::residency_metrics(self)
    }

    async fn ensure_ready(&self, reason: &str) -> anyhow::Result<()> {
        Orchestrator::ensure_ready(self, reason).await
    }

    async fn release_if_idle(&self, reason: &str) -> anyhow::Result<bool> {
        Orchestrator::release_if_idle(self, reason).await
    }

    async fn evict_to_ram(&self) -> anyhow::Result<()> {
        Orchestrator::evict_to_ram(self).await
    }

    async fn reload_to_vram(&self) -> anyhow::Result<()> {
        Orchestrator::reload_to_vram(self).await
    }

    async fn evict_to_cpu(&self) -> anyhow::Result<()> {
        Orchestrator::evict_to_cpu(self).await
    }

    async fn restore_from_cpu(&self) -> anyhow::Result<()> {
        Orchestrator::restore_from_cpu(self).await
    }
}

#[async_trait::async_trait]
impl crate::resource::ResourceTelemetry for Orchestrator {
    async fn sample(&self) -> anyhow::Result<ResourceSnapshot> {
        let (ngl, _) = self.server_manager.current_params();
        Ok(self.build_resource_snapshot(ngl > 0).await)
    }
}

#[async_trait::async_trait]
impl crate::image::swap::LlmEvictionController for Orchestrator {
    fn is_gpu_resident(&self) -> bool {
        let (ngl, _) = self.server_manager.current_params();
        ngl > 0 && self.server_manager.is_healthy()
    }

    async fn evict_to_cpu(&self) -> Result<(), String> {
        Orchestrator::evict_to_cpu(self)
            .await
            .map_err(|e| e.to_string())
    }

    async fn restore_from_cpu(&self) -> Result<(), String> {
        Orchestrator::restore_from_cpu(self)
            .await
            .map_err(|e| e.to_string())
    }
}

impl Drop for Orchestrator {
    fn drop(&mut self) {
        if let Ok(mut lock) = self.watchdog_handle.try_lock() {
            if let Some(handle) = lock.take() {
                handle.abort();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::health::ServiceStatus;
    use crate::resource::ResourceTelemetry;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    fn assert_l1_runtime_impl<T: runtime::L1Runtime>() {}
    fn assert_resource_telemetry_impl<T: ResourceTelemetry>() {}

    struct TestTelemetry;

    #[async_trait]
    impl telemetry::GpuTelemetry for TestTelemetry {
        async fn snapshot(&self) -> telemetry::TelemetrySnapshot {
            telemetry::TelemetrySnapshot {
                free_vram_mb: 4096,
                total_vram_mb: 8192,
                gpu_util_pct: Some(10),
            }
        }

        fn source_name(&self) -> &'static str {
            "test"
        }
    }

    // ── Regression: startup ngl backoff ladder must never collapse ─────────────
    // (root-cause: cached==full previously skipped generating the fraction rungs)

    #[test]
    fn ladder_includes_fraction_rungs_when_cached_equals_full() {
        // This is the exact scenario that caused the bug: a prior successful boot
        // persisted the full ngl (36) as the "safe" cached value. A naive
        // implementation that seeds `cached` first and only builds `full`'s
        // fraction rungs when `!ladder.contains(&full)` would collapse to [36, 0].
        let ladder = build_ngl_backoff_ladder(36, Some(36));
        assert!(
            ladder.contains(&27) && ladder.contains(&18) && ladder.contains(&9),
            "ladder must retain all fraction rungs even when cached == full, got {:?}",
            ladder
        );
        assert_eq!(ladder.last(), Some(&0), "CPU fallback must always be last");
        assert_eq!(ladder[0], 36, "cached value should be tried first");
    }

    #[test]
    fn ladder_without_cache_has_full_backoff_sequence() {
        let ladder = build_ngl_backoff_ladder(36, None);
        assert_eq!(ladder, vec![36, 27, 18, 9, 0]);
    }

    #[test]
    fn ladder_with_lower_cache_puts_cache_first_but_keeps_full() {
        let ladder = build_ngl_backoff_ladder(36, Some(18));
        assert_eq!(ladder[0], 18);
        assert!(ladder.contains(&36));
        assert_eq!(ladder.last(), Some(&0));
    }

    #[test]
    fn ladder_ignores_cache_higher_than_full() {
        // A stale cache above the freshly computed full target must not be trusted.
        let ladder = build_ngl_backoff_ladder(18, Some(36));
        assert_eq!(ladder, vec![18, 13, 9, 4, 0]);
    }

    #[test]
    fn ladder_with_full_zero_is_just_cpu() {
        let ladder = build_ngl_backoff_ladder(0, None);
        assert_eq!(ladder, vec![0]);
    }

    #[test]
    fn ladder_has_no_duplicate_rungs() {
        let ladder = build_ngl_backoff_ladder(36, Some(36));
        let unique: std::collections::HashSet<u32> = ladder.iter().copied().collect();
        assert_eq!(
            unique.len(),
            ladder.len(),
            "ladder must not contain duplicates: {:?}",
            ladder
        );
    }

    fn build_test_orchestrator(config: OrchestratorConfig) -> Orchestrator {
        let health = Arc::new(HealthRegistry::new());
        health.register("llama-server");
        health.register("orchestrator");

        Orchestrator {
            config: config.clone(),
            backend: GpuBackend::CpuOnly,
            server_manager: Arc::new(server_manager::LlamaServerManager::new(
                config,
                "/tmp/kria_missing_model.gguf".into(),
                None,
            )),
            gpu_lease: Arc::new(GpuLeaseManager::default()),
            l1_lease_token: StdMutex::new(None),
            l1_hra_admission: tokio::sync::Mutex::new(None),
            telemetry: Arc::new(TestTelemetry),
            event_bus: Arc::new(EventBus::new(16)),
            health,
            watchdog_handle: Mutex::new(None),
            lifecycle_lock: Mutex::new(()),
            last_restart_at: Mutex::new(None),
            total_vram_mb: 0,
            last_unload_latency_ms: AtomicU64::new(0),
            last_load_latency_ms: AtomicU64::new(0),
            last_slot_save_ok: AtomicBool::new(false),
            last_slot_restore_ok: AtomicBool::new(false),
            remote_tool_bridge: StdMutex::new(None),
            remote_infra_observability: Arc::new(StdMutex::new(
                RemoteInfraObservabilityState::default(),
            )),
            _telemetry_actor: None,
        }
    }

    #[derive(Default)]
    struct MockEnvironment {
        command_outcomes: Mutex<VecDeque<Result<CommandResult, EnvironmentError>>>,
        read_result: Mutex<Option<Result<ReadFileResult, EnvironmentError>>>,
        write_result: Mutex<Option<Result<WriteFileResult, EnvironmentError>>>,
        list_result: Mutex<Option<Result<ListDirResult, EnvironmentError>>>,
        reset_calls: AtomicUsize,
        ensure_calls: AtomicUsize,
    }

    impl MockEnvironment {
        async fn push_command_outcome(&self, outcome: Result<CommandResult, EnvironmentError>) {
            self.command_outcomes.lock().await.push_back(outcome);
        }

        async fn set_read_result(&self, result: Result<ReadFileResult, EnvironmentError>) {
            *self.read_result.lock().await = Some(result);
        }

        async fn set_write_result(&self, result: Result<WriteFileResult, EnvironmentError>) {
            *self.write_result.lock().await = Some(result);
        }

        async fn set_list_result(&self, result: Result<ListDirResult, EnvironmentError>) {
            *self.list_result.lock().await = Some(result);
        }
    }

    #[async_trait]
    impl CommandExecutor for MockEnvironment {
        async fn execute_command(
            &self,
            _request: CommandRequest,
            _shell_state_snapshot: ShellState,
        ) -> Result<CommandResult, EnvironmentError> {
            self.command_outcomes
                .lock()
                .await
                .pop_front()
                .unwrap_or_else(|| {
                    Err(EnvironmentError::ProviderUnavailable {
                        provider: "mock".to_string(),
                        details: "missing command outcome".to_string(),
                    })
                })
        }
    }

    #[async_trait]
    impl FileSystemOps for MockEnvironment {
        async fn read_file(
            &self,
            _request: ReadFileRequest,
        ) -> Result<ReadFileResult, EnvironmentError> {
            self.read_result.lock().await.take().unwrap_or_else(|| {
                Err(EnvironmentError::ProviderUnavailable {
                    provider: "mock".to_string(),
                    details: "missing read_file result".to_string(),
                })
            })
        }

        async fn write_file(
            &self,
            _request: WriteFileRequest,
        ) -> Result<WriteFileResult, EnvironmentError> {
            self.write_result.lock().await.take().unwrap_or_else(|| {
                Err(EnvironmentError::ProviderUnavailable {
                    provider: "mock".to_string(),
                    details: "missing write_file result".to_string(),
                })
            })
        }

        async fn list_dir(
            &self,
            _request: ListDirRequest,
        ) -> Result<ListDirResult, EnvironmentError> {
            self.list_result.lock().await.take().unwrap_or_else(|| {
                Err(EnvironmentError::ProviderUnavailable {
                    provider: "mock".to_string(),
                    details: "missing list_dir result".to_string(),
                })
            })
        }
    }

    #[async_trait]
    impl EnvironmentLifecycle for MockEnvironment {
        async fn ensure_ready(&self) -> Result<(), EnvironmentError> {
            self.ensure_calls.fetch_add(1, AtomicOrdering::AcqRel);
            Ok(())
        }

        async fn reset_environment(&self, _reason: ResetReason) -> Result<(), EnvironmentError> {
            self.reset_calls.fetch_add(1, AtomicOrdering::AcqRel);
            Ok(())
        }

        async fn shutdown(&self) -> Result<(), EnvironmentError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn release_if_idle_returns_false_without_live_process() {
        let orchestrator = build_test_orchestrator(OrchestratorConfig::default());

        let released = orchestrator
            .release_if_idle("unit_test_no_process")
            .await
            .expect("release_if_idle should not error when process is absent");

        assert!(!released);
    }

    #[tokio::test]
    async fn ensure_ready_marks_health_degraded_on_spawn_failure() {
        let mut config = OrchestratorConfig::default();
        config.health_check_timeout_secs = 1;
        config.port_discovery_timeout_secs = 1;

        let orchestrator = build_test_orchestrator(config);

        let result = orchestrator.ensure_ready("unit_test_failure").await;
        assert!(
            result.is_err(),
            "ensure_ready should fail without a valid model/runtime"
        );

        let llama_health = orchestrator
            .health
            .get("llama-server")
            .expect("llama-server health should be registered");
        let orchestrator_health = orchestrator
            .health
            .get("orchestrator")
            .expect("orchestrator health should be registered");

        assert_eq!(llama_health.status, ServiceStatus::Degraded);
        assert_eq!(orchestrator_health.status, ServiceStatus::Degraded);
        assert!(
            llama_health
                .message
                .unwrap_or_default()
                .contains("ensure_ready failed"),
            "llama health message should include ensure_ready failure context"
        );
    }

    #[test]
    fn orchestrator_implements_runtime_boundaries() {
        assert_l1_runtime_impl::<Orchestrator>();
        assert_resource_telemetry_impl::<Orchestrator>();
    }

    #[tokio::test]
    async fn remote_tool_bridge_retries_once_after_reset_required() {
        let env = Arc::new(MockEnvironment::default());
        env.push_command_outcome(Err(EnvironmentError::EnvironmentResetRequired {
            reason: "tainted generation".to_string(),
        }))
        .await;
        env.push_command_outcome(Ok(CommandResult {
            exit_code: 0,
            stdout: "ok".to_string(),
            stderr: String::new(),
            truncated: false,
        }))
        .await;

        let lifecycle_events = Arc::new(StdMutex::new(Vec::new()));
        let event_sink = Arc::clone(&lifecycle_events);
        let bridge = RemoteEnvironmentToolBridge::new(Arc::clone(&env))
            .with_reset_lifecycle_callback(Arc::new(move |stage, reason| {
                event_sink
                    .lock()
                    .expect("event sink lock poisoned")
                    .push((stage, reason.to_string()));
            }));

        let outcome = bridge
            .dispatch_tool_call(RemoteToolCallIntent::ExecuteCommand {
                request: CommandRequest {
                    program: "echo".to_string(),
                    args: vec!["hello".to_string()],
                    timeout_ms: 100,
                    max_bytes: 4096,
                    max_lines: 64,
                },
                shell_state: ShellState::default(),
            })
            .await
            .expect("tool-call should succeed on single retry");

        assert_eq!(
            outcome,
            RemoteToolCallOutcome::Command(CommandResult {
                exit_code: 0,
                stdout: "ok".to_string(),
                stderr: String::new(),
                truncated: false,
            })
        );
        assert_eq!(env.reset_calls.load(AtomicOrdering::Acquire), 1);
        assert_eq!(env.ensure_calls.load(AtomicOrdering::Acquire), 1);

        let events = lifecycle_events
            .lock()
            .expect("event sink lock poisoned")
            .clone();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].0, RemoteResetLifecycleStage::AgentPaused);
        assert_eq!(events[1].0, RemoteResetLifecycleStage::ResetStarted);
        assert_eq!(events[2].0, RemoteResetLifecycleStage::ResetHealthy);
        assert_eq!(events[3].0, RemoteResetLifecycleStage::AgentResumed);
    }

    #[tokio::test]
    async fn remote_tool_bridge_maps_filesystem_intents() {
        let env = Arc::new(MockEnvironment::default());
        env.set_read_result(Ok(ReadFileResult {
            contents: b"abc".to_vec(),
        }))
        .await;
        env.set_write_result(Ok(WriteFileResult { bytes_written: 3 }))
            .await;
        env.set_list_result(Ok(ListDirResult {
            entries: vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")],
        }))
        .await;

        let bridge = RemoteEnvironmentToolBridge::new(env);

        let read = bridge
            .dispatch_tool_call(RemoteToolCallIntent::ReadFile {
                request: ReadFileRequest {
                    path: PathBuf::from("/tmp/input"),
                },
            })
            .await
            .expect("read_file intent should map into FileSystemOps::read_file");
        assert_eq!(
            read,
            RemoteToolCallOutcome::ReadFile(ReadFileResult {
                contents: b"abc".to_vec(),
            })
        );

        let write = bridge
            .dispatch_tool_call(RemoteToolCallIntent::WriteFile {
                request: WriteFileRequest {
                    path: PathBuf::from("/tmp/output"),
                    contents: b"abc".to_vec(),
                    create_parent: true,
                },
            })
            .await
            .expect("write_file intent should map into FileSystemOps::write_file");
        assert_eq!(
            write,
            RemoteToolCallOutcome::WriteFile(WriteFileResult { bytes_written: 3 })
        );

        let list = bridge
            .dispatch_tool_call(RemoteToolCallIntent::ListDir {
                request: ListDirRequest {
                    path: PathBuf::from("/tmp"),
                },
            })
            .await
            .expect("list_dir intent should map into FileSystemOps::list_dir");
        assert_eq!(
            list,
            RemoteToolCallOutcome::ListDir(ListDirResult {
                entries: vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")],
            })
        );
    }
}
