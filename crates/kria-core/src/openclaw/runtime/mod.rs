//! SkillRuntime — the single execution interface every backend satisfies (execution-contract
//! INV-4). The agent loop / handler never special-cases a backend: it selects a `SkillRuntime`
//! from the `RuntimeRegistry` by the skill's declared `RuntimeKind` and calls `execute`.
//!
//! `execute` runs the full frozen lifecycle internally (admit → launch → call → cancel/recover →
//! cleanup) and emits `SkillEvent`s throughout. A1 ships the Docker backend; other `RuntimeKind`s
//! are reserved variants wired in later phases (no stub implementations are registered).

pub mod docker;

use super::types::{ResourceClass, SkillDescriptor};
use crate::infra::isolation::ToolResult;
use crate::tools::ToolContext;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Execution backend kind. Selection is data-driven from skill metadata (execution-contract §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeKind {
    Docker,
    Wasm,
    Firecracker,
    Remote,
    Cloud,
    Gpu,
}

impl RuntimeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Wasm => "wasm",
            Self::Firecracker => "firecracker",
            Self::Remote => "remote",
            Self::Cloud => "cloud",
            Self::Gpu => "gpu",
        }
    }
}

/// One skill invocation request handed to a runtime.
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    pub skill_id: String,
    pub params: serde_json::Value,
    pub resource_class: ResourceClass,
    pub timeout: Duration,
    /// Correlation id tying request → events → audit (spans composition; event-contract §5).
    pub correlation_id: String,
    /// Granted capabilities to materialize into the container (A3). Empty → locked default.
    pub grants: Vec<crate::openclaw::capability::CapabilityGrant>,
    /// Host directory containing a bridge-format skill descriptor
    /// (`<slug>.json`) + its handler, for an installed marketplace/generated
    /// skill whose handler is NOT baked into the substrate image. When set,
    /// `DockerRuntime` runs a bespoke container that bind-mounts this dir
    /// read-only so the MCP bridge can load and execute the skill at runtime.
    /// `None` for baked-in skills (they run from the warm pool). This is the
    /// mechanism that makes install→enable→execute work for skills added
    /// after image build time (additive materialization, A3).
    pub mounted_skill_dir: Option<std::path::PathBuf>,
}

/// Ambient context for an execution (cancellation propagation; extend later for grants, etc.).
#[derive(Clone)]
pub struct RuntimeContext {
    pub cancellation: tokio_util::sync::CancellationToken,
}

impl RuntimeContext {
    pub fn from_tool_context(ctx: &ToolContext) -> Self {
        Self {
            cancellation: ctx.cancellation.clone(),
        }
    }

    pub fn detached() -> Self {
        Self {
            cancellation: tokio_util::sync::CancellationToken::new(),
        }
    }
}

/// The single interface every execution backend implements (INV-4).
#[async_trait]
pub trait SkillRuntime: Send + Sync {
    fn kind(&self) -> RuntimeKind;

    /// Run one skill invocation end-to-end: HRA admission → launch → JSON-RPC call → cleanup,
    /// emitting `SkillEvent`s and honouring cancellation + timeout. Returns the raw skill result
    /// (evidence-wrapping + audit are applied by the caller).
    async fn execute(&self, spec: LaunchSpec, ctx: RuntimeContext) -> ToolResult;
}

/// Registry of available runtimes, keyed by `RuntimeKind`. Populated at boot from host
/// capabilities. Pure dispatch — no logic.
#[derive(Default)]
pub struct RuntimeRegistry {
    runtimes: HashMap<RuntimeKind, Arc<dyn SkillRuntime>>,
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self {
            runtimes: HashMap::new(),
        }
    }

    pub fn register(&mut self, runtime: Arc<dyn SkillRuntime>) {
        self.runtimes.insert(runtime.kind(), runtime);
    }

    pub fn get(&self, kind: RuntimeKind) -> Option<Arc<dyn SkillRuntime>> {
        self.runtimes.get(&kind).cloned()
    }

    /// Resolve the runtime a skill should use. A1: every OpenClaw skill runs on Docker. Later
    /// phases read `manifest.runtime.kind` from the bundle (package-contract) here.
    pub fn kind_for_skill(_skill: &SkillDescriptor) -> RuntimeKind {
        RuntimeKind::Docker
    }
}

pub use docker::DockerRuntime;
