use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::image::orchestrator::{ImageError, ImageRequest, ImageResult};
use crate::image::swap::LlmEvictionController;
use crate::image::ws_bridge::EventEmitter;
use crate::resource::{
    GpuLeaseManager, GpuLeaseState, GpuOwner, ImageLeaseBackendId, L1Residency, ResourceSnapshot,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageBackendId {
    ComfyUi,
    CloudFallback,
    SdCpp,
    Other(String),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImageBackendCapabilities {
    pub supports_local_gpu: bool,
    pub supports_cloud: bool,
    pub supports_cancel: bool,
    pub supports_release: bool,
}

impl Default for ImageBackendCapabilities {
    fn default() -> Self {
        Self {
            supports_local_gpu: true,
            supports_cloud: false,
            supports_cancel: true,
            supports_release: true,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImageBackendHealth {
    pub healthy: bool,
    pub detail: String,
}

impl ImageBackendHealth {
    pub fn healthy(detail: impl Into<String>) -> Self {
        Self {
            healthy: true,
            detail: detail.into(),
        }
    }

    pub fn unhealthy(detail: impl Into<String>) -> Self {
        Self {
            healthy: false,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImageEstimate {
    pub backend: ImageBackendId,
    pub requires_gpu: bool,
    pub expected_seconds: Option<u32>,
    #[serde(default)]
    pub expected_vram_mb: Option<u64>,
    pub notes: Option<String>,
}

pub type ImageJobId = String;

#[derive(Clone, Default)]
pub struct ImageExecutionContext {
    pub emitter: Option<EventEmitter>,
    pub llm_evictor: Option<Arc<dyn LlmEvictionController>>,
    pub cancellation: Option<CancellationToken>,
}

#[async_trait]
pub trait ImageBackend: Send + Sync {
    fn id(&self) -> ImageBackendId;

    fn capabilities(&self) -> ImageBackendCapabilities;

    async fn health(&self) -> ImageBackendHealth;

    async fn estimate(&self, request: &ImageRequest) -> ImageEstimate;

    async fn generate(
        &self,
        request: ImageRequest,
        ctx: ImageExecutionContext,
    ) -> Result<ImageResult, ImageError>;

    async fn cancel(&self, job_id: ImageJobId) -> Result<(), ImageError>;

    async fn release(&self) -> Result<(), ImageError>;
}

/// Registry used by runtime wiring to keep local backends and cloud fallback
/// decoupled from tool-facing call sites.
pub struct ImageBackendRegistry {
    backends: RwLock<HashMap<ImageBackendId, Arc<dyn ImageBackend>>>,
    default_backend: RwLock<Option<ImageBackendId>>,
    cloud_fallback_backend: RwLock<Option<ImageBackendId>>,
    local_gpu_lease: RwLock<Option<Arc<GpuLeaseManager>>>,
    critical_free_vram_mb: RwLock<u64>,
}

impl ImageBackendRegistry {
    pub fn new() -> Self {
        Self {
            backends: RwLock::new(HashMap::new()),
            default_backend: RwLock::new(None),
            cloud_fallback_backend: RwLock::new(None),
            local_gpu_lease: RwLock::new(None),
            critical_free_vram_mb: RwLock::new(512),
        }
    }

    pub fn register(&self, backend: Arc<dyn ImageBackend>) {
        let id = backend.id();
        self.backends
            .write()
            .expect("image backend registry lock poisoned")
            .insert(id, backend);
    }

    pub fn set_default_backend(&self, id: ImageBackendId) {
        *self
            .default_backend
            .write()
            .expect("image backend default lock poisoned") = Some(id);
    }

    pub fn set_cloud_fallback_backend(&self, id: ImageBackendId) {
        *self
            .cloud_fallback_backend
            .write()
            .expect("image backend cloud lock poisoned") = Some(id);
    }

    pub fn set_gpu_lease_manager(&self, lease: Arc<GpuLeaseManager>) {
        *self
            .local_gpu_lease
            .write()
            .expect("image backend lease lock poisoned") = Some(lease);
    }

    pub fn clear_gpu_lease_manager(&self) {
        *self
            .local_gpu_lease
            .write()
            .expect("image backend lease lock poisoned") = None;
    }

    pub fn set_critical_free_vram_mb(&self, threshold_mb: u64) {
        *self
            .critical_free_vram_mb
            .write()
            .expect("image backend threshold lock poisoned") = threshold_mb.max(1);
    }

    pub fn get(&self, id: &ImageBackendId) -> Option<Arc<dyn ImageBackend>> {
        self.backends
            .read()
            .expect("image backend registry lock poisoned")
            .get(id)
            .cloned()
    }

    pub fn default_backend(&self) -> Option<Arc<dyn ImageBackend>> {
        let id = self
            .default_backend
            .read()
            .expect("image backend default lock poisoned")
            .clone()?;
        self.get(&id)
    }

    pub fn cloud_fallback_backend(&self) -> Option<Arc<dyn ImageBackend>> {
        let id = self
            .cloud_fallback_backend
            .read()
            .expect("image backend cloud lock poisoned")
            .clone()?;
        self.get(&id)
    }

    pub fn select_best(
        &self,
        request: &ImageRequest,
        telemetry: &ResourceSnapshot,
    ) -> Result<Arc<dyn ImageBackend>, ImageError> {
        if request.force_cloud {
            return self.select_cloud_or_err("request explicitly forced cloud backend");
        }

        let comfy = self.get(&ImageBackendId::ComfyUi);
        let cloud = self
            .get(&ImageBackendId::CloudFallback)
            .or_else(|| self.cloud_fallback_backend());

        let l1_not_evictable = matches!(
            telemetry.l1.residency,
            L1Residency::GpuHot | L1Residency::ReloadingGpu | L1Residency::Starting
        );
        let critical_free_vram_mb = *self
            .critical_free_vram_mb
            .read()
            .expect("image backend threshold lock poisoned");
        let heavily_fragmented = telemetry.vram.free_mb <= critical_free_vram_mb && l1_not_evictable;

        let lease_ready = self.local_gpu_lease_available_for_comfy();
        let comfy_healthy = comfy
            .as_ref()
            .map(|backend| Self::probe_health(backend).healthy)
            .unwrap_or(false);

        if let Some(comfy_backend) = comfy {
            if lease_ready && comfy_healthy && !heavily_fragmented {
                return Ok(comfy_backend);
            }
        }

        if let Some(cloud_backend) = cloud {
            return Ok(cloud_backend);
        }

        if let Some(default_backend) = self.default_backend() {
            return Ok(default_backend);
        }

        Err(ImageError::OutputDir(
            "no image backend available for current resource state".to_string(),
        ))
    }

    fn select_cloud_or_err(&self, reason: &str) -> Result<Arc<dyn ImageBackend>, ImageError> {
        if let Some(cloud_backend) = self
            .get(&ImageBackendId::CloudFallback)
            .or_else(|| self.cloud_fallback_backend())
        {
            return Ok(cloud_backend);
        }

        Err(ImageError::OutputDir(format!(
            "cloud fallback backend is not registered ({reason})"
        )))
    }

    fn local_gpu_lease_available_for_comfy(&self) -> bool {
        let lease = self
            .local_gpu_lease
            .read()
            .expect("image backend lease lock poisoned")
            .as_ref()
            .map(Arc::clone);

        let Some(lease) = lease else {
            // Registry can still make progress in cloud-only mode even when no lease manager
            // is wired yet, so this defaults to available.
            return true;
        };

        match lease.state() {
            GpuLeaseState::Idle => true,
            GpuLeaseState::Held { owner, .. } => matches!(
                owner,
                GpuOwner::ImageBackend(ImageLeaseBackendId::ComfyUi) | GpuOwner::Maintenance
            ),
            GpuLeaseState::Recovering { .. } | GpuLeaseState::Degraded { .. } => false,
        }
    }

    fn probe_health(backend: &Arc<dyn ImageBackend>) -> ImageBackendHealth {
        futures::executor::block_on(backend.health())
    }
}

impl Default for ImageBackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}
