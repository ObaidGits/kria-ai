#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod device_control;
mod tray;

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

static RUNTIME_SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

fn main() {
    // Ring 4 — install Linux seccomp-BPF filter before anything else.
    // On non-Linux platforms this is a no-op.
    if let Err(e) = kria_core::platform::install_seccomp_filter() {
        eprintln!("[WARN] seccomp filter not installed: {e}");
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // Register the AppStateCell immediately so Tauri never panics with
            // "state not managed" — commands that arrive before init_runtime()
            // finishes will get a clean "still initializing" error instead.
            app.handle().manage(commands::AppStateCell::new());

            // Initialize tray icon
            tray::create_tray(app.handle())?;

            // Initialize kria-core runtime in background
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = commands::init_runtime(&handle).await {
                    tracing::error!("failed to initialize KRIA runtime: {e}");
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::chat::send_message,
            commands::chat::send_manual_tool_message,
            commands::chat::send_lab_message,
            commands::sessions::get_session_history,
            commands::sessions::create_session,
            commands::sessions::list_sessions,
            commands::sessions::switch_session,
            commands::sessions::delete_session,
            commands::sessions::rename_session,
            commands::sessions::auto_rename_session,
            commands::sessions::search_sessions,
            commands::app_commands::cancel_request,
            commands::app_commands::cancel_turn,
            commands::app_commands::cancel_executive_task,
            commands::app_commands::submit_turn_feedback,
            commands::app_commands::approve_action,
            commands::app_commands::deny_action,
            commands::app_commands::list_interaction_decisions,
            commands::app_commands::resolve_interaction_decision,
            commands::app_commands::resume_interaction_decision,
            commands::app_commands::execute_resolved_interaction_decision,
            commands::app_commands::cancel_interaction_execution,
            commands::app_commands::check_continuation_after_decision,
            commands::app_commands::continue_after_decision_execution,
            commands::app_commands::cancel_continuation,
            commands::app_commands::cancel_interaction_decision,
            commands::app_commands::replay_interaction_decisions,
            commands::app_commands::get_health,
            commands::app_commands::get_runtime_diagnostics,
            commands::app_commands::get_settings,
            commands::app_commands::list_audio_devices,
            commands::app_commands::update_settings,
            commands::app_commands::list_models,
            commands::voice::start_voice,
            commands::voice::stop_voice,
            commands::voice::get_voice_status,
            commands::voice::voice_v2_speak,
            commands::voice::voice_v2_abort,
            commands::image_chat::send_image_message,
            commands::document_chat::send_document_message,
            commands::mcp::list_mcp_servers,
            commands::mcp::reconcile_mcp_runtime,
            commands::mcp::add_mcp_server,
            commands::mcp::remove_mcp_server,
            commands::mcp::toggle_mcp_server,
            commands::mcp::restart_mcp_server_runtime,
            commands::telegram::get_telegram_config,
            commands::telegram::update_telegram_config,
            commands::telegram::start_telegram_mcp,
            commands::telegram::stop_telegram_mcp,
            commands::telegram::test_telegram_connection,
            commands::automation::list_scheduled_tasks,
            commands::automation::add_scheduled_task,
            commands::automation::remove_scheduled_task,
            commands::automation::list_macros,
            commands::automation::start_macro_recording,
            commands::automation::stop_macro_recording,
            commands::automation::delete_macro,
            commands::automation::list_workflows,
            commands::automation::delete_workflow,
            commands::app_commands::get_hardware_info,
            commands::app_commands::list_knowledge_base,
            commands::app_commands::get_alerts,
            commands::media::save_export_file,
            commands::media::open_html_for_print,
            commands::media::read_local_image,
            commands::media::save_uploaded_image,
            commands::media::get_session_media,
            commands::n8n::get_n8n_status,
            commands::n8n::get_n8n_runtime_status,
            commands::n8n::save_n8n_settings,
            commands::n8n::save_n8n_api_key_secret,
            commands::n8n::detect_n8n_connection_candidates,
            commands::n8n::test_n8n_connection_profile,
            commands::n8n::repair_n8n_connection,
            commands::n8n::start_or_prepare_managed_n8n,
            commands::n8n::test_n8n_connection,
            commands::n8n::start_managed_n8n,
            commands::n8n::stop_managed_n8n,
            commands::n8n::restart_managed_n8n,
            commands::n8n::open_n8n_dashboard,
            commands::n8n::discover_n8n_workflows,
            commands::n8n::discover_n8n_runtime_profile_drafts,
            commands::n8n::get_n8n_runtime_profiles,
            commands::n8n::save_n8n_runtime_profile_draft,
            commands::n8n::delete_n8n_runtime_profile,
            commands::n8n::refresh_n8n_runtime_profile_draft,
            commands::n8n::analyze_n8n_workflow_input_capability,
            commands::n8n::analyze_n8n_code_nodes,
            commands::n8n::analyze_n8n_v5_workflow_inputs,
            commands::n8n::generate_n8n_binary_input_copy_preview,
            commands::n8n::generate_n8n_code_patch_preview,
            commands::n8n::create_n8n_input_aware_copy,
            commands::n8n::create_n8n_binary_input_aware_copy,
            commands::n8n::create_n8n_code_input_aware_copy,
            commands::n8n::test_n8n_input_aware_copy,
            commands::n8n::test_n8n_binary_input_aware_copy,
            commands::n8n::test_n8n_code_input_aware_copy,
            commands::n8n::save_n8n_preferred_output_node,
            commands::n8n::audit_n8n_workflow_lifecycle,
            commands::n8n::get_n8n_copy_lifecycle_items,
            commands::n8n::refresh_n8n_lifecycle_item,
            commands::n8n::continue_n8n_pending_copy_operation,
            commands::n8n::cleanup_n8n_generated_copy,
            commands::n8n::enrich_n8n_runtime_profile_payload,
            commands::n8n::enrich_n8n_runtime_profile_draft,
            commands::n8n::enrich_n8n_runtime_profile_drafts,
            commands::n8n::save_n8n_profile_as_workflow_draft,
            commands::n8n::archive_legacy_n8n_toml_workflows,
            commands::n8n::import_n8n_workflow,
            commands::n8n::update_n8n_workflow_metadata,
            commands::n8n::reconcile_n8n_run,
            commands::n8n::approve_n8n_workflow,
            commands::n8n::disable_n8n_workflow,
            commands::n8n::delete_n8n_workflow,
            commands::n8n::remove_sample_n8n_workflows,
            commands::n8n::list_n8n_executions,
            commands::n8n::list_n8n_workflow_executions,
            commands::n8n::view_n8n_workflow_execution,
            commands::n8n::resume_n8n_waiting_execution,
            commands::n8n::suggest_n8n_workflows,
            commands::n8n::prepare_n8n_workflow_input,
            commands::n8n::invoke_n8n_workflow_from_ui,
            commands::n8n::validate_n8n_workflow_draft,
            commands::n8n::dry_run_n8n_workflow_validation,
            commands::n8n::backup_n8n_workflow,
            commands::n8n::rollback_n8n_workflow_backup,
            commands::n8n::create_or_update_n8n_workflow_draft,
            commands::colab::get_colab_tier_status,
            commands::colab::connect_colab_tier,
            commands::colab::disconnect_colab_tier,
            commands::colab::set_colab_selected_notebook,
            commands::google_workspace::get_google_workspace_status,
            commands::google_workspace::set_google_workspace_account,
            commands::google_workspace::connect_google_workspace,
            commands::google_workspace::disconnect_google_workspace,
            commands::runtime_status::get_orchestrator_status,
            commands::runtime_status::register_new_target,
            commands::runtime_status::delete_target,
            commands::runtime_status::update_target,
            commands::runtime_status::get_ironclad_status,
            commands::runtime_status::get_ironclad_forensics,
            commands::runtime_status::request_ironclad_soft_reset,
            commands::runtime_status::request_ironclad_hard_reset,
            commands::runtime_status::get_ironclad_config,
            commands::runtime_status::update_ironclad_config,
            commands::test_runner::start_test_run,
            commands::test_runner::stop_test_run,
            commands::test_runner::get_test_run_state,
            commands::test_runner::list_test_history,
            commands::test_runner::list_docker_containers,
            commands::test_runner::list_test_targets,
            commands::test_runner::read_test_report,
            commands::test_runner::delete_test_report,
            commands::test_runner::delete_all_test_logs,
            commands::analytics::get_analytics_dashboard,
            // Provisioning (first-boot setup wizard)
            commands::provisioning::get_provisioning_state,
            commands::provisioning::start_provisioning,
            commands::provisioning::complete_provisioning,
            commands::provisioning::set_provisioning_backend,
            commands::provisioning::run_provisioning_step,
            commands::provisioning::get_provisioning_diagnostics,
            commands::provisioning::get_hardware_profile,
            commands::voice_diagnostics::voice_v2_status,
            commands::voice_diagnostics::voice_transcribe_audio_file,
            commands::voice_diagnostics::voice_transcribe_uploaded_audio,
            commands::openclaw::clawhub_list_skills,
            commands::openclaw::clawhub_search_skills,
            commands::openclaw::clawhub_fetch_remote_skills,
            commands::openclaw::clawhub_install_skill,
            commands::openclaw::clawhub_uninstall_skill,
            commands::openclaw::clawhub_toggle_skill,
            commands::openclaw::openclaw_substrate_status,
            commands::openclaw::openclaw_substrate_restart,
            commands::gui_automation_control::get_gui_automation_status,
            commands::gui_automation_control::set_gui_automation_enabled,
            commands::gui_automation_control::get_grounding_status,
            // Universal Model Provider System
            commands::providers::list_providers,
            commands::providers::get_active_provider,
            commands::providers::get_active_llm_runtime,
            commands::providers::get_llm_runtime_apply_status,
            commands::providers::set_active_llm_selection,
            commands::providers::switch_provider,
            commands::providers::switch_model,
            commands::providers::test_provider_connection_cmd,
            commands::providers::test_provider_config,
            commands::providers::discover_provider_models,
            commands::providers::upsert_provider,
            commands::providers::remove_provider,
            commands::providers::get_provider_types,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        if let tauri::RunEvent::ExitRequested { .. } = event {
            if RUNTIME_SHUTDOWN_REQUESTED.swap(true, Ordering::SeqCst) {
                return;
            }

            tauri::async_runtime::block_on(commands::shutdown_runtime(app_handle));
        }
    });
}
