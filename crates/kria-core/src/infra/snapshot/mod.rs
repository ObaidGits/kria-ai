use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::KriaSystemConfig;
use crate::infra::environment::remote_qemu::QemuSshEnvironment;
use crate::infra::environment::EnvironmentError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SnapshotId(pub Uuid);

impl SnapshotId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub snapshot_id: SnapshotId,
    pub target_instance_id: String,
    pub created_unix_ms: u64,
    pub toolchain_fingerprint: String,
    pub digest_sha256: String,
    pub baseline_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapshotDriftTolerance {
    pub max_normalized_hash_distance: f64,
}

impl SnapshotDriftTolerance {
    pub fn from_system_config(system_config: &KriaSystemConfig) -> Self {
        Self {
            max_normalized_hash_distance: system_config.snapshot.max_normalized_hash_distance,
        }
    }
}

impl Default for SnapshotDriftTolerance {
    fn default() -> Self {
        Self::from_system_config(&KriaSystemConfig::default())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotCreateRequest {
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotRestoreRequest {
    pub snapshot_id: SnapshotId,
    pub drift_tolerance: SnapshotDriftTolerance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotIntegrityReport {
    pub snapshot_id: SnapshotId,
    pub digest_match: bool,
    pub expected_digest_sha256: String,
    pub computed_digest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotRestoreReport {
    pub snapshot_id: SnapshotId,
    pub restore_latency_ms: u64,
    pub drift_distance: f64,
    pub digest_match: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotTelemetryPacket {
    pub timestamp_unix_ms: u64,
    pub event: String,
    pub snapshot_id: Option<String>,
    pub target_instance_id: String,
    pub digest_match: Option<bool>,
    pub restore_latency_ms: Option<u64>,
    pub drift_distance: Option<f64>,
    pub hard_reset_fallback: bool,
    pub details: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotPayload {
    snapshot_id: SnapshotId,
    target_instance_id: String,
    generation: u64,
    epoch_uuid: Uuid,
    transport_generation_id: u64,
    toolchain_fingerprint: String,
    baseline_fingerprint: String,
    created_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LatestSnapshotPointer {
    snapshot_id: SnapshotId,
}

#[async_trait]
pub trait VmSnapshotProvider: Send + Sync {
    async fn create_snapshot(
        &self,
        request: SnapshotCreateRequest,
    ) -> Result<SnapshotMetadata, EnvironmentError>;

    async fn verify_integrity(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<SnapshotIntegrityReport, EnvironmentError>;

    async fn restore_snapshot(
        &self,
        request: SnapshotRestoreRequest,
    ) -> Result<SnapshotRestoreReport, EnvironmentError>;
}

#[async_trait]
impl VmSnapshotProvider for QemuSshEnvironment {
    async fn create_snapshot(
        &self,
        request: SnapshotCreateRequest,
    ) -> Result<SnapshotMetadata, EnvironmentError> {
        let snapshot_id = SnapshotId::new();
        let created_unix_ms = now_unix_ms();
        let baseline_fingerprint = runtime_fingerprint(self).await?;

        let payload = SnapshotPayload {
            snapshot_id: snapshot_id.clone(),
            target_instance_id: self.config.instance_id.clone(),
            generation: self.generation.load(Ordering::Acquire),
            epoch_uuid: *self.epoch_uuid.load_full().as_ref(),
            transport_generation_id: self.transport_generation_id.load(Ordering::Acquire),
            toolchain_fingerprint: self.config.host_artifact_gc.host_binary_sha256_or_build_id.clone(),
            baseline_fingerprint: baseline_fingerprint.clone(),
            created_unix_ms,
        };

        let payload_bytes = serde_json::to_vec(&payload).map_err(|error| EnvironmentError::Serialization {
            details: format!("serialize snapshot payload failed: {error}"),
        })?;
        let digest_sha256 = sha256_hex(&payload_bytes);

        let metadata = SnapshotMetadata {
            snapshot_id: snapshot_id.clone(),
            target_instance_id: self.config.instance_id.clone(),
            created_unix_ms,
            toolchain_fingerprint: self.config.host_artifact_gc.host_binary_sha256_or_build_id.clone(),
            digest_sha256: digest_sha256.clone(),
            baseline_fingerprint,
        };

        persist_snapshot(self, &metadata, &payload_bytes)?;
        write_latest_snapshot_pointer(self, &snapshot_id)?;

        emit_snapshot_packet(SnapshotTelemetryPacket {
            timestamp_unix_ms: now_unix_ms(),
            event: "snapshot_created".to_string(),
            snapshot_id: Some(snapshot_id.0.to_string()),
            target_instance_id: self.config.instance_id.clone(),
            digest_match: Some(true),
            restore_latency_ms: None,
            drift_distance: None,
            hard_reset_fallback: false,
            details: format!("label={} digest={}", request.label, digest_sha256),
        });

        Ok(metadata)
    }

    async fn verify_integrity(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<SnapshotIntegrityReport, EnvironmentError> {
        let metadata = read_snapshot_metadata(self, snapshot_id)?;
        let payload = read_snapshot_payload_bytes(self, snapshot_id)?;
        let computed = sha256_hex(&payload);

        Ok(SnapshotIntegrityReport {
            snapshot_id: snapshot_id.clone(),
            digest_match: computed == metadata.digest_sha256,
            expected_digest_sha256: metadata.digest_sha256,
            computed_digest_sha256: computed,
        })
    }

    async fn restore_snapshot(
        &self,
        request: SnapshotRestoreRequest,
    ) -> Result<SnapshotRestoreReport, EnvironmentError> {
        let started = Instant::now();

        let integrity = self.verify_integrity(&request.snapshot_id).await?;
        if !integrity.digest_match {
            self.tainted.store(true, Ordering::Release);
            *self.taint_reason.lock().await = Some(format!(
                "snapshot integrity mismatch for {} (expected={}, computed={})",
                request.snapshot_id.0, integrity.expected_digest_sha256, integrity.computed_digest_sha256
            ));

            emit_snapshot_packet(SnapshotTelemetryPacket {
                timestamp_unix_ms: now_unix_ms(),
                event: "snapshot_restore_integrity_failed".to_string(),
                snapshot_id: Some(request.snapshot_id.0.to_string()),
                target_instance_id: self.config.instance_id.clone(),
                digest_match: Some(false),
                restore_latency_ms: Some(started.elapsed().as_millis() as u64),
                drift_distance: None,
                hard_reset_fallback: true,
                details: "digest mismatch; fail-closed taint asserted".to_string(),
            });

            return Err(EnvironmentError::EnvironmentResetFailed {
                reason: "snapshot_integrity_mismatch".to_string(),
                details: format!(
                    "expected={} computed={}",
                    integrity.expected_digest_sha256, integrity.computed_digest_sha256
                ),
            });
        }

        let payload = read_snapshot_payload(self, &request.snapshot_id)?;

        if payload.toolchain_fingerprint != self.config.host_artifact_gc.host_binary_sha256_or_build_id {
            self.tainted.store(true, Ordering::Release);
            *self.taint_reason.lock().await = Some(format!(
                "snapshot toolchain fingerprint mismatch for {}",
                request.snapshot_id.0
            ));
            return Err(EnvironmentError::EnvironmentResetFailed {
                reason: "snapshot_toolchain_fingerprint_mismatch".to_string(),
                details: format!(
                    "snapshot={} active={}",
                    payload.toolchain_fingerprint,
                    self.config.host_artifact_gc.host_binary_sha256_or_build_id
                ),
            });
        }

        if let Err(error) = self
            .restore_snapshot_via_qmp(&request.snapshot_id.0.to_string())
            .await
        {
            self.tainted.store(true, Ordering::Release);
            *self.taint_reason.lock().await = Some(format!(
                "snapshot qmp restore failed for {}",
                request.snapshot_id.0
            ));
            return Err(error);
        }

        self.generation.store(payload.generation, Ordering::Release);
        self.epoch_uuid.store(Arc::new(payload.epoch_uuid));
        self.transport_generation_id
            .store(payload.transport_generation_id, Ordering::Release);
        self.admission_inflight.store(0, Ordering::Release);
        self.admissions_frozen.store(false, Ordering::Release);
        self.inflight_registry.write().await.clear();
        self.staged_artifact_index.write().await.clear();
        self.helper_seen_initializations.write().await.clear();
        self.zombie_commands.write().await.clear();
        {
            let mut replay = self.nonce_replay_cache.write().await;
            replay.rotate_to_epoch(payload.epoch_uuid);
        }

        let post_fingerprint = runtime_fingerprint(self).await?;
        let drift_distance = normalized_hash_distance(&payload.baseline_fingerprint, &post_fingerprint);

        if drift_distance > request.drift_tolerance.max_normalized_hash_distance {
            self.tainted.store(true, Ordering::Release);
            *self.taint_reason.lock().await = Some(format!(
                "snapshot drift distance {} exceeded tolerance {}",
                drift_distance, request.drift_tolerance.max_normalized_hash_distance
            ));

            emit_snapshot_packet(SnapshotTelemetryPacket {
                timestamp_unix_ms: now_unix_ms(),
                event: "snapshot_restore_drift_failed".to_string(),
                snapshot_id: Some(request.snapshot_id.0.to_string()),
                target_instance_id: self.config.instance_id.clone(),
                digest_match: Some(true),
                restore_latency_ms: Some(started.elapsed().as_millis() as u64),
                drift_distance: Some(drift_distance),
                hard_reset_fallback: true,
                details: "post-restore drift tolerance exceeded".to_string(),
            });

            return Err(EnvironmentError::EnvironmentResetFailed {
                reason: "snapshot_post_restore_drift".to_string(),
                details: format!(
                    "drift={} tolerance={}",
                    drift_distance, request.drift_tolerance.max_normalized_hash_distance
                ),
            });
        }

        self.tainted.store(false, Ordering::Release);
        self.taint_reason.lock().await.take();

        let report = SnapshotRestoreReport {
            snapshot_id: request.snapshot_id,
            restore_latency_ms: started.elapsed().as_millis() as u64,
            drift_distance,
            digest_match: true,
        };

        emit_snapshot_packet(SnapshotTelemetryPacket {
            timestamp_unix_ms: now_unix_ms(),
            event: "snapshot_restore_succeeded".to_string(),
            snapshot_id: Some(report.snapshot_id.0.to_string()),
            target_instance_id: self.config.instance_id.clone(),
            digest_match: Some(true),
            restore_latency_ms: Some(report.restore_latency_ms),
            drift_distance: Some(report.drift_distance),
            hard_reset_fallback: false,
            details: "fast-path restore completed".to_string(),
        });

        Ok(report)
    }
}

pub async fn ensure_baseline_snapshot(provider: &QemuSshEnvironment) -> Result<(), EnvironmentError> {
    if read_latest_snapshot_pointer(provider)?.is_some() {
        return Ok(());
    }

    let _ = provider
        .create_snapshot(SnapshotCreateRequest {
            label: "ensure_ready_baseline".to_string(),
        })
        .await?;
    Ok(())
}

pub async fn try_fast_restore_latest_snapshot(
    provider: &QemuSshEnvironment,
    drift_tolerance: SnapshotDriftTolerance,
) -> Result<Option<SnapshotRestoreReport>, EnvironmentError> {
    let Some(snapshot_id) = read_latest_snapshot_pointer(provider)? else {
        emit_snapshot_packet(SnapshotTelemetryPacket {
            timestamp_unix_ms: now_unix_ms(),
            event: "snapshot_restore_skipped".to_string(),
            snapshot_id: None,
            target_instance_id: provider.config.instance_id.clone(),
            digest_match: None,
            restore_latency_ms: None,
            drift_distance: None,
            hard_reset_fallback: false,
            details: "no latest snapshot pointer found".to_string(),
        });
        return Ok(None);
    };

    let restore = provider
        .restore_snapshot(SnapshotRestoreRequest {
            snapshot_id,
            drift_tolerance,
        })
        .await?;
    Ok(Some(restore))
}

async fn runtime_fingerprint(provider: &QemuSshEnvironment) -> Result<String, EnvironmentError> {
    let inflight_registry_len = provider.inflight_registry.read().await.len();
    let staged_len = provider.staged_artifact_index.read().await.len();
    let helper_seen_len = provider.helper_seen_initializations.read().await.len();
    let zombie_len = provider.zombie_commands.read().await.len();

    let fingerprint_json = serde_json::json!({
        "instance_id": provider.config.instance_id.clone(),
        "generation": provider.generation.load(Ordering::Acquire),
        "epoch_uuid": provider.epoch_uuid.load_full().as_ref().to_string(),
        "transport_generation_id": provider.transport_generation_id.load(Ordering::Acquire),
        "tainted": provider.tainted.load(Ordering::Acquire),
        "admissions_frozen": provider.admissions_frozen.load(Ordering::Acquire),
        "admission_inflight": provider.admission_inflight.load(Ordering::Acquire),
        "inflight_registry_len": inflight_registry_len,
        "staged_artifact_index_len": staged_len,
        "helper_seen_len": helper_seen_len,
        "zombie_commands_len": zombie_len,
        "toolchain_fingerprint": provider
            .config
            .host_artifact_gc
            .host_binary_sha256_or_build_id
            .clone(),
    });

    let bytes = serde_json::to_vec(&fingerprint_json).map_err(|error| EnvironmentError::Serialization {
        details: format!("serialize runtime fingerprint failed: {error}"),
    })?;

    Ok(sha256_hex(&bytes))
}

fn snapshot_root(provider: &QemuSshEnvironment) -> PathBuf {
    provider.config.remote_control_dir.join("vm_snapshots")
}

fn snapshot_metadata_path(provider: &QemuSshEnvironment, snapshot_id: &SnapshotId) -> PathBuf {
    snapshot_root(provider).join(format!("{}.meta.json", snapshot_id.0))
}

fn snapshot_payload_path(provider: &QemuSshEnvironment, snapshot_id: &SnapshotId) -> PathBuf {
    snapshot_root(provider).join(format!("{}.payload.json", snapshot_id.0))
}

fn latest_snapshot_pointer_path(provider: &QemuSshEnvironment) -> PathBuf {
    snapshot_root(provider).join("latest.json")
}

fn persist_snapshot(
    provider: &QemuSshEnvironment,
    metadata: &SnapshotMetadata,
    payload_bytes: &[u8],
) -> Result<(), EnvironmentError> {
    let root = snapshot_root(provider);
    std::fs::create_dir_all(&root).map_err(|error| EnvironmentError::Io {
        operation: "snapshot::create_dir_all".to_string(),
        details: format!("{} ({})", error, root.display()),
    })?;

    let metadata_path = snapshot_metadata_path(provider, &metadata.snapshot_id);
    let payload_path = snapshot_payload_path(provider, &metadata.snapshot_id);

    let metadata_json = serde_json::to_vec_pretty(metadata).map_err(|error| EnvironmentError::Serialization {
        details: format!("serialize snapshot metadata failed: {error}"),
    })?;

    std::fs::write(&metadata_path, metadata_json).map_err(|error| EnvironmentError::Io {
        operation: "snapshot::write_metadata".to_string(),
        details: format!("{} ({})", error, metadata_path.display()),
    })?;

    std::fs::write(&payload_path, payload_bytes).map_err(|error| EnvironmentError::Io {
        operation: "snapshot::write_payload".to_string(),
        details: format!("{} ({})", error, payload_path.display()),
    })?;

    Ok(())
}

fn write_latest_snapshot_pointer(
    provider: &QemuSshEnvironment,
    snapshot_id: &SnapshotId,
) -> Result<(), EnvironmentError> {
    let pointer = LatestSnapshotPointer {
        snapshot_id: snapshot_id.clone(),
    };
    let pointer_json = serde_json::to_vec_pretty(&pointer).map_err(|error| EnvironmentError::Serialization {
        details: format!("serialize latest snapshot pointer failed: {error}"),
    })?;

    let path = latest_snapshot_pointer_path(provider);
    std::fs::write(&path, pointer_json).map_err(|error| EnvironmentError::Io {
        operation: "snapshot::write_latest_pointer".to_string(),
        details: format!("{} ({})", error, path.display()),
    })?;

    Ok(())
}

fn read_latest_snapshot_pointer(
    provider: &QemuSshEnvironment,
) -> Result<Option<SnapshotId>, EnvironmentError> {
    let path = latest_snapshot_pointer_path(provider);
    if !path.exists() {
        return Ok(None);
    }

    let bytes = std::fs::read(&path).map_err(|error| EnvironmentError::Io {
        operation: "snapshot::read_latest_pointer".to_string(),
        details: format!("{} ({})", error, path.display()),
    })?;

    let pointer: LatestSnapshotPointer =
        serde_json::from_slice(&bytes).map_err(|error| EnvironmentError::Serialization {
            details: format!("deserialize latest snapshot pointer failed: {error}"),
        })?;

    Ok(Some(pointer.snapshot_id))
}

fn read_snapshot_metadata(
    provider: &QemuSshEnvironment,
    snapshot_id: &SnapshotId,
) -> Result<SnapshotMetadata, EnvironmentError> {
    let path = snapshot_metadata_path(provider, snapshot_id);
    let bytes = std::fs::read(&path).map_err(|error| EnvironmentError::Io {
        operation: "snapshot::read_metadata".to_string(),
        details: format!("{} ({})", error, path.display()),
    })?;

    serde_json::from_slice::<SnapshotMetadata>(&bytes).map_err(|error| EnvironmentError::Serialization {
        details: format!("deserialize snapshot metadata failed: {error}"),
    })
}

fn read_snapshot_payload_bytes(
    provider: &QemuSshEnvironment,
    snapshot_id: &SnapshotId,
) -> Result<Vec<u8>, EnvironmentError> {
    let path = snapshot_payload_path(provider, snapshot_id);
    std::fs::read(&path).map_err(|error| EnvironmentError::Io {
        operation: "snapshot::read_payload".to_string(),
        details: format!("{} ({})", error, path.display()),
    })
}

fn read_snapshot_payload(
    provider: &QemuSshEnvironment,
    snapshot_id: &SnapshotId,
) -> Result<SnapshotPayload, EnvironmentError> {
    let bytes = read_snapshot_payload_bytes(provider, snapshot_id)?;
    serde_json::from_slice::<SnapshotPayload>(&bytes).map_err(|error| EnvironmentError::Serialization {
        details: format!("deserialize snapshot payload failed: {error}"),
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn normalized_hash_distance(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 1.0;
    }

    let len = a.len().min(b.len());
    if len == 0 {
        return 1.0;
    }

    let mismatches = a
        .as_bytes()
        .iter()
        .zip(b.as_bytes().iter())
        .take(len)
        .filter(|(left, right)| left != right)
        .count();

    mismatches as f64 / len as f64
}

fn emit_snapshot_packet(packet: SnapshotTelemetryPacket) {
    let payload = serde_json::to_string(&packet)
        .unwrap_or_else(|error| format!("telemetry_error:{error}"));
    tracing::info!(target: "kria_snapshot", packet = %payload, "snapshot_telemetry");
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_hash_distance_detects_divergence() {
        let a = "aaaaaaaa";
        let b = "aaaabbbb";
        let distance = normalized_hash_distance(a, b);
        assert!(distance > 0.0);
        assert!(distance <= 1.0);
    }

    #[test]
    fn normalized_hash_distance_zero_for_equal_hashes() {
        let hash = "abcdef0123456789";
        assert_eq!(normalized_hash_distance(hash, hash), 0.0);
    }
}
