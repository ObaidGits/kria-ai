//! A7.5 Execution Scheduler — ONE generic scheduler. No backend assumptions.
//!
//! Drives an `ExecutionGraph` to completion: sequential, parallel, conditional,
//! join/merge, retry, timeout, cancellation, rollback and checkpoint. Skill nodes
//! are dispatched to an `Executor` resolved from the registry by `ExecutorKind`;
//! the scheduler never references a concrete backend.

use super::context::ExecutionContext;
use super::events::{ExecutionEvent, ExecutionEventStream};
use super::executor::{ExecutionRequest, ExecutorRegistry};
use super::graph::{ExecutionGraph, NodeKind};
use super::metrics::ExecutionMetrics;
use super::recovery::{RecoveryManager, RecoveryOutcome, RecoveryPolicy};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// Terminal status of a scheduled graph run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleStatus {
    Completed,
    Failed(String),
    Cancelled,
}

/// Result of running a graph.
#[derive(Debug, Clone)]
pub struct ScheduleResult {
    pub status: ScheduleStatus,
    pub completed_nodes: Vec<String>,
    pub failed_nodes: Vec<String>,
}

/// The single generic scheduler (A7.5).
pub struct ExecutionScheduler {
    registry: ExecutorRegistry,
    events: ExecutionEventStream,
    metrics: ExecutionMetrics,
    recovery: RecoveryPolicy,
}

impl ExecutionScheduler {
    pub fn new(
        registry: ExecutorRegistry,
        events: ExecutionEventStream,
        metrics: ExecutionMetrics,
    ) -> Self {
        Self {
            registry,
            events,
            metrics,
            recovery: RecoveryPolicy::default(),
        }
    }

    pub fn with_recovery(mut self, policy: RecoveryPolicy) -> Self {
        self.recovery = policy;
        self
    }

    /// Run a graph to completion. Waves of ready nodes execute in parallel.
    pub async fn run(&self, graph: &ExecutionGraph, ctx: &ExecutionContext) -> ScheduleResult {
        let start = Instant::now();
        self.events.emit(ExecutionEvent::ExecutionStarted {
            graph_id: graph.id.clone(),
        });

        let completed: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let failed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        // Checkpoint store: label → snapshot of outputs.
        let checkpoints: Arc<Mutex<HashMap<String, HashMap<String, serde_json::Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        loop {
            if ctx.is_cancelled() {
                self.events.emit(ExecutionEvent::Cancelled {
                    graph_id: graph.id.clone(),
                    node_id: None,
                });
                return ScheduleResult {
                    status: ScheduleStatus::Cancelled,
                    completed_nodes: completed.lock().await.iter().cloned().collect(),
                    failed_nodes: failed.lock().await.clone(),
                };
            }

            // Find ready nodes: all deps completed, not yet completed/failed.
            let ready = {
                let comp = completed.lock().await;
                let fail = failed.lock().await;
                self.ready_nodes(graph, &comp, &fail)
            };
            if ready.is_empty() {
                break;
            }

            // Execute the ready wave in parallel.
            let mut handles = Vec::new();
            for node_id in ready {
                let graph_c = graph.clone();
                let ctx_c = ctx.clone();
                let registry_c = self.registry.clone();
                let events_c = self.events.clone();
                let metrics_c = self.metrics.clone();
                let recovery_c = self.recovery.clone();
                let checkpoints_c = checkpoints.clone();
                handles.push(tokio::spawn(async move {
                    let ok = run_node(
                        &graph_c,
                        &node_id,
                        &ctx_c,
                        &registry_c,
                        &events_c,
                        &metrics_c,
                        &recovery_c,
                        &checkpoints_c,
                    )
                    .await;
                    (node_id, ok)
                }));
            }

            let mut wave_failed = false;
            for handle in handles {
                if let Ok((node_id, ok)) = handle.await {
                    if ok {
                        completed.lock().await.insert(node_id);
                    } else {
                        failed.lock().await.push(node_id);
                        wave_failed = true;
                    }
                }
            }

            if wave_failed {
                let reason = format!("nodes failed: {:?}", failed.lock().await);
                self.events.emit(ExecutionEvent::GraphFailed {
                    graph_id: graph.id.clone(),
                    reason: reason.clone(),
                });
                self.metrics
                    .set_execution_latency(start.elapsed().as_millis() as u64);
                return ScheduleResult {
                    status: ScheduleStatus::Failed(reason),
                    completed_nodes: completed.lock().await.iter().cloned().collect(),
                    failed_nodes: failed.lock().await.clone(),
                };
            }
        }

        let latency = start.elapsed().as_millis() as u64;
        self.metrics.set_execution_latency(latency);
        self.events.emit(ExecutionEvent::GraphCompleted {
            graph_id: graph.id.clone(),
            latency_ms: latency,
        });

        let completed_nodes: Vec<String> = completed.lock().await.iter().cloned().collect();
        ScheduleResult {
            status: ScheduleStatus::Completed,
            completed_nodes,
            failed_nodes: Vec::new(),
        }
    }

    /// Nodes whose dependencies are all completed and are not done/failed yet.
    fn ready_nodes(
        &self,
        graph: &ExecutionGraph,
        completed: &HashSet<String>,
        failed: &[String],
    ) -> Vec<String> {
        let failed_set: HashSet<&String> = failed.iter().collect();
        graph
            .nodes()
            .filter(|n| !completed.contains(&n.id) && !failed_set.contains(&n.id))
            .filter(|n| n.dependencies.iter().all(|d| completed.contains(d)))
            .map(|n| n.id.clone())
            .collect()
    }
}

/// Execute a single node. Returns true on success. Handles control-flow node kinds
/// and dispatches Skill nodes to the resolved executor with retry/recovery.
#[allow(clippy::too_many_arguments)]
async fn run_node(
    graph: &ExecutionGraph,
    node_id: &str,
    ctx: &ExecutionContext,
    registry: &ExecutorRegistry,
    events: &ExecutionEventStream,
    metrics: &ExecutionMetrics,
    recovery: &RecoveryPolicy,
    checkpoints: &Arc<Mutex<HashMap<String, HashMap<String, serde_json::Value>>>>,
) -> bool {
    let node = match graph.get(node_id) {
        Some(n) => n,
        None => return false,
    };

    events.emit(ExecutionEvent::NodeStarted {
        graph_id: graph.id.clone(),
        node_id: node_id.to_string(),
        kind: node.kind_name().to_string(),
    });
    let start = Instant::now();

    let ok = match &node.kind {
        NodeKind::Skill {
            provider_id,
            action_id,
            params,
        } => {
            run_skill_node(
                graph,
                node_id,
                provider_id,
                action_id,
                params,
                ctx,
                registry,
                events,
                metrics,
                recovery,
            )
            .await
        }
        // Control-flow structural nodes: succeed immediately (dependencies enforce order).
        NodeKind::Parallel | NodeKind::Merge | NodeKind::Barrier => true,
        NodeKind::Condition { expression } => eval_condition(ctx, expression).await,
        NodeKind::Loop { .. } => true, // loop body modeled as dependent skill nodes
        NodeKind::Wait { millis } => {
            tokio::time::sleep(std::time::Duration::from_millis(*millis)).await;
            true
        }
        NodeKind::Timeout { .. } => true, // timeout enforced at skill level
        NodeKind::Retry { .. } => true,   // retry enforced at skill level
        NodeKind::Subgraph { .. } => true, // nested graphs run by the engine
        NodeKind::Checkpoint { label } => {
            let snapshot = ctx.all_outputs().await;
            checkpoints.lock().await.insert(label.clone(), snapshot);
            metrics.inc_checkpoint_hit();
            true
        }
        NodeKind::Rollback { to_label } => {
            let store = checkpoints.lock().await;
            if let Some(snapshot) = store.get(to_label) {
                for (k, v) in snapshot.iter() {
                    ctx.set_output(k.clone(), v.clone()).await;
                }
                metrics.inc_rollback();
                events.emit(ExecutionEvent::Rollback {
                    graph_id: graph.id.clone(),
                    node_id: node_id.to_string(),
                });
                true
            } else {
                false
            }
        }
    };

    let latency = start.elapsed().as_millis() as u64;
    if ok {
        metrics.inc_success();
        events.emit(ExecutionEvent::NodeCompleted {
            graph_id: graph.id.clone(),
            node_id: node_id.to_string(),
            latency_ms: latency,
        });
    } else {
        metrics.inc_failure();
        events.emit(ExecutionEvent::NodeFailed {
            graph_id: graph.id.clone(),
            node_id: node_id.to_string(),
            reason: "node execution failed".to_string(),
        });
    }
    ok
}

/// Dispatch a skill node to its executor, applying the frozen lifecycle + retry policy.
#[allow(clippy::too_many_arguments)]
async fn run_skill_node(
    graph: &ExecutionGraph,
    node_id: &str,
    provider_id: &str,
    action_id: &str,
    params: &serde_json::Value,
    ctx: &ExecutionContext,
    registry: &ExecutorRegistry,
    events: &ExecutionEventStream,
    metrics: &ExecutionMetrics,
    recovery: &RecoveryPolicy,
) -> bool {
    let executor = match registry.get(provider_id) {
        Some(e) => e,
        None => return false,
    };

    let req = ExecutionRequest {
        node_id: node_id.to_string(),
        action_id: action_id.to_string(),
        params: params.clone(),
        resource_hint: None,
    };

    let mut attempt: u32 = 0;
    loop {
        if ctx.is_cancelled() {
            let _ = executor.cancel(node_id).await;
            return false;
        }

        events.emit(ExecutionEvent::ExecutorStarted {
            executor: provider_id.to_string(),
            node_id: node_id.to_string(),
        });
        metrics.record_executor(provider_id);

        // Frozen lifecycle: prepare → validate → admit → execute.
        let lifecycle_ok = executor.prepare(&req, ctx).await.is_ok()
            && executor.validate(&req, ctx).await.is_ok()
            && executor.admit(&req, ctx).await.is_ok();

        let result = if lifecycle_ok {
            let r = executor.execute(&req, ctx).await;
            if r.success {
                ctx.set_output(node_id.to_string(), r.data.clone()).await;
            }
            r.success
        } else {
            false
        };

        let _ = executor.cleanup(node_id).await;
        events.emit(ExecutionEvent::ExecutorFinished {
            executor: provider_id.to_string(),
            node_id: node_id.to_string(),
            success: result,
        });

        if result {
            return true;
        }

        // Failure → consult recovery policy.
        attempt += 1;
        match RecoveryManager::decide(recovery, attempt) {
            RecoveryOutcome::RetryNow => {
                let backoff = RecoveryManager::backoff_ms(recovery, attempt);
                events.emit(ExecutionEvent::Retry {
                    graph_id: graph.id.clone(),
                    node_id: node_id.to_string(),
                    attempt,
                });
                metrics.inc_retry();
                let _ = executor.recover(&req, ctx).await;
                if backoff > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                }
                continue;
            }
            RecoveryOutcome::Continue => return true, // partial completion accepted
            _ => return false, // Abort/Cancel/alternate handled by engine level
        }
    }
}

/// Evaluate a simple condition against context variables.
/// Supports "var == value", "var != value", or "var" (truthy) forms.
async fn eval_condition(ctx: &ExecutionContext, expression: &str) -> bool {
    let expr = expression.trim();
    if let Some((lhs, rhs)) = expr.split_once("==") {
        let key = lhs.trim();
        let want = rhs.trim().trim_matches('"');
        match ctx.get_var(key).await {
            Some(v) => value_as_string(&v) == want,
            None => false,
        }
    } else if let Some((lhs, rhs)) = expr.split_once("!=") {
        let key = lhs.trim();
        let want = rhs.trim().trim_matches('"');
        match ctx.get_var(key).await {
            Some(v) => value_as_string(&v) != want,
            None => true,
        }
    } else {
        // truthy check
        match ctx.get_var(expr).await {
            Some(serde_json::Value::Bool(b)) => b,
            Some(serde_json::Value::Null) | None => false,
            Some(_) => true,
        }
    }
}

fn value_as_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}
