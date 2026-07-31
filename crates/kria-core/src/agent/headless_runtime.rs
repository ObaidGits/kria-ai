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
use crate::memory::api::{MemoryConfig, MemorySystem};
use crate::memory::conversation::ConversationStore;
use crate::memory::embedding::OnnxEmbedder;
use crate::memory::stores::ports::Embedder;
use crate::memory::{KriaMemoryRuntime, MemoryRuntime};
use crate::safety::hitl::HitlGateway;
use crate::safety::{AuditLogger, PolicyEngine, RollbackManager};
use crate::tools::mount_manager::ToolMountManager;
use crate::tools::registry::{
    build_registry_full_with_memory, build_registry_with_store, ToolRegistry,
};

/// Default HITL approval timeout for headless hosts (seconds).
const HEADLESS_HITL_TIMEOUT_SECS: u64 = 60;

/// Handles produced by [`build_minimal`], so the host can wire the loop and
/// resolve HITL approvals (e.g. from WebSocket `approve`/`deny` frames).
pub struct HeadlessRuntime {
    pub agent_loop: Arc<AgentLoop>,
    pub hitl: Arc<HitlGateway>,
    pub tool_registry: Arc<ToolRegistry>,
    pub model_router: Arc<ModelRouter>,
    /// The unified cognitive [`MemorySystem`] over the shared authority DB.
    /// `Some` whenever the embedder loaded (the common case); the server threads
    /// this into its `ServerState`, `/memory/*` routes, and cognition scheduler
    /// so desktop + server share ONE memory architecture. `None` only if the
    /// MiniLM model is unavailable, in which case the loop degrades to no-memory
    /// rather than failing to boot.
    pub memory_system: Option<Arc<MemorySystem>>,
    /// Conversation/session store over the SAME single authority `Database`
    /// handle opened for this runtime (F1.2.4 — one authority per process).
    /// Vended here so the host adapter (server) reuses this handle for session
    /// history instead of independently re-opening the authority — even in the
    /// degraded no-embedder path where `memory_system` is `None`. `None` only if
    /// the authority DB itself could not be opened.
    pub session_store: Option<Arc<ConversationStore>>,
}

/// Build a minimal but functional agent loop from configuration.
///
/// Persists audit/rollback under the standard KRIA data paths. Returns an
/// `Arc<AgentLoop>` ready to `run`/`run_with_profile`.
pub fn build_minimal(config: &KriaConfig) -> anyhow::Result<HeadlessRuntime> {
    let paths = crate::platform::paths::KriaPaths::resolve();

    let model_router = Arc::new(ModelRouter::from_config(config));

    // ── Unified cognitive Memory System over the shared authority DB ──────────
    // The server opens the SAME `kria_memory.db` the desktop uses. When the
    // MiniLM embedder loads, we build the full MemorySystem (Write Policy,
    // retriever, graph, planner/goal/reasoning, cognition) and a memory-backed
    // tool registry — identical to the desktop path. If the embedder is
    // unavailable we degrade to the no-memory core registry so the server still
    // boots. There is NO separate server database or retrieval pipeline.
    let memory_db_path = paths.data_dir.join("kria_memory.db");
    #[allow(clippy::type_complexity)]
    let (tool_registry, memory_system, session_store): (
        Arc<ToolRegistry>,
        Option<Arc<MemorySystem>>,
        Option<Arc<ConversationStore>>,
    ) = match KriaMemoryRuntime::open(&memory_db_path) {
        Ok(backend) => {
            let backend = Arc::new(backend);
            let store: Arc<dyn MemoryRuntime> = backend.clone();
            // Conversation store over the SINGLE authority handle this
            // backend opened — reused by the server for session history so
            // the process never opens a second `Database` (F1.2.4).
            let session_store = Some(Arc::new(backend.conversation()));
            match OnnxEmbedder::new_minilm() {
                Ok(embedder) => {
                    let embedder: Arc<dyn Embedder> = Arc::new(embedder);
                    // Wire to the ONE memory composition root
                    // (`MemorySystem::compose`, design §19.1 / F1.2.4) over the
                    // SAME injected authority handle the backend opened. Because
                    // the handle is injected, `MemoryConfig.db_path` is unused
                    // (it only applies to the standalone self-opening path), so
                    // we let it default instead of duplicating the path.
                    match MemorySystem::compose(
                        backend.database(),
                        MemoryConfig {
                            device_id: "local-server".to_string(),
                            ..Default::default()
                        },
                        embedder,
                        true,
                    ) {
                        Ok(ms) => {
                            let reg = build_registry_full_with_memory(
                                Some(store),
                                None,
                                None,
                                None,
                                Some(ms.clone()),
                            );
                            tracing::info!(
                                "[headless] MemorySystem online — server is memory-driven"
                            );
                            (Arc::new(reg), Some(ms), session_store)
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "[headless] MemorySystem unavailable; no-memory registry");
                            (
                                Arc::new(build_registry_with_store(Some(store))),
                                None,
                                session_store,
                            )
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "[headless] embedder unavailable; no-memory registry");
                    (
                        Arc::new(build_registry_with_store(Some(store))),
                        None,
                        session_store,
                    )
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "[headless] memory backend unavailable; core registry only");
            (Arc::new(build_registry_with_store(None)), None, None)
        }
    };

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

    let mut loop_builder = AgentLoop::new(
        model_router.clone(),
        tool_registry.clone(),
        mount_manager,
        policy_engine,
        hitl.clone(),
        audit_logger,
        rollback_mgr,
    )
    .with_max_tool_rounds(max_tool_rounds)
    .with_hardware_tier("standard");

    // Make the loop memory-driven (grounding + observe + learning) — identical
    // to the desktop wiring.
    if let Some(ms) = &memory_system {
        loop_builder = loop_builder.with_memory_system(ms.clone());
    }

    let agent_loop = Arc::new(loop_builder);

    tracing::info!(
        max_tool_rounds,
        memory = memory_system.is_some(),
        "[headless] minimal agent runtime constructed"
    );

    Ok(HeadlessRuntime {
        agent_loop,
        hitl,
        tool_registry,
        model_router,
        memory_system,
        session_store,
    })
}
