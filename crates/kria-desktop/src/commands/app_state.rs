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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmRuntimeApplySnapshot {
    pub state: String,
    pub phase: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub message: String,
    pub last_error: Option<String>,
    pub updated_unix_ms: u64,
}

impl Default for LlmRuntimeApplySnapshot {
    fn default() -> Self {
        Self {
            state: "idle".to_string(),
            phase: "idle".to_string(),
            provider_id: None,
            model_id: None,
            message: "No runtime change in progress".to_string(),
            last_error: None,
            updated_unix_ms: 0,
        }
    }
}

/// Shared application state managed by Tauri.
pub struct AppState {
    pub config: Arc<RwLock<KriaConfig>>,
    /// Single source of truth for config reads/writes (settings-config-revamp).
    /// Wraps the SAME `config` handle above + the event bus, so routing through
    /// it is behaviourally identical when `KRIA_CONFIG_SERVICE` is off.
    pub config_service: Arc<kria_core::config::ConfigService>,
    /// Hash-chained audit ledger (also the durable config-change history source,
    /// settings-config-revamp Task 15). Shares the same `kria.db` used elsewhere.
    pub audit_logger: Arc<kria_core::safety::AuditLogger>,
    /// Held to keep the Arc alive for the app's lifetime.
    #[allow(dead_code)]
    pub model_router: Arc<ModelRouter>,
    pub agent_loop: Arc<AgentLoop>,
    /// Hot-swappable ExecutiveController handle. None routes through AgentLoop.
    pub executive_sender: Arc<RwLock<Option<kria_core::agent::executive::ExecutiveSender>>>,
    pub tool_registry: Arc<ToolRegistry>,
    /// Durable safety gate for generated/discovered tools awaiting review.
    pub quarantine_registry: Arc<kria_core::tools::quarantine::QuarantineRegistry>,
    pub memory_store: Arc<dyn MemoryRuntime>,
    /// New cognitive-memory conversation/session/preference store (Phase-1
    /// cutover). Consumers migrate off `memory_store` onto this incrementally.
    pub conversation: Arc<kria_core::memory::conversation::ConversationStore>,
    /// The unified cognitive Memory System (write policy, retriever, background
    /// cognition) — the central intelligence backbone. Every subsystem records
    /// observations/outcomes and retrieves context through this, never bypassing
    /// the Write Policy. Shares the single authority DB with `conversation`.
    pub memory_system: Arc<kria_core::memory::api::MemorySystem>,
    /// Desktop cognition scheduler/UI bridge. Absent while Memory is disabled.
    pub memory_cognition_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Active cold-start import cancellation handle (AUD-03 / L4). Set for the
    /// duration of an in-flight `memory_cold_start_import`; `memory_cold_start_cancel`
    /// cancels it. `None` when no import is running (single onboarding import at
    /// a time).
    pub cold_start_cancel: Arc<std::sync::Mutex<Option<tokio_util::sync::CancellationToken>>>,
    pub hitl: Arc<HitlGateway>,
    pub decision_store: Arc<kria_core::agent::collaborative_decision::DecisionStore>,
    pub policy_engine: Arc<PolicyEngine>,
    pub resume_executor: Arc<kria_core::agent::resume_executor::ResumeExecutor>,
    pub continuation_reentry:
        Arc<kria_core::agent::continuation_reentry::ContinuationReentryService>,
    pub workflow_continuation:
        Arc<kria_core::agent::workflow_continuation::WorkflowContinuationRuntime>,
    pub event_bus: Arc<EventBus>,
    /// Held to keep the sidecar process alive for the app's lifetime.
    #[allow(dead_code)]
    pub sidecar: Arc<SidecarBridge>,
    pub embeddings: Arc<EmbeddingModel>,
    pub current_session_id: Arc<RwLock<String>>,
    pub voice_active: Arc<std::sync::atomic::AtomicBool>,
    pub voice_pipeline: Arc<RwLock<Arc<VoicePipeline>>>,
    /// Engine-aware voice handle. Holds either the v1 [`VoicePipeline`]
    /// (default) or the v2 [`kria_core::voice::v2::VoicePipelineV2`] when
    /// `voice.engine = "v2"`. Existing call-sites keep using
    /// `voice_pipeline` directly; v2-aware code reads `active_voice`.
    pub active_voice: Arc<RwLock<kria_core::voice::v2::ActivePipeline>>,
    /// Telemetry receiver for the v2 pipeline (when active). `None` while
    /// running v1. Broadcast so each session can subscribe fresh (Issue 3).
    pub voice_v2_telemetry: Arc<
        tokio::sync::Mutex<
            Option<tokio::sync::broadcast::Receiver<kria_core::voice::v2::VoiceTelemetry>>,
        >,
    >,
    pub health: Arc<HealthRegistry>,
    pub scheduler: Arc<RwLock<AutomationScheduler>>,
    pub macro_recorder: Arc<RwLock<MacroRecorder>>,
    pub started_at: std::time::Instant,
    pub hardware_info: Arc<HardwareInfo>,
    pub gpu_lease: Arc<kria_core::resource::gpu_lease::GpuLeaseManager>,
    pub proactive: Arc<kria_core::automation::ProactiveEngine>,
    pub telegram_bridge: Arc<RwLock<Option<TelegramBridge>>>,
    /// MCP server manager — kept alive for background health monitoring + dynamic tool registration.
    #[allow(dead_code)]
    pub mcp_manager: Arc<tokio::sync::Mutex<McpServerManager>>,
    /// Global MCP heartbeat task. Absent when MCP master switch is off.
    pub mcp_heartbeat: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
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
    /// Startup, HRA, idle-release, and event-forwarder tasks owned by current
    /// model orchestrator. Drained on hot disable and runtime shutdown.
    pub orchestrator_tasks: Arc<tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    /// Serializes Settings-driven LLM provider/model apply operations.
    pub llm_runtime_apply_lock: Arc<tokio::sync::Mutex<()>>,
    /// Last known runtime apply/swap state, mirrored to the UI via events.
    pub llm_runtime_apply_status: Arc<RwLock<LlmRuntimeApplySnapshot>>,
    /// Number of active turn executions that currently depend on local runtime.
    pub orchestrator_active_turns: Arc<std::sync::atomic::AtomicUsize>,
    /// Last observed local-runtime activity timestamp for idle release decisions.
    pub orchestrator_last_activity_at: Arc<tokio::sync::Mutex<std::time::Instant>>,
    /// Image generation orchestrator — ComfyUI sidecar + cloud fallback.
    #[allow(dead_code)]
    pub image_orchestrator: Arc<ImageOrchestrator>,
    /// OpenClaw skill registry — SQLite-backed, populated at boot.
    pub skill_registry: Arc<kria_core::openclaw::registry::SkillRegistry>,
    /// OpenClaw container pool. Hot-swappable so enable/disable never needs a
    /// full KRIA restart and all consumers reuse the same owner.
    pub container_pool: Arc<RwLock<Option<Arc<kria_core::openclaw::ContainerPool>>>>,
    /// Serialized feature lifecycle transitions shared by Settings, prompt tools,
    /// startup reconciliation, and generic config changes.
    pub feature_controls: Arc<super::feature_controls::FeatureControlRuntime>,
    /// n8n background timeout/cleanup task. Absent while n8n is disabled.
    pub n8n_maintenance: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// n8n workflow catalog for callback validation and UI status.
    pub n8n_catalog: Arc<RwLock<Option<Arc<kria_core::n8n::N8nCatalog>>>>,
    /// n8n callback/run state. Durable replay is backed by the JSONL inbox path below.
    pub n8n_state_store: Arc<kria_core::n8n::N8nWorkflowStateStore>,
    /// JSONL callback inbox path used to replay async workflow evidence after restart.
    pub n8n_inbox_path: PathBuf,
    /// JSONL governance/audit trail for n8n decisions and reconciliation.
    pub n8n_audit_path: PathBuf,
    /// Recent n8n governance decisions for UI/debugging.
    pub n8n_governance_log: Arc<RwLock<Vec<kria_core::n8n::N8nGovernanceDecision>>>,
    /// n8n HITL responses that external workflows can poll after KRIA user approval.
    pub n8n_hitl_responses: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    /// Pending GUI Cognition HITL proposals keyed by approval request id/session.
    pub gui_cognition_hitl_proposals:
        Arc<RwLock<kria_core::agent::gui_cognition::safety_hitl::GuiHitlProposalStore>>,
    /// RFC 008 Service Orchestrator — manages vision sidecar + uinput daemon lifecycle.
    /// `None` if orchestrator failed to start (e.g. missing binaries); automation will
    /// be globally halted in that case.
    pub gui_orchestrator: Option<Arc<kria_core::orchestrator::ServiceOrchestrator>>,
    /// Batch 1: PSDG handle — persistent semantic desktop cognition graph.
    ///
    /// Provides access to `WorldModelStore` (SQLite-backed Bayesian (s,p,o) triple store)
    /// for all subsystems that need to read or write semantic desktop state.
    ///
    /// All writes are fire-and-forget (non-blocking). All reads are bounded.
    /// `None` if WorldModelStore failed to open at startup (non-fatal degradation).
    #[allow(dead_code)]
    pub world_model: Option<kria_core::agent::PsdgHandle>,
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
