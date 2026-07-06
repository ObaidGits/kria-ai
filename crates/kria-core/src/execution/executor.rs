//! A7.3 Executor Interface + A7.12 Executor Registry.
//!
//! The single authoritative interface every execution backend implements. The
//! planner and scheduler NEVER special-case a backend — they select an `Executor`
//! from the `ExecutorRegistry` by `ExecutorKind` and drive it through the frozen
//! lifecycle: prepare → validate → admit → execute → cancel/recover → cleanup.
//!
//! OpenClaw is the first `Executor` (A7.4). GUI, Native, MCP, Browser, Memory,
//! Cloud and Agent executors plug into the exact same interface later — no planner
//! changes required (A7.13).

use super::context::ExecutionContext;
use crate::infra::isolation::ToolResult;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// A unit of work handed to an executor. Backend-agnostic — the executor decides HOW.
#[derive(Debug, Clone)]
pub struct ExecutionRequest {
    /// Node id in the execution graph (for correlation).
    pub node_id: String,
    /// Logical action/skill id the executor should run.
    pub action_id: String,
    /// Parameters for the action.
    pub params: serde_json::Value,
    /// Optional resource hint (executor may override).
    pub resource_hint: Option<String>,
}

/// Health snapshot reported by an executor (A7.3 health()).
#[derive(Debug, Clone, Default)]
pub struct ExecutorHealth {
    pub available: bool,
    pub detail: String,
}

/// Per-executor metrics snapshot (A7.3 metrics(), feeds A7.11).
#[derive(Debug, Clone, Default)]
pub struct ExecutorMetrics {
    pub executions: u64,
    pub successes: u64,
    pub failures: u64,
    pub total_latency_ms: u64,
}

/// The single interface every execution backend implements (A7.3).
///
/// Default methods make prepare/validate/admit/recover/cleanup optional so simple
/// executors only implement `execute`. Lifecycle order is fixed by the scheduler.
#[async_trait]
pub trait Executor: Send + Sync {
    /// The open-vocabulary provider id this executor handles (e.g. `"openclaw"`,
    /// `"mcp:github"`). Replaces the former closed `ExecutorKind` enum so a new
    /// provider is registered under a string, never a new KRIA-core enum variant
    /// (CPP R1.3).
    fn provider_id(&self) -> String;

    /// Prepare resources before admission (warm pool, connections). Optional.
    async fn prepare(
        &self,
        _req: &ExecutionRequest,
        _ctx: &ExecutionContext,
    ) -> Result<(), ExecutorError> {
        Ok(())
    }

    /// Validate the request is well-formed and runnable. Optional.
    async fn validate(
        &self,
        _req: &ExecutionRequest,
        _ctx: &ExecutionContext,
    ) -> Result<(), ExecutorError> {
        Ok(())
    }

    /// Admit against resource authority (HRA). Optional (defaults to admitted).
    async fn admit(
        &self,
        _req: &ExecutionRequest,
        _ctx: &ExecutionContext,
    ) -> Result<(), ExecutorError> {
        Ok(())
    }

    /// Run the unit of work end-to-end. Required.
    async fn execute(&self, req: &ExecutionRequest, ctx: &ExecutionContext) -> ToolResult;

    /// Cancel an in-flight execution for a node. Optional.
    async fn cancel(&self, _node_id: &str) -> Result<(), ExecutorError> {
        Ok(())
    }

    /// Attempt backend-level recovery after failure. Optional.
    async fn recover(
        &self,
        _req: &ExecutionRequest,
        _ctx: &ExecutionContext,
    ) -> Result<(), ExecutorError> {
        Ok(())
    }

    /// Release resources after execution. Optional.
    async fn cleanup(&self, _node_id: &str) -> Result<(), ExecutorError> {
        Ok(())
    }

    /// Report metrics. Optional.
    async fn metrics(&self) -> ExecutorMetrics {
        ExecutorMetrics::default()
    }

    /// Report health. Optional (defaults to available).
    async fn health(&self) -> ExecutorHealth {
        ExecutorHealth {
            available: true,
            detail: "ok".into(),
        }
    }
}

/// Errors surfaced by executors during the lifecycle.
#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("admission denied: {0}")]
    AdmissionDenied(String),
    #[error("preparation failed: {0}")]
    Preparation(String),
    #[error("execution failed: {0}")]
    Execution(String),
    #[error("cancelled")]
    Cancelled,
    #[error("recovery failed: {0}")]
    Recovery(String),
    #[error("executor unavailable: {0}")]
    Unavailable(String),
}

/// A7.12 Executor Registry — ONE registry. Planner discovers executors here.
///
/// No hardcoded executors. OpenClaw registers at boot; future executors register
/// through this same API. Pure dispatch — contains no execution logic.
#[derive(Default, Clone)]
pub struct ExecutorRegistry {
    executors: HashMap<String, Arc<dyn Executor>>,
}

impl ExecutorRegistry {
    pub fn new() -> Self {
        Self {
            executors: HashMap::new(),
        }
    }

    /// Register an executor. Keyed by its open-vocabulary `provider_id`.
    pub fn register(&mut self, executor: Arc<dyn Executor>) {
        self.executors.insert(executor.provider_id(), executor);
    }

    /// Look up an executor by provider id.
    pub fn get(&self, provider_id: &str) -> Option<Arc<dyn Executor>> {
        self.executors.get(provider_id).cloned()
    }

    /// Whether a backend is available for the given provider id.
    pub fn has(&self, provider_id: &str) -> bool {
        self.executors.contains_key(provider_id)
    }

    /// All registered provider ids (planner discovery).
    pub fn available_providers(&self) -> Vec<String> {
        self.executors.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.executors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.executors.is_empty()
    }
}
