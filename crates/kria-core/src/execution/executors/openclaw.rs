//! A7.4 OpenClaw Executor — the FIRST executor on the generic engine.
//!
//! Adapts KRIA's existing OpenClaw `SkillRuntime` (Docker runtime + runtime manager)
//! to the generic `Executor` interface. The planner/scheduler never see anything
//! OpenClaw-specific — they only see an `Executor` with kind `OpenClaw`.
//!
//! Future executors (GUI, Native, MCP, Browser, Memory, Cloud, Agent) implement the
//! same `Executor` trait and register alongside this one (A7.13). No engine changes.

use crate::execution::context::ExecutionContext;
use crate::execution::executor::{
    ExecutionRequest, Executor, ExecutorError, ExecutorHealth, ExecutorMetrics,
};
use crate::infra::isolation::ToolResult;
use crate::openclaw::runtime::{LaunchSpec, RuntimeContext, RuntimeKind, SkillRuntime};
use crate::openclaw::types::ResourceClass;
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// OpenClaw execution backend adapted to the generic `Executor` interface (A7.4).
pub struct OpenClawExecutor {
    runtime: Arc<dyn SkillRuntime>,
    default_timeout: Duration,
    executions: AtomicU64,
    successes: AtomicU64,
    failures: AtomicU64,
    total_latency_ms: AtomicU64,
}

impl OpenClawExecutor {
    /// Build from any `SkillRuntime` (Docker runtime in A1+, runtime manager in A4).
    pub fn new(runtime: Arc<dyn SkillRuntime>) -> Self {
        Self {
            runtime,
            default_timeout: Duration::from_secs(120),
            executions: AtomicU64::new(0),
            successes: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            total_latency_ms: AtomicU64::new(0),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Map a resource hint string to a `ResourceClass` (defaults to Light).
    fn resource_class(hint: Option<&str>) -> ResourceClass {
        match hint {
            Some("medium") => ResourceClass::Medium,
            Some("heavy") => ResourceClass::Heavy,
            _ => ResourceClass::Light,
        }
    }
}

#[async_trait]
impl Executor for OpenClawExecutor {
    fn provider_id(&self) -> String {
        crate::capability::acl::openclaw::OPENCLAW_PROVIDER_ID.to_string()
    }

    async fn validate(
        &self,
        req: &ExecutionRequest,
        _ctx: &ExecutionContext,
    ) -> Result<(), ExecutorError> {
        if req.action_id.is_empty() {
            return Err(ExecutorError::Validation("empty action_id".into()));
        }
        Ok(())
    }

    async fn execute(&self, req: &ExecutionRequest, ctx: &ExecutionContext) -> ToolResult {
        self.executions.fetch_add(1, Ordering::Relaxed);
        let start = Instant::now();

        let spec = LaunchSpec {
            skill_id: req.action_id.clone(),
            params: req.params.clone(),
            resource_class: Self::resource_class(req.resource_hint.as_deref()),
            timeout: self.default_timeout,
            correlation_id: ctx.correlation_id.clone(),
            grants: Vec::new(),
            mounted_skill_dir: None,
        };

        let runtime_ctx = RuntimeContext {
            cancellation: ctx.cancellation.clone(),
        };

        let result = self.runtime.execute(spec, runtime_ctx).await;

        let latency = start.elapsed().as_millis() as u64;
        self.total_latency_ms.fetch_add(latency, Ordering::Relaxed);
        if result.success {
            self.successes.fetch_add(1, Ordering::Relaxed);
        } else {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    async fn metrics(&self) -> ExecutorMetrics {
        ExecutorMetrics {
            executions: self.executions.load(Ordering::Relaxed),
            successes: self.successes.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
            total_latency_ms: self.total_latency_ms.load(Ordering::Relaxed),
        }
    }

    async fn health(&self) -> ExecutorHealth {
        // Docker backend is considered available if the runtime is Docker-kind.
        let available = matches!(self.runtime.kind(), RuntimeKind::Docker | RuntimeKind::Gpu);
        ExecutorHealth {
            available,
            detail: format!("openclaw runtime kind={}", self.runtime.kind().as_str()),
        }
    }
}

/// Build an `OpenClawExecutor` from a container pool by wrapping the Docker runtime.
///
/// This is the boot wiring: the generic `ExecutionEngine` stays backend-agnostic;
/// the knowledge that "OpenClaw runs on the Docker runtime" lives here in the
/// executors boundary, never in the planner/scheduler.
pub fn openclaw_executor_from_pool(
    pool: Arc<crate::openclaw::pool::ContainerPool>,
) -> OpenClawExecutor {
    let runtime: Arc<dyn SkillRuntime> =
        Arc::new(crate::openclaw::runtime::DockerRuntime::new(pool));
    OpenClawExecutor::new(runtime)
}
