//! A8.12 Platform Metrics — ONE metrics pipeline for the ClawHub platform.
//!
//! Downloads, installs, updates, failures, publisher stats, repo/sync latency,
//! verification failures, cache hits, repair events. Backend-agnostic, thread-safe.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Snapshot of platform metrics (A8.12).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlatformMetricsSnapshot {
    pub downloads: u64,
    pub installs: u64,
    pub updates: u64,
    pub failures: u64,
    pub verification_failures: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub repair_events: u64,
    pub sync_runs: u64,
    pub repo_latency_ms_total: u64,
    pub sync_latency_ms_total: u64,
    /// Per-publisher install counts.
    pub publisher_installs: HashMap<String, u64>,
}

/// Thread-safe platform metrics collector.
#[derive(Clone, Default)]
pub struct PlatformMetrics {
    inner: Arc<MetricsInner>,
}

#[derive(Default)]
struct MetricsInner {
    downloads: AtomicU64,
    installs: AtomicU64,
    updates: AtomicU64,
    failures: AtomicU64,
    verification_failures: AtomicU64,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    repair_events: AtomicU64,
    sync_runs: AtomicU64,
    repo_latency_ms_total: AtomicU64,
    sync_latency_ms_total: AtomicU64,
    publisher_installs: Mutex<HashMap<String, u64>>,
}

impl PlatformMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_download(&self) {
        self.inner.downloads.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_install(&self, publisher: &str) {
        self.inner.installs.fetch_add(1, Ordering::Relaxed);
        *self
            .inner
            .publisher_installs
            .lock()
            .unwrap()
            .entry(publisher.to_string())
            .or_insert(0) += 1;
    }
    pub fn inc_update(&self) {
        self.inner.updates.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_failure(&self) {
        self.inner.failures.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_verification_failure(&self) {
        self.inner
            .verification_failures
            .fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_cache_hit(&self) {
        self.inner.cache_hits.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_cache_miss(&self) {
        self.inner.cache_misses.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_repair(&self) {
        self.inner.repair_events.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_sync(&self, latency_ms: u64) {
        self.inner.sync_runs.fetch_add(1, Ordering::Relaxed);
        self.inner
            .sync_latency_ms_total
            .fetch_add(latency_ms, Ordering::Relaxed);
    }
    pub fn record_repo_latency(&self, latency_ms: u64) {
        self.inner
            .repo_latency_ms_total
            .fetch_add(latency_ms, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> PlatformMetricsSnapshot {
        PlatformMetricsSnapshot {
            downloads: self.inner.downloads.load(Ordering::Relaxed),
            installs: self.inner.installs.load(Ordering::Relaxed),
            updates: self.inner.updates.load(Ordering::Relaxed),
            failures: self.inner.failures.load(Ordering::Relaxed),
            verification_failures: self.inner.verification_failures.load(Ordering::Relaxed),
            cache_hits: self.inner.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.inner.cache_misses.load(Ordering::Relaxed),
            repair_events: self.inner.repair_events.load(Ordering::Relaxed),
            sync_runs: self.inner.sync_runs.load(Ordering::Relaxed),
            repo_latency_ms_total: self.inner.repo_latency_ms_total.load(Ordering::Relaxed),
            sync_latency_ms_total: self.inner.sync_latency_ms_total.load(Ordering::Relaxed),
            publisher_installs: self.inner.publisher_installs.lock().unwrap().clone(),
        }
    }
}
