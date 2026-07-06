//! DEPRECATED: Use runtime_manager.rs for new implementations.
//!
//! This module provides compatibility layer for existing code.
//! New code should use RuntimeManager directly.

use super::config::OpenClawConfig;
use super::runtime_manager::{Priority, RuntimeManager, WarmPoolConfig};
use super::types::ResourceClass;
use bollard::container::Config;
use std::sync::Arc;

// Re-export types for compatibility
pub use super::runtime_manager::{ContainerHandle, RuntimeError as PoolError};

/// Compatibility layer - delegates to RuntimeManager.
pub struct ContainerPool {
    runtime_manager: RuntimeManager,
}

impl ContainerPool {
    /// Create new container pool - delegates to RuntimeManager.
    ///
    /// Wires the warm-pool sizing from the user's `OpenClawConfig` and eagerly
    /// runs `RuntimeManager::initialize()` so the substrate is ACTUALLY ready
    /// (warm containers created + health/recycle background tasks started) after
    /// `new()` returns. Previously both `new()` and `initialize()` skipped this,
    /// leaving the pool permanently empty (`warm=0`) — the root cause of
    /// "container does not always start correctly".
    pub async fn new(config: OpenClawConfig) -> Result<Self, PoolError> {
        let mut warm_config = WarmPoolConfig::default();
        // Respect the user's configured warm-pool sizing instead of ignoring it.
        warm_config.minimum_containers = config.warm_per_class.clamp(1, 8);
        warm_config.warm_reserve = config.warm_per_class.clamp(1, 8);
        warm_config.maximum_containers = config
            .max_concurrent_invocations
            .max(config.warm_per_class)
            .clamp(1, 32);
        warm_config.max_idle_duration =
            std::time::Duration::from_secs(config.max_warm_age_secs.max(30));

        let mut runtime_manager = RuntimeManager::new(config, warm_config).await?;
        runtime_manager.initialize().await?;

        Ok(Self { runtime_manager })
    }

    /// Initialize pool. The warm pool + background tasks are already started in
    /// `new()`; this is a no-op kept for boot-flow compatibility.
    pub async fn initialize(&self) -> Result<(), PoolError> {
        Ok(())
    }

    /// Drain and re-warm the pool (UI "Restart Substrate"). Keeps background
    /// health/recycle tasks alive.
    pub async fn rewarm(&self) -> Result<(), PoolError> {
        self.runtime_manager.rewarm().await
    }

    /// Checkout container - delegates with default priority.
    pub async fn checkout(
        &self,
        resource_class: ResourceClass,
        skill_id: &str,
    ) -> Result<ContainerHandle, PoolError> {
        self.runtime_manager
            .checkout_container(resource_class, skill_id, Priority::Background)
            .await
    }

    /// Return container after use - delegates to RuntimeManager.
    pub async fn checkin(&self, handle: ContainerHandle) -> Result<(), PoolError> {
        self.runtime_manager.checkin_container(handle).await
    }

    /// Spawn pre-warm loop - compatibility method.
    pub fn spawn_prewarm_loop(_pool: Arc<ContainerPool>) {
        // RuntimeManager handles warming internally
        tracing::info!("Pre-warm loop handled by RuntimeManager internally");
    }

    /// Verify image available - compatibility method.
    pub async fn verify_image_available(&self) -> Result<(), PoolError> {
        // RuntimeManager handles this in create_container
        Ok(())
    }

    /// Access docker client - compatibility method.
    pub fn docker(&self) -> &bollard::Docker {
        &self.runtime_manager.docker
    }

    /// Get image name - compatibility method.
    pub fn image(&self) -> &str {
        &self.runtime_manager.config.image
    }

    /// Get active container count - compatibility method.
    pub async fn active_count(&self) -> usize {
        // Use public metrics method instead of accessing private fields
        self.runtime_manager
            .get_runtime_metrics()
            .await
            .active_runtimes as usize
    }

    /// Get warm container count - compatibility method.
    pub async fn warm_count_total(&self) -> usize {
        // Use public metrics method instead of accessing private fields
        self.runtime_manager
            .get_runtime_metrics()
            .await
            .ready_containers as usize
    }

    /// Shutdown pool — destroys every container so nothing leaks on app exit.
    pub async fn shutdown(&self) -> Result<(), PoolError> {
        self.runtime_manager.shutdown().await;
        Ok(())
    }

    /// Create a real bespoke materialized container FROM the provided config.
    ///
    /// Previously this discarded the config and did a generic `checkout`,
    /// which meant A3 materialization (capability mounts/network) and the
    /// bundle-execution skill mount NEVER reached a real container. Now it
    /// creates a genuine one-off container applying the materialized config
    /// (image + idle cmd + resource limits + security opts + binds), tracked
    /// for leak detection + destroyed by the caller after execution.
    pub async fn create_materialized(
        &self,
        config: Config<String>,
        resource_class: ResourceClass,
    ) -> Result<ContainerHandle, PoolError> {
        self.runtime_manager
            .create_bespoke_container(config, resource_class)
            .await
    }

    /// Destroy container - compatibility method.
    pub async fn destroy(&self, container_id: &str) -> Result<(), PoolError> {
        self.runtime_manager.destroy_container(container_id).await
    }
}
