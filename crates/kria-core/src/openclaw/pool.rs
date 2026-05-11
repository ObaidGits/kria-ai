//! Container warm pool with per-invocation ephemeral isolation.
//!
//! Maintains pre-started Docker containers per `ResourceClass` (Light/Medium/Heavy).
//! Each tool invocation checks out a container, uses it, and destroys it.
//! No state persists between invocations — preventing cross-skill workspace poisoning.
//!
//! # Lifecycle
//!
//! 1. On startup: pre-warm `config.warm_per_class` containers per resource class.
//! 2. On `checkout()`: pop a warm container (or create one if pool empty).
//! 3. On `checkin()`: destroy the container, pre-warm a replacement.
//!
//! # Thread Safety
//!
//! The pool is `Send + Sync` and can be shared across async tasks via `Arc`.

use super::config::OpenClawConfig;
use super::types::ResourceClass;
use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, InspectContainerOptions,
    RemoveContainerOptions, StartContainerOptions,
};
use bollard::models::HostConfig;
use bollard::Docker;
use futures::StreamExt;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Mutex, OwnedSemaphorePermit, Semaphore};

/// Handle to a checked-out container.
#[derive(Debug, Clone)]
pub struct ContainerHandle {
    pub invocation_id: String,
    pub container_id: String,
    pub workspace_path: String,
    pub resource_class: ResourceClass,
}

/// Errors from the container pool.
#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    #[error("pool exhausted: no warm containers available and creation failed: {0}")]
    Exhausted(String),
    #[error("max concurrent invocations reached ({0})")]
    MaxConcurrent(usize),
    #[error("container creation failed: {0}")]
    CreationFailed(String),
    #[error("container start failed: {0}")]
    StartFailed(String),
    #[error("docker API error: {0}")]
    Docker(#[from] bollard::errors::Error),
    #[error("pool not initialized")]
    NotInitialized,
}

/// A warm container waiting in the pool.
#[derive(Debug)]
#[allow(dead_code)]
struct WarmContainer {
    container_id: String,
    created_at: Instant,
    resource_class: ResourceClass,
}

/// Container warm pool.
pub struct ContainerPool {
    docker: Docker,
    config: OpenClawConfig,
    /// Warm containers per resource class.
    pools: Arc<Mutex<HashMap<ResourceClass, VecDeque<WarmContainer>>>>,
    /// Active invocations tracking.
    active: Arc<Mutex<HashMap<String, ActiveInvocation>>>,
    /// Concurrency limiter.
    semaphore: Arc<Semaphore>,
    /// Monotonic generation counter — incremented on every checkout/recycle
    /// to prevent duplicate recycle attempts under concurrent crash events.
    generation: AtomicU64,
    /// Shutdown signal.
    shutdown: broadcast::Sender<()>,
}

#[derive(Debug)]
#[allow(dead_code)]
struct ActiveInvocation {
    invocation_id: String,
    container_id: String,
    skill_id: String,
    started_at: Instant,
    /// Semaphore permit held for the lifetime of this invocation.
    _permit: OwnedSemaphorePermit,
}

impl ContainerPool {
    /// Create a new container pool.
    pub async fn new(config: OpenClawConfig) -> Result<Self, PoolError> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| PoolError::CreationFailed(format!("Docker connection failed: {}", e)))?;

        // Verify Docker is accessible
        docker.ping().await.map_err(|e| {
            PoolError::CreationFailed(format!("Docker ping failed: {}", e))
        })?;

        let (shutdown, _) = broadcast::channel(1);
        let max_concurrent = config.max_concurrent_invocations;

        Ok(Self {
            docker,
            config,
            pools: Arc::new(Mutex::new(HashMap::new())),
            active: Arc::new(Mutex::new(HashMap::new())),
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            generation: AtomicU64::new(0),
            shutdown,
        })
    }

    /// Pre-warm containers for all resource classes.
    pub async fn initialize(&self) -> Result<(), PoolError> {
        for class in &[ResourceClass::Light, ResourceClass::Medium, ResourceClass::Heavy] {
            for _ in 0..self.config.warm_per_class {
                let container = self.create_container(*class).await?;
                self.pools
                    .lock()
                    .await
                    .entry(*class)
                    .or_default()
                    .push_back(container);
            }
            tracing::info!(
                resource_class = %class,
                count = self.config.warm_per_class,
                "Pre-warmed OpenClaw containers"
            );
        }
        Ok(())
    }

    /// Checkout a container for a tool invocation.
    ///
    /// 1. Acquire a concurrency permit.
    /// 2. Get a warm container from the pool (or create one).
    /// 3. Create an ephemeral workspace directory.
    /// 4. Return a handle.
    pub async fn checkout(
        &self,
        resource_class: ResourceClass,
        skill_id: &str,
    ) -> Result<ContainerHandle, PoolError> {
        // Acquire concurrency permit — released automatically when
        // ActiveInvocation is dropped in checkin().
        let permit = self.semaphore.clone().try_acquire_owned().map_err(|_| {
            PoolError::MaxConcurrent(self.config.max_concurrent_invocations)
        })?;

        // Bump generation counter (CAS guard for concurrent recycle races).
        let _gen = self.generation.fetch_add(1, Ordering::AcqRel);

        // Get or create a warm container.
        let warm = self.get_or_create_warm(resource_class).await?;

        // Unique per-invocation workspace path inside the container's tmpfs.
        let invocation_id = uuid::Uuid::new_v4().to_string();
        let workspace_path = format!("/workspace/{}", invocation_id);

        // mkdir inside the container's tmpfs (already mounted at /workspace).
        self.exec_in_container(
            &warm.container_id,
            &["mkdir", "-p", &workspace_path],
        )
        .await?;

        let handle = ContainerHandle {
            invocation_id: invocation_id.clone(),
            container_id: warm.container_id.clone(),
            workspace_path,
            resource_class,
        };

        // Track active invocation — permit is stored here and released on drop.
        self.active.lock().await.insert(
            invocation_id.clone(),
            ActiveInvocation {
                invocation_id,
                container_id: warm.container_id.clone(),
                skill_id: skill_id.to_string(),
                started_at: Instant::now(),
                _permit: permit,
            },
        );

        Ok(handle)
    }

    /// Return a container after invocation. Destroys it immediately.
    pub async fn checkin(&self, handle: ContainerHandle) -> Result<(), PoolError> {
        // Remove from active tracking
        self.active.lock().await.remove(&handle.invocation_id);

        // Destroy the container (not just stop — remove entirely)
        let _ = self
            .docker
            .remove_container(
                &handle.container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        // Pre-warm a replacement (async, non-blocking)
        let pools = self.pools.clone();
        let docker = self.docker.clone();
        let config = self.config.clone();
        let class = handle.resource_class;
        tokio::spawn(async move {
            match create_container_static(&docker, &config, class).await {
                Ok(container) => {
                    pools.lock().await.entry(class).or_default().push_back(container);
                }
                Err(e) => {
                    tracing::warn!(error = %e, resource_class = %class, "Failed to pre-warm replacement container");
                }
            }
        });

        Ok(())
    }

    /// Get a warm container or create one if pool is empty.
    async fn get_or_create_warm(&self, class: ResourceClass) -> Result<WarmContainer, PoolError> {
        // Try the pool first
        {
            let mut pools = self.pools.lock().await;
            if let Some(pool) = pools.get_mut(&class) {
                while let Some(container) = pool.pop_front() {
                    // Verify it's still healthy
                    if self.is_container_healthy(&container.container_id).await {
                        return Ok(container);
                    }
                    // Stale — destroy it
                    let _ = self
                        .docker
                        .remove_container(
                            &container.container_id,
                            Some(RemoveContainerOptions {
                                force: true,
                                ..Default::default()
                            }),
                        )
                        .await;
                }
            }
        }

        // Pool empty — create a fresh container
        self.create_container(class).await
    }

    /// Create a new container for a resource class.
    async fn create_container(&self, class: ResourceClass) -> Result<WarmContainer, PoolError> {
        create_container_static(&self.docker, &self.config, class).await
    }

    /// Check if a container is still running and healthy.
    async fn is_container_healthy(&self, container_id: &str) -> bool {
        match self
            .docker
            .inspect_container(container_id, None::<InspectContainerOptions>)
            .await
        {
            Ok(info) => info
                .state
                .as_ref()
                .and_then(|s| s.running)
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Execute a command inside a running container.
    async fn exec_in_container(
        &self,
        container_id: &str,
        cmd: &[&str],
    ) -> Result<(), PoolError> {
        use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};

        let exec = self
            .docker
            .create_exec(
                container_id,
                CreateExecOptions {
                    cmd: Some(cmd.to_vec()),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    ..Default::default()
                },
            )
            .await?;

        match self
            .docker
            .start_exec(&exec.id, Some(StartExecOptions::default()))
            .await?
        {
            StartExecResults::Attached { mut output, .. } => {
                while let Some(Ok(_)) = output.next().await {}
            }
            _ => {}
        }

        Ok(())
    }

    /// Shutdown the pool — destroy all containers.
    pub async fn shutdown(&self) -> Result<(), PoolError> {
        let _ = self.shutdown.send(());

        // Destroy all warm containers
        let mut pools = self.pools.lock().await;
        for (class, pool) in pools.drain() {
            for container in pool {
                let _ = self
                    .docker
                    .remove_container(
                        &container.container_id,
                        Some(RemoveContainerOptions {
                            force: true,
                            ..Default::default()
                        }),
                    )
                    .await;
            }
            tracing::info!(resource_class = %class, "Destroyed warm containers");
        }

        // Force-kill any active invocations
        let active = self.active.lock().await;
        for (id, inv) in active.iter() {
            tracing::warn!(
                invocation_id = id,
                skill_id = %inv.skill_id,
                "Force-killing active invocation during shutdown"
            );
            let _ = self
                .docker
                .remove_container(
                    &inv.container_id,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await;
        }

        Ok(())
    }

    /// Get the number of active invocations.
    pub async fn active_count(&self) -> usize {
        self.active.lock().await.len()
    }

    /// Get the total warm container count across all resource classes.
    pub async fn warm_count_total(&self) -> usize {
        self.pools.lock().await.values().map(|q| q.len()).sum()
    }

    /// Get the number of warm containers per class.
    pub async fn warm_counts(&self) -> HashMap<ResourceClass, usize> {
        let pools = self.pools.lock().await;
        pools
            .iter()
            .map(|(class, pool)| (*class, pool.len()))
            .collect()
    }

    /// Spawn a background task that keeps the Light pool pre-warmed to
    /// `config.warm_per_class` containers. Runs until the shutdown signal fires.
    pub fn spawn_prewarm_loop(pool: Arc<Self>) {
        let mut shutdown_rx = pool.shutdown.subscribe();
        tokio::spawn(async move {
            let target = pool.config.warm_per_class;
            let interval = Duration::from_secs(5);
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = shutdown_rx.recv() => { break; }
                }

                // Only pre-warm Light class in the background loop.
                let current = {
                    pool.pools
                        .lock()
                        .await
                        .get(&ResourceClass::Light)
                        .map(|q| q.len())
                        .unwrap_or(0)
                };

                for _ in current..target {
                    match create_container_static(&pool.docker, &pool.config, ResourceClass::Light).await {
                        Ok(container) => {
                            pool.pools
                                .lock()
                                .await
                                .entry(ResourceClass::Light)
                                .or_default()
                                .push_back(container);
                            pool.generation.fetch_add(1, Ordering::AcqRel);
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "pre-warm loop: failed to create Light container");
                            break;
                        }
                    }
                }
            }
        });
    }
}

/// Create a container with the correct resource limits for a class.
async fn create_container_static(
    docker: &Docker,
    config: &OpenClawConfig,
    class: ResourceClass,
) -> Result<WarmContainer, PoolError> {
    let container_name = format!("{}-{}-{}", config.container_name, class, uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("x"));

    let memory_limit = match class {
        ResourceClass::Light => 256 * 1024 * 1024i64,   // 256MB
        ResourceClass::Medium => 512 * 1024 * 1024i64,  // 512MB
        ResourceClass::Heavy => 2 * 1024 * 1024 * 1024i64, // 2GB
    };

    let cpu_limit = match class {
        ResourceClass::Light => 500_000i64,   // 0.5 CPU
        ResourceClass::Medium => 1_000_000i64, // 1.0 CPU
        ResourceClass::Heavy => 2_000_000i64,  // 2.0 CPU
    };

    let host_config = HostConfig {
        memory: Some(memory_limit),
        nano_cpus: Some(cpu_limit),
        readonly_rootfs: Some(true),
        network_mode: Some("none".to_string()), // No network by default
        security_opt: Some(vec!["no-new-privileges:true".to_string()]),
        cap_drop: Some(vec!["ALL".to_string()]),
        tmpfs: Some({
            let mut m = HashMap::new();
            m.insert("/workspace".to_string(), "size=256M".to_string());
            m
        }),
        ..Default::default()
    };

    let container_config = ContainerConfig {
        image: Some(config.image.clone()),
        cmd: Some(vec![
            "node".to_string(),
            "--max-old-space-size=256".to_string(),
            "src/mcp-bridge.js".to_string(),
        ]),
        open_stdin: Some(true), // MCP stdio transport
        host_config: Some(host_config),
        ..Default::default()
    };

    let container = docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name,
                platform: None,
            }),
            container_config,
        )
        .await
        .map_err(|e| PoolError::CreationFailed(format!("Container creation failed: {}", e)))?;

    docker
        .start_container(&container.id, None::<StartContainerOptions<String>>)
        .await
        .map_err(|e| PoolError::StartFailed(format!("Container start failed: {}", e)))?;

    tracing::debug!(
        container_id = %container.id,
        resource_class = %class,
        "Created and started OpenClaw container"
    );

    Ok(WarmContainer {
        container_id: container.id,
        created_at: Instant::now(),
        resource_class: class,
    })
}
