pub(crate) mod analytics;
pub(crate) mod app_commands;
pub(crate) mod app_state;
pub(crate) mod automation;
pub(crate) mod chat;
pub(crate) mod colab;
pub(crate) mod colab_dispatch;
pub(crate) mod command_helpers;
pub(crate) mod constants;
pub(crate) mod device_enrollment;
pub(crate) mod device_tools;
pub(crate) mod document_chat;
pub(crate) mod google_workspace;
pub(crate) mod gui_automation_control;
pub(crate) mod history_helpers;
pub(crate) mod image_chat;
pub(crate) mod local_api;
pub(crate) mod mcp;
pub(crate) mod media;
pub(crate) mod openclaw;
pub(crate) mod orchestrator_helpers;
pub(crate) mod providers;
pub(crate) mod provisioning;
pub(crate) mod runtime;
pub(crate) mod runtime_status;
pub(crate) mod sessions;
pub(crate) mod telegram;
pub(crate) mod test_runner;
pub(crate) mod tool_result_helpers;
pub(crate) mod voice;
pub(crate) mod voice_diagnostics;
pub(crate) mod voice_runtime_helpers;

use colab::collect_colab_tier_status;
#[cfg(test)]
use colab::{build_colab_tier_status_payload, migrate_legacy_colab_server_command};
use colab_dispatch::*;
use command_helpers::*;
use constants::*;
use device_enrollment::*;
use device_tools::*;
#[cfg(test)]
use google_workspace::{
    build_google_workspace_status_payload, inspect_google_account_registry,
    remove_google_account_registry_entry, GoogleWorkspaceRuntimeSnapshot,
};
use history_helpers::*;
#[cfg(test)]
use local_api::{local_api_chat, LocalApiBridgeState, LocalApiChatRequest};
use local_api::{start_local_api_bridge, AgentLoopLocalApiResponder, LocalApiResponder};
use mcp::{
    apply_mcp_runtime_from_config, mcp_state_name, sync_colab_runtime_snapshot,
    sync_google_workspace_client_ref, update_mcp_health_status,
};
use orchestrator_helpers::*;
use runtime_status::collect_ironclad_status_from_parts;
use tool_result_helpers::*;
use voice_runtime_helpers::*;

#[allow(unused_imports)]
pub use analytics::get_analytics_dashboard;
#[allow(unused_imports)]
pub use app_commands::{
    approve_action, cancel_executive_task, cancel_request, cancel_turn, deny_action, get_alerts,
    get_hardware_info, get_health, get_settings, list_audio_devices, list_knowledge_base,
    list_models, submit_turn_feedback, update_settings,
};
#[allow(unused_imports)]
pub use app_state::{
    AppState, AppStateCell, ColabRuntimeSnapshot, ColabRuntimeState, FleetRuntimeState,
    McpFailureRecord,
};
#[allow(unused_imports)]
pub use automation::{
    add_scheduled_task, delete_macro, delete_workflow, list_macros, list_scheduled_tasks,
    list_workflows, remove_scheduled_task, start_macro_recording, stop_macro_recording,
};
#[allow(unused_imports)]
pub use chat::{send_lab_message, send_message};
#[allow(unused_imports)]
pub use colab::{
    connect_colab_tier, disconnect_colab_tier, get_colab_tier_status, set_colab_selected_notebook,
};
#[allow(unused_imports)]
pub use document_chat::send_document_message;
#[allow(unused_imports)]
pub use google_workspace::{
    connect_google_workspace, disconnect_google_workspace, get_google_workspace_status,
    set_google_workspace_account,
};
#[allow(unused_imports)]
pub use image_chat::send_image_message;
#[allow(unused_imports)]
pub use mcp::{
    add_mcp_server, list_mcp_servers, reconcile_mcp_runtime, remove_mcp_server,
    restart_mcp_server_runtime, toggle_mcp_server,
};
#[allow(unused_imports)]
pub use media::{
    get_session_media, open_html_for_print, read_local_image, save_export_file, save_uploaded_image,
};
#[allow(unused_imports)]
pub use openclaw::{
    clawhub_fetch_remote_skills, clawhub_install_skill, clawhub_list_skills, clawhub_search_skills,
    clawhub_toggle_skill, clawhub_uninstall_skill, openclaw_substrate_restart,
    openclaw_substrate_status,
};
#[allow(unused_imports)]
pub use providers::{
    discover_provider_models, get_active_provider, get_provider_types, list_providers,
    remove_provider, switch_model, switch_provider, test_provider_config,
    test_provider_connection_cmd, upsert_provider,
};
#[allow(unused_imports)]
pub use provisioning::{
    complete_provisioning, get_hardware_profile, get_provisioning_diagnostics,
    get_provisioning_state, run_provisioning_step, set_provisioning_backend, start_provisioning,
};
#[allow(unused_imports)]
pub use runtime::{init_runtime, shutdown_runtime};
#[allow(unused_imports)]
pub use runtime_status::{
    delete_target, get_ironclad_config, get_ironclad_forensics, get_ironclad_status,
    get_orchestrator_status, register_new_target, request_ironclad_hard_reset,
    request_ironclad_soft_reset, update_ironclad_config, update_target,
};
#[allow(unused_imports)]
pub use sessions::{
    auto_rename_session, create_session, delete_session, get_session_history, list_sessions,
    rename_session, search_sessions, switch_session,
};
#[allow(unused_imports)]
pub use telegram::{
    get_telegram_config, start_telegram_mcp, stop_telegram_mcp, test_telegram_connection,
    update_telegram_config,
};
#[allow(unused_imports)]
pub use test_runner::{
    delete_all_test_logs, delete_test_report, get_test_run_state, list_docker_containers,
    list_test_history, list_test_targets, read_test_report, start_test_run, stop_test_run,
};
#[allow(unused_imports)]
pub use voice::{get_voice_status, start_voice, stop_voice, voice_v2_abort, voice_v2_speak};
#[allow(unused_imports)]
pub use voice_diagnostics::{
    voice_transcribe_audio_file, voice_transcribe_uploaded_audio, voice_v2_status,
};

use async_stream::stream;
use async_trait::async_trait;
use axum::{
    extract::{
        ws::Message, ws::WebSocket, ws::WebSocketUpgrade, Path as AxumPath, Query,
        State as AxumState,
    },
    http::StatusCode,
    response::{sse::Event, sse::KeepAlive, sse::Sse, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use kria_connection_control::manager::{
    ControlPlaneEvent, DockerEvalRequest, DockerHealthStatus, TargetMode, TargetState,
    TerminalStream,
};
use kria_core::agent::loop_engine::{
    PromptLabToolSelectionStrategy, StreamEvent, TurnExecutionMode, TurnExecutionProfile,
};
use kria_core::agent::AgentLoop;
use kria_core::automation::{AutomationScheduler, MacroRecorder, WorkflowEngine};
use kria_core::config::{ColabConfig, KriaConfig, KriaSystemConfig};
use kria_core::image::ImageOrchestrator;
use kria_core::infra::environment::remote_qemu::{
    ControlPlaneTransport, FileCommitPolicy, GuestFilesystemPolicy, GuestOsFamily,
    HelperProvisioning, HostArtifactGcConfig, HostPlatform, InfrastructureRuntimeConfig,
    PrivilegedCommitMode, QemuSshEnvironment, RemoteConfig, ReplayCachePolicy, ResetPolicy,
    SshMultiplexingConfig, SshPoolConfig, SshTransportBackend, TargetKind,
};
use kria_core::infra::health::{HealthRegistry, ServiceStatus};
use kria_core::infra::pool::{SelectionWeights, TargetHealthTelemetry, TargetId, TargetPool};
use kria_core::infra::qos::AdaptiveQosScheduler;
use kria_core::infra::EventBus;
use kria_core::infra::ToolResult;
use kria_core::llm::model_router::RoutingMode;
use kria_core::llm::orchestrator::{Orchestrator, RemoteQemuToolBridge};
use kria_core::llm::{ChatMessage, ImageAttachment, ModelRouter};
use kria_core::mcp::client::McpServerState;
use kria_core::mcp::server_manager::McpServerStatus;
use kria_core::mcp::{build_colab_capability_summary, McpServerManager};
use kria_core::memory::embeddings::EmbeddingModel;
use kria_core::memory::vectors::VectorIndex;
use kria_core::memory::{
    ChatMediaRecord, ConversationTurn, MemoryManager, MemoryRuntime, MemoryStore, MemoryTurnWrite,
    PreferenceRecord,
};
use kria_core::platform::detect::{
    detect_hardware, get_available_package_managers, HardwareInfo, HardwareTier,
};
use kria_core::resource::GpuLeaseManager;
use kria_core::safety::hitl::{ApprovalResponse, HitlGateway};
use kria_core::safety::{AuditLogger, PolicyEngine, RiskLevel, RollbackManager};
use kria_core::sidecar::SidecarBridge;
use kria_core::tools::google_workspace as gw;
use kria_core::tools::google_workspace_contract as gw_contract;
use kria_core::tools::mount_manager;
use kria_core::tools::registry::{self, ParamDef, ToolDef, ToolHandler, ToolRegistry};
use kria_core::voice::{
    default_input_device_name, default_output_device_name, list_input_devices, list_output_devices,
    SpeechToText, TextToSpeech, VoicePipeline, VoicePipelineEvent, VoicePipelineState,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::RwLock;
use tokio::time::timeout;
use tower_http::cors::CorsLayer;
use uuid::Uuid;

use crate::device_control::DesktopFleetControlRuntime;
use kria_core::platform::telegram::TelegramBridge;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IroncladResetSnapshot {
    pub event_id: String,
    pub phase: String,
    pub reason: String,
    pub detail: String,
    pub started_unix_ms: u64,
    pub completed_unix_ms: Option<u64>,
    pub in_flight: bool,
}

impl Default for IroncladResetSnapshot {
    fn default() -> Self {
        Self {
            event_id: uuid::Uuid::new_v4().to_string(),
            phase: "idle".to_string(),
            reason: "none".to_string(),
            detail: "No reset activity recorded yet".to_string(),
            started_unix_ms: unix_now_ms(),
            completed_unix_ms: Some(unix_now_ms()),
            in_flight: false,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IroncladForensicRecord {
    pub id: String,
    pub timestamp_unix_ms: u64,
    pub category: String,
    pub severity: String,
    pub summary: String,
    pub source: String,
    pub evidence: String,
    pub last_gasp_detected: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct IroncladConfigUpdatePayload {
    pub high_recovery_slo_ms: Option<u64>,
    pub lease_ttl_ms: Option<u64>,
    pub heartbeat_grace_ms: Option<u64>,
    pub quarantine_cooldown_ms: Option<u64>,
    pub max_normalized_hash_distance: Option<f64>,
}
#[cfg(test)]
mod tests;
