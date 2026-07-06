pub(crate) mod analytics;
pub(crate) mod api_auth;
pub(crate) mod api_hitl;
pub(crate) mod app_commands;
pub(crate) mod app_state;
pub(crate) mod automation;
pub(crate) mod briefing;
pub(crate) mod capability;
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
pub(crate) mod gui_cognition;
pub(crate) mod history_helpers;
pub(crate) mod image_chat;
pub(crate) mod local_api;
pub(crate) mod mcp;
pub(crate) mod media;
pub(crate) mod mobile_gateway;
pub(crate) mod n8n;
pub(crate) mod openclaw;
pub(crate) mod orchestrator_helpers;
pub(crate) mod providers;
pub(crate) mod provisioning;
pub(crate) mod runtime;
pub(crate) mod runtime_status;
pub(crate) mod sessions;
pub(crate) mod tasks;
pub(crate) mod telegram;
pub(crate) mod test_runner;
pub(crate) mod tool_result_helpers;
pub(crate) mod voice;
pub(crate) mod voice_diagnostics;
pub(crate) mod voice_runtime_helpers;
pub(crate) mod wake_listener;
pub(crate) mod workflow;

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
    approve_action, cancel_continuation, cancel_executive_task, cancel_interaction_decision,
    cancel_interaction_execution, cancel_request, cancel_turn, check_continuation_after_decision,
    continue_after_decision_execution, deny_action, execute_resolved_interaction_decision,
    get_alerts, get_hardware_info, get_health, get_runtime_diagnostics, get_settings,
    list_audio_devices, list_interaction_decisions, list_knowledge_base, list_models,
    replay_interaction_decisions, resolve_interaction_decision, resume_interaction_decision,
    submit_turn_feedback, update_settings,
};
#[allow(unused_imports)]
pub use app_state::{
    AppState, AppStateCell, ColabRuntimeSnapshot, ColabRuntimeState, FleetRuntimeState,
    LlmRuntimeApplySnapshot, McpFailureRecord,
};
#[allow(unused_imports)]
pub use automation::{
    add_scheduled_task, delete_macro, delete_workflow, list_macros, list_scheduled_tasks,
    list_workflows, remove_scheduled_task, start_macro_recording, stop_macro_recording,
};
#[allow(unused_imports)]
pub use chat::{send_lab_message, send_manual_tool_message, send_message};
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
pub use mobile_gateway::{
    get_mobile_config, mobile_begin_pairing, mobile_gateway_start, mobile_gateway_status,
    mobile_gateway_stop, mobile_list_devices, mobile_revoke_device, remote_desktop_kill,
    remote_desktop_status, set_mobile_config,
};
#[allow(unused_imports)]
pub use n8n::{
    analyze_n8n_code_nodes, analyze_n8n_v5_workflow_inputs, analyze_n8n_workflow_authoring_request,
    analyze_n8n_workflow_input_capability, apply_n8n_workflow_update_after_confirmation,
    approve_n8n_workflow_draft, archive_legacy_n8n_toml_workflows, archive_n8n_workflow,
    audit_n8n_workflow_lifecycle, cleanup_n8n_generated_copy, cleanup_n8n_workflow_draft,
    continue_n8n_pending_copy_operation, continue_n8n_workflow_authoring_operation,
    continue_n8n_workflow_crud_operation, create_n8n_binary_input_aware_copy,
    create_n8n_code_input_aware_copy, create_n8n_input_aware_copy,
    create_n8n_workflow_draft_in_n8n, create_n8n_workflow_updated_copy, delete_n8n_runtime_profile,
    delete_n8n_workflow_permanently, detect_n8n_connection_candidates,
    discover_n8n_runtime_profile_drafts, discover_n8n_workflows, enrich_n8n_runtime_profile_draft,
    enrich_n8n_runtime_profile_drafts, enrich_n8n_runtime_profile_payload,
    export_n8n_production_audit_bundle, generate_n8n_binary_input_copy_preview,
    generate_n8n_code_patch_preview, generate_n8n_workflow_draft_plan,
    get_n8n_copy_lifecycle_items, get_n8n_production_audit_summary, get_n8n_runtime_profiles,
    get_n8n_runtime_status, get_n8n_status, get_n8n_workflow_authoring_sessions,
    get_n8n_workflow_crud_operations, import_n8n_workflow, invoke_n8n_workflow_from_ui,
    list_archived_n8n_workflows, list_n8n_credential_summaries, list_n8n_workflow_executions,
    open_n8n_dashboard, prepare_n8n_workflow_input, preview_n8n_workflow_update_diff,
    reconcile_n8n_run, refresh_n8n_lifecycle_item, refresh_n8n_runtime_profile_draft,
    reject_n8n_workflow_draft, remove_n8n_workflow_from_kria, remove_sample_n8n_workflows,
    repair_n8n_audit_finding, repair_n8n_connection, restart_managed_n8n, restore_n8n_workflow,
    restore_n8n_workflow_from_backup, resume_n8n_waiting_execution,
    rollback_n8n_workflow_authoring_update, route_n8n_chat_prompt, run_n8n_production_audit,
    save_n8n_api_key_secret, save_n8n_authoring_credential_mapping, save_n8n_preferred_output_node,
    save_n8n_profile_as_workflow_draft, save_n8n_runtime_profile_draft, save_n8n_settings,
    start_managed_n8n, start_or_prepare_managed_n8n, stop_managed_n8n,
    test_n8n_binary_input_aware_copy, test_n8n_code_input_aware_copy, test_n8n_connection,
    test_n8n_connection_profile, test_n8n_input_aware_copy, test_n8n_workflow_draft,
    update_n8n_workflow_metadata, view_n8n_workflow_execution,
};
#[allow(unused_imports)]
pub use openclaw::{
    clawhub_fetch_remote_skills, clawhub_install_skill, clawhub_list_skills, clawhub_search_skills,
    clawhub_toggle_skill, clawhub_uninstall_skill, install_skill_bundle, openclaw_capability_graph,
    openclaw_capability_manager, openclaw_execution_logs, openclaw_generate_skill,
    openclaw_get_developer_mode, openclaw_get_settings, openclaw_list_grants,
    openclaw_recommend_skills, openclaw_revoke_grant, openclaw_set_developer_mode,
    openclaw_substrate_restart, openclaw_substrate_status, openclaw_update_settings,
    uninstall_skill_bundle,
};
#[allow(unused_imports)]
pub use providers::{
    discover_provider_models, get_active_llm_runtime, get_active_provider,
    get_llm_runtime_apply_status, get_provider_types, list_providers, remove_provider,
    set_active_llm_selection, switch_model, switch_provider, test_provider_config,
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
    delete_target, get_hra_diagnostics, get_ironclad_config, get_ironclad_forensics,
    get_ironclad_status, get_orchestrator_status, register_new_target, request_ironclad_hard_reset,
    request_ironclad_soft_reset, update_ironclad_config, update_target,
};
#[allow(unused_imports)]
pub use sessions::{
    auto_rename_session, clear_all_chat_sessions, create_session, delete_session,
    get_memory_enabled, get_session_history, list_sessions, rename_session, search_sessions,
    set_memory_enabled, set_session_archived, set_session_pinned, set_session_temporary,
    switch_session,
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
pub use voice::{
    get_voice_status, start_voice, stop_voice, voice_ptt_release, voice_v2_abort, voice_v2_speak,
};
#[allow(unused_imports)]
pub use voice_diagnostics::{
    voice_transcribe_audio_file, voice_transcribe_uploaded_audio, voice_turn_diagnostics,
    voice_v2_status,
};

use async_stream::stream;
use async_trait::async_trait;
use axum::{
    body::Bytes,
    extract::{
        ws::Message, ws::WebSocket, ws::WebSocketUpgrade, Path as AxumPath, Query,
        State as AxumState,
    },
    http::{HeaderMap, StatusCode},
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
use kria_core::automation::{AutomationScheduler, MacroRecorder};
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
