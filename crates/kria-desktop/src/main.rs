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
            commands::app_commands::approve_action,
            commands::app_commands::deny_action,
            commands::app_commands::get_health,
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
            commands::runtime_status::get_ironclad_status,
            commands::runtime_status::get_ironclad_forensics,
            commands::runtime_status::request_ironclad_soft_reset,
            commands::runtime_status::request_ironclad_hard_reset,
            commands::runtime_status::get_ironclad_config,
            commands::runtime_status::update_ironclad_config,
            // Provisioning (first-boot setup wizard)
            commands::provisioning::get_provisioning_state,
            commands::provisioning::start_provisioning,
            commands::provisioning::complete_provisioning,
            commands::provisioning::set_provisioning_backend,
            commands::provisioning::run_provisioning_step,
            commands::provisioning::get_provisioning_diagnostics,
            commands::provisioning::get_hardware_profile,
            commands::voice_diagnostics::voice_v2_status,
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
