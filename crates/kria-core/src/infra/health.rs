use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::Serialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Service health status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceStatus {
    Unknown,
    Starting,
    Healthy,
    Degraded,
    Unhealthy,
    Stopped,
}

/// Health entry for a single service.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceHealth {
    pub name: String,
    pub status: ServiceStatus,
    pub last_check: Option<DateTime<Utc>>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthSnapshot {
    pub status: String,
    pub collected_at: DateTime<Utc>,
    pub total_services: usize,
    pub healthy_services: usize,
    pub degraded_services: Vec<String>,
    pub unhealthy_services: Vec<String>,
    pub starting_services: Vec<String>,
    pub stopped_services: Vec<String>,
    pub unknown_services: Vec<String>,
    pub event_count: usize,
    pub services: Vec<ServiceHealth>,
}

/// Global health registry, shared across the application.
#[derive(Debug)]
pub struct HealthRegistry {
    services: DashMap<String, ServiceHealth>,
    event_count: AtomicUsize,
}

impl Default for HealthRegistry {
    fn default() -> Self {
        Self {
            services: DashMap::new(),
            event_count: AtomicUsize::new(0),
        }
    }
}

impl HealthRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new service (initial status: Unknown).
    pub fn register(&self, name: impl Into<String>) {
        let name = name.into();
        self.services.insert(
            name.clone(),
            ServiceHealth {
                name,
                status: ServiceStatus::Unknown,
                last_check: None,
                message: None,
            },
        );
    }

    /// Update a service's status.
    pub fn update(&self, name: &str, status: ServiceStatus, message: Option<String>) {
        if let Some(mut entry) = self.services.get_mut(name) {
            entry.status = status;
            entry.last_check = Some(Utc::now());
            entry.message = message;
        }
    }

    /// Get a single service's health.
    pub fn get(&self, name: &str) -> Option<ServiceHealth> {
        self.services.get(name).map(|e| e.clone())
    }

    /// Get all services' health as a snapshot.
    pub fn status_all(&self) -> Vec<ServiceHealth> {
        let mut services: Vec<_> = self.services.iter().map(|e| e.value().clone()).collect();
        services.sort_by(|a, b| a.name.cmp(&b.name));
        services
    }

    /// True if all registered services are Healthy.
    pub fn all_healthy(&self) -> bool {
        self.services
            .iter()
            .all(|e| e.value().status == ServiceStatus::Healthy)
    }

    /// Increment the cumulative event counter.
    pub fn inc_events(&self, n: usize) {
        self.event_count.fetch_add(n, Ordering::Relaxed);
    }

    /// Return the total number of events observed since startup.
    pub fn event_count(&self) -> usize {
        self.event_count.load(Ordering::Relaxed)
    }

    pub fn snapshot(&self) -> HealthSnapshot {
        let services = self.status_all();
        let mut healthy_services = 0;
        let mut degraded_services = Vec::new();
        let mut unhealthy_services = Vec::new();
        let mut starting_services = Vec::new();
        let mut stopped_services = Vec::new();
        let mut unknown_services = Vec::new();

        for service in &services {
            match service.status {
                ServiceStatus::Healthy => healthy_services += 1,
                ServiceStatus::Degraded => degraded_services.push(service.name.clone()),
                ServiceStatus::Unhealthy => unhealthy_services.push(service.name.clone()),
                ServiceStatus::Starting => starting_services.push(service.name.clone()),
                ServiceStatus::Stopped => stopped_services.push(service.name.clone()),
                ServiceStatus::Unknown => unknown_services.push(service.name.clone()),
            }
        }

        let status = if unhealthy_services.is_empty()
            && degraded_services.is_empty()
            && starting_services.is_empty()
            && stopped_services.is_empty()
            && unknown_services.is_empty()
        {
            "healthy"
        } else if unhealthy_services.is_empty() {
            "degraded"
        } else {
            "unhealthy"
        };

        HealthSnapshot {
            status: status.to_string(),
            collected_at: Utc::now(),
            total_services: services.len(),
            healthy_services,
            degraded_services,
            unhealthy_services,
            starting_services,
            stopped_services,
            unknown_services,
            event_count: self.event_count(),
            services,
        }
    }
}

/// Lightweight periodic health reporter.
///
/// Spawns a background task that logs a snapshot of all services every
/// `interval_secs`.  The snapshot is emitted as a structured `tracing::info!`
/// event under the `runtime_health` target so log aggregators can chart it.
pub struct RuntimeHealthReporter;

impl RuntimeHealthReporter {
    /// Spawn a background task that reports health every `interval_secs`.
    pub fn spawn(health: Arc<HealthRegistry>, interval_secs: u64) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;
                let snapshot = health.snapshot();
                tracing::info!(
                    target: "runtime_health",
                    status = %snapshot.status,
                    total_services = snapshot.total_services,
                    healthy_services = snapshot.healthy_services,
                    degraded_services = ?snapshot.degraded_services,
                    unhealthy_services = ?snapshot.unhealthy_services,
                    starting_services = ?snapshot.starting_services,
                    event_count = snapshot.event_count,
                    "runtime health snapshot"
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_snapshot_counts_services() {
        let health = HealthRegistry::new();
        health.register("model_router");
        health.register("gui_uinput_daemon");
        health.update("model_router", ServiceStatus::Healthy, None);
        health.update(
            "gui_uinput_daemon",
            ServiceStatus::Degraded,
            Some("sidecar unavailable".into()),
        );

        let snapshot = health.snapshot();
        assert_eq!(snapshot.status, "degraded");
        assert_eq!(snapshot.total_services, 2);
        assert_eq!(snapshot.healthy_services, 1);
        assert_eq!(snapshot.degraded_services, vec!["gui_uinput_daemon"]);
    }
}
