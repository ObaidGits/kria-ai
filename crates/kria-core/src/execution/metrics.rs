//! A7.11 Metrics — ONE metrics pipeline for the execution engine.
//!
//! Collects planning/optimization/execution latency, critical path, graph depth,
//! parallelism, node count, executor & resource utilization, retry/rollback counts,
//! success/failure rates, cache and checkpoint hits. Backend-agnostic.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Snapshot of engine metrics (A7.11).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionMetricsSnapshot {
    pub planning_latency_ms: u64,
    pub optimization_latency_ms: u64,
    pub execution_latency_ms: u64,
    pub critical_path_len: usize,
    pub graph_depth: usize,
    pub max_parallelism: usize,
    pub node_count: usize,
    pub retry_count: u64,
    pub rollback_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub cache_hits: u64,
    pub checkpoint_hits: u64,
    /// Per-executor execution counts (utilization).
    pub executor_utilization: HashMap<String, u64>,
}

impl ExecutionMetricsSnapshot {
    pub fn success_rate(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            0.0
        } else {
            self.success_count as f64 / total as f64
        }
    }

    pub fn failure_rate(&self) -> f64 {
        1.0 - self.success_rate()
    }
}

/// Thread-safe metrics collector. Cheaply cloneable (Arc counters).
#[derive(Clone, Default)]
pub struct ExecutionMetrics {
    inner: Arc<MetricsInner>,
}

#[derive(Default)]
struct MetricsInner {
    planning_latency_ms: AtomicU64,
    optimization_latency_ms: AtomicU64,
    execution_latency_ms: AtomicU64,
    critical_path_len: AtomicU64,
    graph_depth: AtomicU64,
    max_parallelism: AtomicU64,
    node_count: AtomicU64,
    retry_count: AtomicU64,
    rollback_count: AtomicU64,
    success_count: AtomicU64,
    failure_count: AtomicU64,
    cache_hits: AtomicU64,
    checkpoint_hits: AtomicU64,
    executor_utilization: std::sync::Mutex<HashMap<String, u64>>,
}

impl ExecutionMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_planning_latency(&self, ms: u64) {
        self.inner.planning_latency_ms.store(ms, Ordering::Relaxed);
    }
    pub fn set_optimization_latency(&self, ms: u64) {
        self.inner
            .optimization_latency_ms
            .store(ms, Ordering::Relaxed);
    }
    pub fn set_execution_latency(&self, ms: u64) {
        self.inner.execution_latency_ms.store(ms, Ordering::Relaxed);
    }
    pub fn set_graph_shape(&self, depth: usize, parallelism: usize, nodes: usize) {
        self.inner
            .graph_depth
            .store(depth as u64, Ordering::Relaxed);
        self.inner
            .critical_path_len
            .store(depth as u64, Ordering::Relaxed);
        self.inner
            .max_parallelism
            .store(parallelism as u64, Ordering::Relaxed);
        self.inner.node_count.store(nodes as u64, Ordering::Relaxed);
    }
    pub fn inc_retry(&self) {
        self.inner.retry_count.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_rollback(&self) {
        self.inner.rollback_count.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_success(&self) {
        self.inner.success_count.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_failure(&self) {
        self.inner.failure_count.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_cache_hit(&self) {
        self.inner.cache_hits.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_checkpoint_hit(&self) {
        self.inner.checkpoint_hits.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_executor(&self, executor: &str) {
        let mut map = self.inner.executor_utilization.lock().unwrap();
        *map.entry(executor.to_string()).or_insert(0) += 1;
    }

    pub fn snapshot(&self) -> ExecutionMetricsSnapshot {
        ExecutionMetricsSnapshot {
            planning_latency_ms: self.inner.planning_latency_ms.load(Ordering::Relaxed),
            optimization_latency_ms: self.inner.optimization_latency_ms.load(Ordering::Relaxed),
            execution_latency_ms: self.inner.execution_latency_ms.load(Ordering::Relaxed),
            critical_path_len: self.inner.critical_path_len.load(Ordering::Relaxed) as usize,
            graph_depth: self.inner.graph_depth.load(Ordering::Relaxed) as usize,
            max_parallelism: self.inner.max_parallelism.load(Ordering::Relaxed) as usize,
            node_count: self.inner.node_count.load(Ordering::Relaxed) as usize,
            retry_count: self.inner.retry_count.load(Ordering::Relaxed),
            rollback_count: self.inner.rollback_count.load(Ordering::Relaxed),
            success_count: self.inner.success_count.load(Ordering::Relaxed),
            failure_count: self.inner.failure_count.load(Ordering::Relaxed),
            cache_hits: self.inner.cache_hits.load(Ordering::Relaxed),
            checkpoint_hits: self.inner.checkpoint_hits.load(Ordering::Relaxed),
            executor_utilization: self.inner.executor_utilization.lock().unwrap().clone(),
        }
    }
}
