use super::*;
use std::collections::HashMap;

/// OnceCell populated by init_runtime() once the full runtime is ready.
/// Managing this (not AppState) in Tauri allows commands to be registered
/// before init completes without a "state not managed" panic.
pub type AppStateCell = tokio::sync::OnceCell<AppState>;

pub struct FleetRuntimeState {
    pub target_pool: Arc<TargetPool>,
    pub system_config: KriaSystemConfig,
    pub runtime_root: PathBuf,
    pub(super) admission_lock: tokio::sync::Mutex<()>,
}

impl FleetRuntimeState {
    pub(super) fn new(
        target_pool: Arc<TargetPool>,
        system_config: KriaSystemConfig,
        runtime_root: PathBuf,
    ) -> Self {
        Self {
            target_pool,
            system_config,
            runtime_root,
            admission_lock: tokio::sync::Mutex::new(()),
        }
    }
}

/// Shared application state managed by Tauri.
pub struct AppState {
    pub config: Arc<RwLock<KriaConfig>>,
    /// Held to keep the Arc alive for the app's lifetime.
    #[allow(dead_code)]
    pub model_router: Arc<ModelRouter>,
    pub agent_loop: Arc<AgentLoop>,
    pub tool_registry: Arc<ToolRegistry>,
    pub memory_store: Arc<dyn MemoryRuntime>,
    pub hitl: Arc<HitlGateway>,
    pub event_bus: Arc<EventBus>,
    /// Held to keep the sidecar process alive for the app's lifetime.
    #[allow(dead_code)]
    pub sidecar: Arc<SidecarBridge>,
    pub embeddings: Arc<EmbeddingModel>,
    pub vectors: Arc<VectorIndex>,
    pub current_session_id: Arc<RwLock<String>>,
    pub voice_active: Arc<std::sync::atomic::AtomicBool>,
    pub voice_pipeline: Arc<RwLock<Arc<VoicePipeline>>>,
    /// Engine-aware voice handle. Holds either the v1 [`VoicePipeline`]
    /// (default) or the v2 [`kria_core::voice::v2::VoicePipelineV2`] when
    /// `voice.engine = "v2"`. Existing call-sites keep using
    /// `voice_pipeline` directly; v2-aware code reads `active_voice`.
    pub active_voice: Arc<RwLock<kria_core::voice::v2::ActivePipeline>>,
    /// Telemetry receiver for the v2 pipeline (when active). `None` while
    /// running v1. Wrapped in a Mutex so the background driver task can
    /// take it without dropping the AppState lock.
    pub voice_v2_telemetry: Arc<
        tokio::sync::Mutex<
            Option<tokio::sync::mpsc::UnboundedReceiver<kria_core::voice::v2::VoiceTelemetry>>,
        >,
    >,
    pub health: Arc<HealthRegistry>,
    pub scheduler: Arc<RwLock<AutomationScheduler>>,
    pub macro_recorder: Arc<RwLock<MacroRecorder>>,
    pub workflow_engine: Arc<RwLock<WorkflowEngine>>,
    pub started_at: std::time::Instant,
    pub hardware_info: Arc<HardwareInfo>,
    pub proactive: Arc<kria_core::automation::ProactiveEngine>,
    pub telegram_bridge: Arc<RwLock<Option<TelegramBridge>>>,
    /// MCP server manager — kept alive for background health monitoring + dynamic tool registration.
    #[allow(dead_code)]
    pub mcp_manager: Arc<tokio::sync::Mutex<McpServerManager>>,
    /// Lazy Google Workspace MCP client reference used by gw_* tool handlers.
    pub gw_client_ref: gw::GwClientRef,
    /// Colab cloud-tier runtime status surface.
    pub colab_runtime: Arc<RwLock<ColabRuntimeSnapshot>>,
    /// Rolling MCP runtime failures per connector for operator diagnostics.
    pub mcp_failure_history: Arc<RwLock<HashMap<String, Vec<McpFailureRecord>>>>,
    /// Latest reset lifecycle snapshot for operator visibility.
    pub ironclad_reset: Arc<RwLock<IroncladResetSnapshot>>,
    /// In-memory rolling forensic audit feed for trust-first diagnostics.
    pub ironclad_forensic_log: Arc<RwLock<Vec<IroncladForensicRecord>>>,
    /// Fleet runtime admission state for TargetPool-backed remote targets.
    pub fleet_runtime: Arc<FleetRuntimeState>,
    /// Connection-control runtime hydrated from enrollment registry for fleet control-plane telemetry.
    pub fleet_control_runtime: Arc<DesktopFleetControlRuntime>,
    /// Hardware orchestrator — manages llama-server lifecycle and dynamic GPU offloading.
    /// Wrapped in RwLock so the background startup task can populate it after AppState
    /// is set, keeping the main init path non-blocking.
    #[allow(dead_code)]
    pub orchestrator: Arc<tokio::sync::RwLock<Option<Arc<Orchestrator>>>>,
    /// Number of active turn executions that currently depend on local runtime.
    pub orchestrator_active_turns: Arc<std::sync::atomic::AtomicUsize>,
    /// Last observed local-runtime activity timestamp for idle release decisions.
    pub orchestrator_last_activity_at: Arc<tokio::sync::Mutex<std::time::Instant>>,
    /// Image generation orchestrator — ComfyUI sidecar + cloud fallback.
    #[allow(dead_code)]
    pub image_orchestrator: Arc<ImageOrchestrator>,
    /// OpenClaw skill registry — SQLite-backed, populated at boot.
    pub skill_registry: Arc<kria_core::openclaw::registry::SkillRegistry>,
    /// OpenClaw container pool — None if Docker is unavailable (graceful degradation).
    pub container_pool: Option<Arc<kria_core::openclaw::ContainerPool>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpFailureRecord {
    pub timestamp_unix_ms: u64,
    pub state: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColabRuntimeState {
    Disconnected,
    SidecarStarting,
    AwaitingBrowserConnection,
    NotebookSelectionRequired,
    Ready,
    Degraded,
}

impl ColabRuntimeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::SidecarStarting => "sidecar_starting",
            Self::AwaitingBrowserConnection => "awaiting_browser_connection",
            Self::NotebookSelectionRequired => "notebook_selection_required",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ColabRuntimeSnapshot {
    pub state: ColabRuntimeState,
    pub sidecar_server_name: String,
    pub selected_notebook: Option<String>,
    pub last_error: Option<String>,
}

impl ColabRuntimeSnapshot {
    pub(super) fn new(state: ColabRuntimeState, sidecar_server_name: String) -> Self {
        Self {
            state,
            sidecar_server_name,
            selected_notebook: None,
            last_error: None,
        }
    }
}
