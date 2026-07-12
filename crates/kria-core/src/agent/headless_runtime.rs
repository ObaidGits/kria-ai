//! Minimal headless agent runtime (Phase 0.4).
//!
//! Builds a functional [`AgentLoop`] without the desktop's Tauri/UI graph, so
//! the standalone server (and other headless hosts) can stream the real agent
//! over a WebSocket. This is the **v0** runtime: model router + core tool
//! registry + safety/HITL/audit/rollback. It deliberately omits the desktop's
//! heavy optional engines (MCP/GW tools, semantic router, world model, the
//! Batch2/Batch3 cognition engines) — those can be layered on later via the
//! `AgentLoop` builder methods without changing this entry point.
//!
//! Desktop keeps its richer builder in `kria-desktop`; this shared constructor
//! is the single place headless hosts get a working loop.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::agent::AgentLoop;
use crate::config::KriaConfig;
use crate::llm::ModelRouter;
use crate::safety::hitl::HitlGateway;
use crate::safety::{AuditLogger, PolicyEngine, RollbackManager};
use crate::tools::mount_manager::ToolMountManager;
use crate::tools::registry::{build_registry_with_store, ToolRegistry};

/// Default HITL approval timeout for headless hosts (seconds).
const HEADLESS_HITL_TIMEOUT_SECS: u64 = 60;

/// Handles produced by [`build_minimal`], so the host can wire the loop and
/// resolve HITL approvals (e.g. from WebSocket `approve`/`deny` frames).
pub struct HeadlessRuntime {
    pub agent_loop: Arc<AgentLoop>,
    pub hitl: Arc<HitlGateway>,
    pub tool_registry: Arc<ToolRegistry>,
    pub model_router: Arc<ModelRouter>,
}

/// Build a minimal but functional agent loop from configuration.
///
/// Persists audit/rollback under the standard KRIA data paths. Returns an
/// `Arc<AgentLoop>` ready to `run`/`run_with_profile`.
pub fn build_minimal(config: &KriaConfig) -> anyhow::Result<HeadlessRuntime> {
    let paths = crate::platform::paths::KriaPaths::resolve();

    let model_router = Arc::new(ModelRouter::from_config(config));

    // Core tool registry (no memory/RAG in v0 headless).
    let tool_registry = Arc::new(build_registry_with_store(None));

    let mount_manager = Arc::new(RwLock::new(ToolMountManager::new()));
    let policy_engine = Arc::new(PolicyEngine::new());
    let hitl = Arc::new(HitlGateway::new(HEADLESS_HITL_TIMEOUT_SECS));

    // Audit log persisted into the shared kria.db (self-initialises its schema).
    // settings-config-revamp Task 9: unified onto `paths.db_path` (WAL-safe,
    // same pattern as the desktop AppState) to retire the redundant audit.db.
    let audit_conn = rusqlite::Connection::open(&paths.db_path)
        .map_err(|e| anyhow::anyhow!("open audit db: {e}"))?;
    let audit_logger = Arc::new(AuditLogger::new(audit_conn));

    let rollback_mgr = Arc::new(RollbackManager::new(paths.rollback_dir.clone(), 24, 512));

    let max_tool_rounds = config.agent.max_tool_rounds.max(1);

    let agent_loop = Arc::new(
        AgentLoop::new(
            model_router.clone(),
            tool_registry.clone(),
            mount_manager,
            policy_engine,
            hitl.clone(),
            audit_logger,
            rollback_mgr,
        )
        .with_max_tool_rounds(max_tool_rounds)
        .with_hardware_tier("standard"),
    );

    tracing::info!(
        max_tool_rounds,
        "[headless] minimal agent runtime constructed"
    );

    Ok(HeadlessRuntime {
        agent_loop,
        hitl,
        tool_registry,
        model_router,
    })
}
