use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::config::KriaSystemConfig;
use crate::infra::environment::remote_qemu::QemuSshEnvironment;
use crate::infra::environment::{EnvironmentError, EnvironmentLifecycle};
use crate::infra::qos::{AdaptiveQosScheduler, QosAdaptationPacket, QosAdmission, QosClass};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TargetId(pub Uuid);

impl Default for TargetId {
    fn default() -> Self {
        Self::new()
    }
}

impl TargetId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LeaseId(pub Uuid);

impl Default for LeaseId {
    fn default() -> Self {
        Self::new()
    }
}

impl LeaseId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Clone)]
pub struct LeaseHandle {
    pub target_id: TargetId,
    pub lease_id: LeaseId,
    pub heartbeat_ttl: Duration,
    pub expires_at: Instant,
    pub last_heartbeat_at: Instant,
}

impl LeaseHandle {
    fn new(target_id: TargetId, heartbeat_ttl: Duration, now: Instant) -> Self {
        Self {
            target_id,
            lease_id: LeaseId::new(),
            heartbeat_ttl,
            expires_at: now + heartbeat_ttl,
            last_heartbeat_at: now,
        }
    }

    fn renew(&mut self, now: Instant) {
        self.last_heartbeat_at = now;
        self.expires_at = now + self.heartbeat_ttl;
    }

    fn is_expired(&self, now: Instant, heartbeat_grace: Duration) -> bool {
        now > self.expires_at + heartbeat_grace
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetHealthTelemetry {
    pub health_score: f64,
    pub latency_ewma_ms: f64,
    pub recent_failure_rate: f64,
}

impl Default for TargetHealthTelemetry {
    fn default() -> Self {
        Self {
            health_score: 1.0,
            latency_ewma_ms: 50.0,
            recent_failure_rate: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetPoolConfig {
    pub lease_ttl_ms: u64,
    pub heartbeat_grace_ms: u64,
    pub quarantine_cooldown_ms: u64,
}

impl TargetPoolConfig {
    pub fn from_system_config(system_config: &KriaSystemConfig) -> Self {
        Self {
            lease_ttl_ms: system_config.target_pool.lease_ttl_ms,
            heartbeat_grace_ms: system_config.target_pool.heartbeat_grace_ms,
            quarantine_cooldown_ms: system_config.target_pool.quarantine_cooldown_ms,
        }
    }
}

impl Default for TargetPoolConfig {
    fn default() -> Self {
        Self::from_system_config(&KriaSystemConfig::default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionWeights {
    pub health: f64,
    pub latency: f64,
    pub failure: f64,
}

impl Default for SelectionWeights {
    fn default() -> Self {
        Self {
            health: 0.50,
            latency: 0.30,
            failure: 0.20,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QuarantineState {
    pub reason: String,
    pub quarantined_at: Instant,
    pub cooldown_until: Instant,
    pub failed_probe_count: u32,
    pub last_probe_detail: Option<String>,
}

#[derive(Debug, Clone)]
pub enum InventoryState {
    Ready,
    Leased {
        lease_id: LeaseId,
        expires_at: Instant,
    },
    Tainted {
        reason: String,
        tainted_at: Instant,
    },
    Quarantined(QuarantineState),
}

#[derive(Debug, Default)]
pub struct InventoryRegistry {
    states: HashMap<TargetId, InventoryState>,
}

impl InventoryRegistry {
    fn insert_ready(&mut self, target_id: TargetId) {
        self.states.insert(target_id, InventoryState::Ready);
    }

    fn remove(&mut self, target_id: &TargetId) {
        self.states.remove(target_id);
    }

    fn state(&self, target_id: &TargetId) -> Option<InventoryState> {
        self.states.get(target_id).cloned()
    }

    fn ready_target_ids(&self) -> Vec<TargetId> {
        self.states
            .iter()
            .filter_map(|(target_id, state)| {
                if matches!(state, InventoryState::Ready) {
                    Some(target_id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    fn transition_ready_to_leased(
        &mut self,
        target_id: &TargetId,
        lease_id: LeaseId,
        expires_at: Instant,
    ) -> Result<(), EnvironmentError> {
        let Some(state) = self.states.get(target_id) else {
            return Err(EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: format!("target {} not found", target_id.0),
            });
        };

        if !matches!(state, InventoryState::Ready) {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!(
                    "target {} not in Ready state during lease acquisition",
                    target_id.0
                ),
            });
        }

        self.states.insert(
            target_id.clone(),
            InventoryState::Leased {
                lease_id,
                expires_at,
            },
        );
        Ok(())
    }

    fn transition_leased_to_ready(
        &mut self,
        target_id: &TargetId,
        lease_id: &LeaseId,
    ) -> Result<(), EnvironmentError> {
        let Some(state) = self.states.get(target_id) else {
            return Err(EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: format!("target {} not found", target_id.0),
            });
        };

        match state {
            InventoryState::Leased {
                lease_id: active, ..
            } if active == lease_id => {
                self.states.insert(target_id.clone(), InventoryState::Ready);
                Ok(())
            }
            _ => Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!(
                    "target {} lease {} is not active in inventory",
                    target_id.0, lease_id.0
                ),
            }),
        }
    }

    fn transition_to_tainted(
        &mut self,
        target_id: &TargetId,
        reason: String,
    ) -> Result<(), EnvironmentError> {
        if !self.states.contains_key(target_id) {
            return Err(EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: format!("target {} not found", target_id.0),
            });
        }

        self.states.insert(
            target_id.clone(),
            InventoryState::Tainted {
                reason,
                tainted_at: Instant::now(),
            },
        );
        Ok(())
    }

    fn transition_to_quarantined(
        &mut self,
        target_id: &TargetId,
        reason: String,
        cooldown: Duration,
    ) -> Result<(), EnvironmentError> {
        if !self.states.contains_key(target_id) {
            return Err(EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: format!("target {} not found", target_id.0),
            });
        }

        let now = Instant::now();
        self.states.insert(
            target_id.clone(),
            InventoryState::Quarantined(QuarantineState {
                reason,
                quarantined_at: now,
                cooldown_until: now + cooldown,
                failed_probe_count: 0,
                last_probe_detail: None,
            }),
        );
        Ok(())
    }

    fn bump_quarantine_probe_failure(
        &mut self,
        target_id: &TargetId,
        detail: String,
    ) -> Result<(), EnvironmentError> {
        let Some(state) = self.states.get_mut(target_id) else {
            return Err(EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: format!("target {} not found", target_id.0),
            });
        };

        match state {
            InventoryState::Quarantined(q) => {
                q.failed_probe_count = q.failed_probe_count.saturating_add(1);
                q.last_probe_detail = Some(detail);
                Ok(())
            }
            _ => Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!(
                    "target {} is not quarantined while recording probe failure",
                    target_id.0
                ),
            }),
        }
    }

    fn transition_to_ready(&mut self, target_id: &TargetId) -> Result<(), EnvironmentError> {
        if !self.states.contains_key(target_id) {
            return Err(EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: format!("target {} not found", target_id.0),
            });
        }

        self.states.insert(target_id.clone(), InventoryState::Ready);
        Ok(())
    }

    fn occupancy_counts(&self) -> (usize, usize, usize, usize) {
        let mut ready = 0usize;
        let mut leased = 0usize;
        let mut tainted = 0usize;
        let mut quarantined = 0usize;

        for state in self.states.values() {
            match state {
                InventoryState::Ready => ready += 1,
                InventoryState::Leased { .. } => leased += 1,
                InventoryState::Tainted { .. } => tainted += 1,
                InventoryState::Quarantined(_) => quarantined += 1,
            }
        }

        (ready, leased, tainted, quarantined)
    }

    fn total_targets(&self) -> usize {
        self.states.len()
    }
}

struct TargetEntry {
    environment: Arc<QemuSshEnvironment>,
    telemetry: TargetHealthTelemetry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OccupancySnapshot {
    pub total_targets: usize,
    pub ready_targets: usize,
    pub leased_targets: usize,
    pub tainted_targets: usize,
    pub quarantined_targets: usize,
    pub active_leases: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolTelemetryPacket {
    pub timestamp_unix_ms: u64,
    pub event: String,
    pub total_targets: usize,
    pub ready_targets: usize,
    pub leased_targets: usize,
    pub tainted_targets: usize,
    pub quarantined_targets: usize,
    pub active_leases: usize,
    pub expired_lease_count: u64,
}

pub type PoolTelemetryCallback = Arc<dyn Fn(PoolTelemetryPacket) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct HealthGateProbeResult {
    pub passed: bool,
    pub detail: String,
    pub latency_ms: u64,
}

#[async_trait]
pub trait HealthGateProbe: Send + Sync {
    fn name(&self) -> &'static str;
    async fn run(
        &self,
        target: Arc<QemuSshEnvironment>,
    ) -> Result<HealthGateProbeResult, EnvironmentError>;
}

#[derive(Debug, Default)]
pub struct DiskHeadroomProbe;

#[async_trait]
impl HealthGateProbe for DiskHeadroomProbe {
    fn name(&self) -> &'static str {
        "disk_headroom"
    }

    async fn run(
        &self,
        target: Arc<QemuSshEnvironment>,
    ) -> Result<HealthGateProbeResult, EnvironmentError> {
        let started = Instant::now();
        target.probe_disk_headroom().await?;
        Ok(HealthGateProbeResult {
            passed: true,
            detail: "disk headroom probe passed".to_string(),
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }
}

#[derive(Debug, Default)]
pub struct WriteabilityProbe;

#[async_trait]
impl HealthGateProbe for WriteabilityProbe {
    fn name(&self) -> &'static str {
        "writeability"
    }

    async fn run(
        &self,
        target: Arc<QemuSshEnvironment>,
    ) -> Result<HealthGateProbeResult, EnvironmentError> {
        let started = Instant::now();
        target.probe_writeability().await?;
        Ok(HealthGateProbeResult {
            passed: true,
            detail: "writeability probe passed".to_string(),
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }
}

#[derive(Debug, Default)]
pub struct TransportProbe;

#[async_trait]
impl HealthGateProbe for TransportProbe {
    fn name(&self) -> &'static str {
        "transport"
    }

    async fn run(
        &self,
        target: Arc<QemuSshEnvironment>,
    ) -> Result<HealthGateProbeResult, EnvironmentError> {
        let started = Instant::now();
        target.probe_transport_health().await?;
        Ok(HealthGateProbeResult {
            passed: true,
            detail: "transport probe passed".to_string(),
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }
}

#[derive(Debug, Default)]
pub struct EnsureReadyProbe;

#[async_trait]
impl HealthGateProbe for EnsureReadyProbe {
    fn name(&self) -> &'static str {
        "ensure_ready"
    }

    async fn run(
        &self,
        target: Arc<QemuSshEnvironment>,
    ) -> Result<HealthGateProbeResult, EnvironmentError> {
        let started = Instant::now();
        target.ensure_ready().await?;
        Ok(HealthGateProbeResult {
            passed: true,
            detail: "target ensure_ready passed".to_string(),
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }
}

#[derive(Debug, Default)]
pub struct AdmissionBarrierProbe;

#[async_trait]
impl HealthGateProbe for AdmissionBarrierProbe {
    fn name(&self) -> &'static str {
        "admission_barrier"
    }

    async fn run(
        &self,
        target: Arc<QemuSshEnvironment>,
    ) -> Result<HealthGateProbeResult, EnvironmentError> {
        let started = Instant::now();
        target.probe_admission_barrier().await?;
        Ok(HealthGateProbeResult {
            passed: true,
            detail: "admission barrier probe passed".to_string(),
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }
}

/// RFC-003 TargetPool manager for multi-target lifecycle, leasing, tainting, and quarantine.
pub struct TargetPool {
    config: TargetPoolConfig,
    selection_weights: SelectionWeights,
    qos: Arc<AdaptiveQosScheduler>,
    targets: RwLock<HashMap<TargetId, TargetEntry>>,
    inventory: RwLock<InventoryRegistry>,
    leases: RwLock<HashMap<LeaseId, LeaseHandle>>,
    probes: RwLock<Vec<Arc<dyn HealthGateProbe>>>,
    expired_lease_count: AtomicU64,
    telemetry_callbacks: StdMutex<Vec<PoolTelemetryCallback>>,
    latest_telemetry_packet: StdMutex<Option<PoolTelemetryPacket>>,
}

impl TargetPool {
    pub fn new(
        system_config: &KriaSystemConfig,
        selection_weights: SelectionWeights,
        qos: Arc<AdaptiveQosScheduler>,
    ) -> Self {
        Self::with_config(
            TargetPoolConfig::from_system_config(system_config),
            selection_weights,
            qos,
        )
    }

    pub fn with_config(
        config: TargetPoolConfig,
        selection_weights: SelectionWeights,
        qos: Arc<AdaptiveQosScheduler>,
    ) -> Self {
        Self {
            config,
            selection_weights,
            qos,
            targets: RwLock::new(HashMap::new()),
            inventory: RwLock::new(InventoryRegistry::default()),
            leases: RwLock::new(HashMap::new()),
            probes: RwLock::new(Vec::new()),
            expired_lease_count: AtomicU64::new(0),
            telemetry_callbacks: StdMutex::new(Vec::new()),
            latest_telemetry_packet: StdMutex::new(None),
        }
    }

    pub fn register_telemetry_callback(&self, callback: PoolTelemetryCallback) {
        self.telemetry_callbacks
            .lock()
            .expect("target_pool telemetry callback lock poisoned")
            .push(callback);
    }

    pub fn latest_telemetry_packet(&self) -> Option<PoolTelemetryPacket> {
        self.latest_telemetry_packet
            .lock()
            .expect("target_pool latest telemetry lock poisoned")
            .clone()
    }

    pub fn qos_adaptation_snapshot(&self, limit: usize) -> Vec<QosAdaptationPacket> {
        self.qos.adaptation_snapshot(limit)
    }

    pub async fn register_default_probes(&self) {
        let mut probes = self.probes.write().await;
        probes.push(Arc::new(DiskHeadroomProbe));
        probes.push(Arc::new(WriteabilityProbe));
        probes.push(Arc::new(TransportProbe));
        probes.push(Arc::new(EnsureReadyProbe));
        probes.push(Arc::new(AdmissionBarrierProbe));
    }

    pub async fn register_probe(&self, probe: Arc<dyn HealthGateProbe>) {
        self.probes.write().await.push(probe);
    }

    pub async fn add_target(
        &self,
        target_id: TargetId,
        environment: Arc<QemuSshEnvironment>,
        telemetry: TargetHealthTelemetry,
    ) {
        {
            let mut targets = self.targets.write().await;
            targets.insert(
                target_id.clone(),
                TargetEntry {
                    environment,
                    telemetry,
                },
            );
        }

        self.inventory.write().await.insert_ready(target_id);
        self.emit_packet("target_added").await;
    }

    /// Remove a target from the pool. Evicts from both the target map and inventory.
    /// Use when a user unenrolls/deletes a VM.
    pub fn remove_target(&self, target_id: Uuid) {
        let id = TargetId(target_id);
        // Use try_write to avoid blocking — best-effort removal
        if let Ok(mut targets) = self.targets.try_write() {
            targets.remove(&id);
        }
        if let Ok(mut inventory) = self.inventory.try_write() {
            inventory.remove(&id);
        }
    }

    pub async fn inventory_state(&self, target_id: &TargetId) -> Option<InventoryState> {
        self.inventory.read().await.state(target_id)
    }

    pub async fn update_target_telemetry(
        &self,
        target_id: &TargetId,
        telemetry: TargetHealthTelemetry,
    ) -> Result<(), EnvironmentError> {
        let mut targets = self.targets.write().await;
        let Some(entry) = targets.get_mut(target_id) else {
            return Err(EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: format!("target {} not found", target_id.0),
            });
        };

        entry.telemetry = telemetry;
        Ok(())
    }

    pub async fn occupancy_snapshot(&self) -> OccupancySnapshot {
        let inventory = self.inventory.read().await;
        let leases = self.leases.read().await;
        let (ready_targets, leased_targets, tainted_targets, quarantined_targets) =
            inventory.occupancy_counts();

        OccupancySnapshot {
            total_targets: inventory.total_targets(),
            ready_targets,
            leased_targets,
            tainted_targets,
            quarantined_targets,
            active_leases: leases.len(),
        }
    }

    pub async fn acquire_lease(&self) -> Result<LeaseHandle, EnvironmentError> {
        self.reap_expired_leases().await?;

        let started = Instant::now();
        let admission = self
            .qos
            .try_start_task(QosClass::MediumReconnect, "target_pool::acquire_lease");
        if let Some(error) = map_qos_denial(admission) {
            return Err(error);
        }

        let selected_target = self.select_best_ready_target().await.ok_or_else(|| {
            EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: "no ready target available for lease".to_string(),
            }
        })?;

        let ttl = Duration::from_millis(self.config.lease_ttl_ms.max(1));
        let lease = LeaseHandle::new(selected_target.clone(), ttl, Instant::now());

        {
            let mut inventory = self.inventory.write().await;
            inventory.transition_ready_to_leased(
                &selected_target,
                lease.lease_id.clone(),
                lease.expires_at,
            )?;
        }

        let Some(environment) = self.environment_for_target(&selected_target).await else {
            let mut inventory = self.inventory.write().await;
            let _ = inventory.transition_to_ready(&selected_target);
            self.qos.finish_task(
                QosClass::MediumReconnect,
                started.elapsed().as_millis() as u64,
                false,
            );
            return Err(EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: format!("selected target {} vanished", selected_target.0),
            });
        };

        if let Err(error) = environment
            .activate_verified_lease(lease.lease_id.0, lease.heartbeat_ttl)
            .await
        {
            let taint_reason = format!(
                "lease {} activation failed on target {}",
                lease.lease_id.0, selected_target.0
            );

            {
                let mut inventory = self.inventory.write().await;
                let _ = inventory.transition_to_tainted(&selected_target, taint_reason.clone());
            }
            mark_environment_tainted(&environment, taint_reason).await;

            self.qos.finish_task(
                QosClass::MediumReconnect,
                started.elapsed().as_millis() as u64,
                false,
            );
            self.emit_packet("lease_acquire_failed").await;
            return Err(error);
        }

        {
            let mut leases = self.leases.write().await;
            leases.insert(lease.lease_id.clone(), lease.clone());
        }

        self.qos.finish_task(
            QosClass::MediumReconnect,
            started.elapsed().as_millis() as u64,
            true,
        );
        self.emit_packet("lease_acquired").await;

        Ok(lease)
    }

    pub async fn heartbeat(&self, lease_id: &LeaseId) -> Result<LeaseHandle, EnvironmentError> {
        self.reap_expired_leases().await?;

        let now = Instant::now();
        let grace = Duration::from_millis(self.config.heartbeat_grace_ms.max(1));

        let mut expired_target: Option<TargetId> = None;
        let mut renewed: Option<LeaseHandle> = None;

        {
            let mut leases = self.leases.write().await;
            let Some(active) = leases.get_mut(lease_id) else {
                return Err(EnvironmentError::EnvironmentResetRequired {
                    reason: format!("unknown lease {} during heartbeat", lease_id.0),
                });
            };

            if active.is_expired(now, grace) {
                expired_target = Some(active.target_id.clone());
                leases.remove(lease_id);
            } else {
                active.renew(now);
                renewed = Some(active.clone());
            }
        }

        if let Some(target_id) = expired_target {
            self.expired_lease_count.fetch_add(1, Ordering::AcqRel);
            self.taint_target_for_lease_expiry(&target_id, lease_id)
                .await?;
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!(
                    "lease {} expired before heartbeat; target tainted",
                    lease_id.0
                ),
            });
        }

        let renewed = renewed.ok_or_else(|| EnvironmentError::EnvironmentResetRequired {
            reason: format!("lease {} heartbeat renew race", lease_id.0),
        })?;

        let Some(environment) = self.environment_for_target(&renewed.target_id).await else {
            self.leases.write().await.remove(lease_id);
            self.expired_lease_count.fetch_add(1, Ordering::AcqRel);
            self.taint_target_for_lease_expiry(&renewed.target_id, lease_id)
                .await?;
            return Err(EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: format!("target {} missing during heartbeat", renewed.target_id.0),
            });
        };

        if let Err(error) = environment
            .renew_verified_lease(renewed.lease_id.0, renewed.heartbeat_ttl)
            .await
        {
            self.leases.write().await.remove(lease_id);
            self.expired_lease_count.fetch_add(1, Ordering::AcqRel);
            self.taint_target_for_lease_expiry(&renewed.target_id, lease_id)
                .await?;
            return Err(error);
        }

        Ok(renewed)
    }

    pub async fn release_lease(&self, lease_id: &LeaseId) -> Result<(), EnvironmentError> {
        let Some(lease_entry) = self.leases.write().await.remove(lease_id) else {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!("release called for unknown lease {}", lease_id.0),
            });
        };

        {
            let mut inventory = self.inventory.write().await;
            inventory.transition_leased_to_ready(&lease_entry.target_id, lease_id)?;
        }

        if let Some(environment) = self.environment_for_target(&lease_entry.target_id).await {
            environment.clear_verified_lease().await;
        }

        self.emit_packet("lease_released").await;
        Ok(())
    }

    pub async fn environment_for_lease(
        &self,
        lease: &LeaseHandle,
    ) -> Result<Arc<QemuSshEnvironment>, EnvironmentError> {
        let Some(active_lease) = self.leases.read().await.get(&lease.lease_id).cloned() else {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!(
                    "no active lease {} for target {}",
                    lease.lease_id.0, lease.target_id.0
                ),
            });
        };

        if active_lease.target_id != lease.target_id {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!(
                    "lease {} target mismatch: expected {}, found {}",
                    lease.lease_id.0, lease.target_id.0, active_lease.target_id.0
                ),
            });
        }

        self.environment_for_target(&lease.target_id)
            .await
            .ok_or_else(|| EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: format!(
                    "target {} missing for lease {}",
                    lease.target_id.0, lease.lease_id.0
                ),
            })
    }

    pub async fn quarantine_target(
        &self,
        target_id: &TargetId,
        reason: &str,
    ) -> Result<(), EnvironmentError> {
        {
            let mut inventory = self.inventory.write().await;
            inventory.transition_to_tainted(target_id, reason.to_string())?;
            inventory.transition_to_quarantined(
                target_id,
                reason.to_string(),
                Duration::from_millis(self.config.quarantine_cooldown_ms.max(1)),
            )?;
        }

        let Some(environment) = self.environment_for_target(target_id).await else {
            return Err(EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: format!("target {} not found", target_id.0),
            });
        };

        mark_environment_tainted(&environment, reason.to_string()).await;
        self.emit_packet("target_quarantined").await;
        Ok(())
    }

    pub async fn run_quarantine_health_gates(
        &self,
        target_id: &TargetId,
    ) -> Result<(), EnvironmentError> {
        let state = self
            .inventory
            .read()
            .await
            .state(target_id)
            .ok_or_else(|| EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: format!("target {} not found", target_id.0),
            })?;

        if let InventoryState::Tainted { reason, .. } = state {
            {
                let mut inventory = self.inventory.write().await;
                inventory.transition_to_quarantined(
                    target_id,
                    format!("tainted gate entry: {reason}"),
                    Duration::from_millis(self.config.quarantine_cooldown_ms.max(1)),
                )?;
            }

            self.emit_packet("target_quarantined").await;
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!(
                    "target {} moved from tainted to quarantine cooldown",
                    target_id.0
                ),
            });
        }

        let cooldown_until = match state {
            InventoryState::Quarantined(q) => q.cooldown_until,
            _ => {
                return Err(EnvironmentError::EnvironmentResetRequired {
                    reason: format!("target {} is neither tainted nor quarantined", target_id.0),
                });
            }
        };

        if Instant::now() < cooldown_until {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!("target {} still cooling down in quarantine", target_id.0),
            });
        }

        let Some(environment) = self.environment_for_target(target_id).await else {
            return Err(EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: format!("target {} not found", target_id.0),
            });
        };

        let probes = self.probes.read().await.clone();
        if probes.is_empty() {
            return Err(EnvironmentError::StartupPolicyNotReady {
                policy: "target_pool_health_gate_probes".to_string(),
                details: "no health-gate probes registered".to_string(),
            });
        }

        for probe in probes {
            let outcome = probe.run(Arc::clone(&environment)).await?;
            if !outcome.passed {
                let detail = format!(
                    "probe={} failed latency_ms={} detail={}",
                    probe.name(),
                    outcome.latency_ms,
                    outcome.detail
                );
                {
                    let mut inventory = self.inventory.write().await;
                    inventory.bump_quarantine_probe_failure(target_id, detail.clone())?;
                    inventory.transition_to_tainted(target_id, detail.clone())?;
                }
                mark_environment_tainted(&environment, detail.clone()).await;
                self.emit_packet("target_tainted").await;
                return Err(EnvironmentError::EnvironmentResetRequired {
                    reason: format!("health gate failed: {detail}"),
                });
            }
        }

        {
            let mut inventory = self.inventory.write().await;
            inventory.transition_to_ready(target_id)?;
        }

        environment.tainted.store(false, Ordering::Release);
        environment.taint_reason.lock().await.take();

        self.emit_packet("quarantine_exit_ready").await;
        Ok(())
    }

    async fn select_best_ready_target(&self) -> Option<TargetId> {
        let ready = self.inventory.read().await.ready_target_ids();
        if ready.is_empty() {
            return None;
        }

        let targets = self.targets.read().await;
        ready
            .into_iter()
            .filter_map(|target_id| {
                targets.get(&target_id).map(|entry| {
                    (
                        target_id,
                        weighted_score(entry.telemetry, self.selection_weights),
                    )
                })
            })
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(target_id, _)| target_id)
    }

    async fn environment_for_target(
        &self,
        target_id: &TargetId,
    ) -> Option<Arc<QemuSshEnvironment>> {
        self.targets
            .read()
            .await
            .get(target_id)
            .map(|entry| Arc::clone(&entry.environment))
    }

    async fn taint_target_for_lease_expiry(
        &self,
        target_id: &TargetId,
        lease_id: &LeaseId,
    ) -> Result<(), EnvironmentError> {
        let reason = format!(
            "lease {} heartbeat expired; fail-closed taint applied",
            lease_id.0
        );

        {
            let mut inventory = self.inventory.write().await;
            inventory.transition_to_tainted(target_id, reason.clone())?;
        }

        let Some(environment) = self.environment_for_target(target_id).await else {
            return Err(EnvironmentError::ProviderUnavailable {
                provider: "target_pool".to_string(),
                details: format!("target {} not found", target_id.0),
            });
        };

        mark_environment_tainted(&environment, reason).await;
        environment.clear_verified_lease().await;
        self.emit_packet("target_tainted").await;
        Ok(())
    }

    async fn reap_expired_leases(&self) -> Result<(), EnvironmentError> {
        let now = Instant::now();
        let grace = Duration::from_millis(self.config.heartbeat_grace_ms.max(1));

        let expired = {
            let leases = self.leases.read().await;
            leases
                .values()
                .filter(|lease| lease.is_expired(now, grace))
                .map(|lease| (lease.lease_id.clone(), lease.target_id.clone()))
                .collect::<Vec<_>>()
        };

        if expired.is_empty() {
            return Ok(());
        }

        {
            let mut leases = self.leases.write().await;
            for (lease_id, _) in &expired {
                leases.remove(lease_id);
            }
        }

        for (lease_id, target_id) in expired {
            self.expired_lease_count.fetch_add(1, Ordering::AcqRel);
            self.taint_target_for_lease_expiry(&target_id, &lease_id)
                .await?;
        }

        Ok(())
    }

    async fn emit_packet(&self, event: &str) {
        let occupancy = self.occupancy_snapshot().await;
        let packet = PoolTelemetryPacket {
            timestamp_unix_ms: now_unix_ms(),
            event: event.to_string(),
            total_targets: occupancy.total_targets,
            ready_targets: occupancy.ready_targets,
            leased_targets: occupancy.leased_targets,
            tainted_targets: occupancy.tainted_targets,
            quarantined_targets: occupancy.quarantined_targets,
            active_leases: occupancy.active_leases,
            expired_lease_count: self.expired_lease_count.load(Ordering::Acquire),
        };

        let payload = serde_json::to_string(&packet)
            .unwrap_or_else(|error| format!("telemetry_error:{error}"));
        tracing::info!(target: "kria_pool", packet = %payload, "target_pool_telemetry");

        {
            let mut latest = self
                .latest_telemetry_packet
                .lock()
                .expect("target_pool latest telemetry lock poisoned");
            *latest = Some(packet.clone());
        }

        let callbacks = self
            .telemetry_callbacks
            .lock()
            .expect("target_pool telemetry callback lock poisoned")
            .clone();
        for callback in callbacks {
            callback(packet.clone());
        }
    }
}

fn weighted_score(metrics: TargetHealthTelemetry, weights: SelectionWeights) -> f64 {
    let health = metrics.health_score.clamp(0.0, 1.0);
    let failure_component = (1.0 - metrics.recent_failure_rate.clamp(0.0, 1.0)).clamp(0.0, 1.0);

    let latency_ms = metrics.latency_ewma_ms.max(1.0);
    let latency_component = 1.0 / (1.0 + latency_ms / 100.0);

    (weights.health * health)
        + (weights.latency * latency_component)
        + (weights.failure * failure_component)
}

fn map_qos_denial(admission: QosAdmission) -> Option<EnvironmentError> {
    match admission {
        QosAdmission::Accepted => None,
        QosAdmission::Deferred {
            retry_after,
            reason,
        } => Some(EnvironmentError::EnvironmentResetRequired {
            reason: format!(
                "target_pool deferred by qos for {}ms: {}",
                retry_after.as_millis(),
                reason
            ),
        }),
        QosAdmission::Rejected { reason } => Some(EnvironmentError::EnvironmentResetRequired {
            reason: format!("target_pool rejected by qos: {reason}"),
        }),
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

async fn mark_environment_tainted(environment: &Arc<QemuSshEnvironment>, reason: String) {
    environment.tainted.store(true, Ordering::Release);
    *environment.taint_reason.lock().await = Some(reason);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_selection_prefers_healthier_and_more_reliable_target() {
        let strong = TargetHealthTelemetry {
            health_score: 0.95,
            latency_ewma_ms: 35.0,
            recent_failure_rate: 0.01,
        };
        let weak = TargetHealthTelemetry {
            health_score: 0.65,
            latency_ewma_ms: 120.0,
            recent_failure_rate: 0.15,
        };

        let weights = SelectionWeights::default();
        assert!(weighted_score(strong, weights) > weighted_score(weak, weights));
    }

    #[test]
    fn lease_expiry_tainting_transitions_inventory_to_tainted_state() {
        let mut inventory = InventoryRegistry::default();
        let target_id = TargetId::new();
        inventory.insert_ready(target_id.clone());

        let lease_id = LeaseId::new();
        inventory
            .transition_ready_to_leased(&target_id, lease_id.clone(), Instant::now())
            .expect("transition ready->leased");

        inventory
            .transition_to_tainted(
                &target_id,
                format!(
                    "lease {} heartbeat expired; fail-closed taint applied",
                    lease_id.0
                ),
            )
            .expect("transition leased->tainted");

        assert!(matches!(
            inventory.state(&target_id),
            Some(InventoryState::Tainted { .. })
        ));
    }
}
