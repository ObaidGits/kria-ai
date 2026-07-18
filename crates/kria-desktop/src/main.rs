#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod device_control;
mod safe_mode;
mod summon;
mod tray;
mod windows;

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

static RUNTIME_SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

fn main() {
    // Establish the Linux rendering baseline (kria-ui-redesign task 0.6) BEFORE
    // the webview initializes. On WebKitGTK — notably NVIDIA under Wayland (this
    // project's target hardware) — the DMABUF/accelerated-compositing paths can
    // paint a BLANK white WebView or crash. `establish_baseline` detects a
    // problematic env, resolves safe-mode (`--safe-mode` / `KRIA_SAFE_MODE`),
    // and sets the appropriate WEBKIT_DISABLE_* flags without ever clobbering an
    // explicit user value. No-op on non-Linux platforms. See
    // docs/LINUX_GRAPHICS.md for the user-facing guidance.
    #[cfg(target_os = "linux")]
    let boot_env = safe_mode::establish_baseline();

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
            // Create the primary webview once instead of declaring it in
            // `tauri.conf.json`. On Linux with a GTK application ID, a second
            // desktop activation asks Tauri to apply declarative windows again;
            // that panics when `main` already exists. Setup runs once, so this
            // keeps launcher/summon reactivation non-destructive while App URLs
            // still resolve to Vite in dev and embedded assets in bundles.
            if app.get_webview_window("main").is_none() {
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("K.R.I.A.")
                    .inner_size(900.0, 700.0)
                    .min_inner_size(400.0, 500.0)
                    .resizable(true)
                    .decorations(true)
                    .center()
                    .build()?;
            }

            // Register the AppStateCell immediately so Tauri never panics with
            // "state not managed" — commands that arrive before init_runtime()
            // finishes will get a clean "still initializing" error instead.
            app.handle().manage(commands::AppStateCell::new());
            app.handle().manage(tray::TrayPresentationState::default());
            app.handle().manage(windows::WindowPresentationState::default());

            // Tray is an enhancement, never a startup dependency. GNOME/KDE
            // Wayland sessions may have no StatusNotifier/AppIndicator host;
            // in-app Core, Approval Center, palette, and Mini remain available.
            if let Err(error) = tray::create_tray(app.handle()) {
                tracing::warn!("tray unavailable ({error}); in-app fallbacks remain active");
            }

            // Register the global summon hotkey (kria-ui-redesign task 2.5,
            // Req 2.5/18.2). ENHANCEMENT ONLY — try/degrade: if the OS/DE
            // refuses a system-wide hotkey (e.g. Wayland) this logs and
            // continues; the in-app Ctrl/Cmd+K webview hotkey is the guaranteed
            // fallback, so summon never breaks.
            let _global_summon = summon::register_global_summon(app.handle());

            // Wave 9 warm path: bind the wake-daemon IPC socket so an optional
            // running kria-wake-daemon can start a session without relaunch.
            commands::wake_listener::spawn(app.handle().clone());

            // UI event-forwarding fix (R16, product gap 5/8): bridge the real
            // OpenClaw bundle-lifecycle + execution event streams to the
            // frontend. Safe to start immediately (subscribes to broadcast
            // buses that exist regardless of whether OpenClaw is enabled yet).
            commands::openclaw::spawn_openclaw_event_forwarding(app.handle().clone());

            // Task 13.2: buffer the same OpenClaw execution + bundle-lifecycle
            // event streams into an in-memory ring the `openclaw_execution_logs`
            // command reads. Additive subscription (own broadcast receivers).
            commands::openclaw::spawn_openclaw_log_buffer();

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
            commands::sessions::branch_session,
            commands::sessions::list_sessions,
            commands::sessions::switch_session,
            commands::sessions::delete_session,
            commands::sessions::clear_all_chat_sessions,
            commands::sessions::rename_session,
            commands::sessions::auto_rename_session,
            commands::sessions::search_sessions,
            commands::sessions::set_session_pinned,
            commands::sessions::set_session_archived,
            commands::sessions::set_session_temporary,
            commands::sessions::get_memory_enabled,
            commands::sessions::set_memory_enabled,
            commands::memory::memory_search,
            commands::memory::memory_recall,
            commands::memory::memory_reason,
            commands::memory::memory_health,
            commands::memory::memory_metrics,
            commands::memory::memory_remember,
            commands::memory::memory_update,
            commands::memory::memory_verify,
            commands::memory::memory_forget,
            commands::memory::memory_hard_delete,
            commands::memory::memory_resolve_entities,
            commands::memory::memory_record_feedback,
            commands::memory::memory_correct,
            commands::memory::memory_restore_forgotten,
            commands::memory::memory_reflect,
            commands::memory::memory_consolidate,
            commands::memory::memory_run_dream,
            commands::memory::memory_run_active_learning,
            commands::memory::memory_run_self_improvement,
            commands::memory::memory_run_entity_extraction,
            commands::memory::memory_library_list,
            commands::memory::memory_library_ingest,
            commands::memory::memory_library_delete,
            commands::memory::memory_timeline,
            commands::memory::memory_meta,
            commands::memory::memory_goals_list,
            commands::memory::memory_goal_create,
            commands::memory::memory_goal_set_status,
            commands::memory::memory_plans_analytics,
            commands::memory::memory_plans_for,
            commands::memory::memory_reasoning_analytics,
            commands::memory::memory_reasoning_history,
            commands::memory::memory_causal_effects_of,
            commands::memory::memory_causal_causes_of,
            commands::memory::memory_causal_chains,
            commands::memory::memory_graph_centrality,
            commands::memory::memory_graph_communities,
            commands::memory::memory_graph_neighbors,
            commands::memory::memory_graph_relationships,
            commands::memory::memory_graph_search,
            commands::memory::memory_graph_predict_links,
            commands::memory::memory_graph_create_relationship,
            commands::memory::memory_explain,
            commands::memory::memory_backup,
            commands::memory::memory_restore,
            commands::memory::memory_health_report,
            commands::memory::memory_reasoning_replay,
            commands::memory::memory_cold_start_status,
            commands::memory::memory_cold_start_preview,
            commands::memory::memory_cold_start_import,
            commands::memory::memory_cold_start_cancel,
            commands::memory::memory_cold_start_set,
            commands::memory::memory_cold_start_complete,
            commands::app_commands::cancel_request,
            commands::app_commands::cancel_turn,
            commands::app_commands::get_executive_snapshot,
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
            commands::app_commands::patch_config,
            commands::app_commands::get_config_schema,
            commands::app_commands::get_config_history,
            commands::config_prompt::config_prompt,
            commands::app_commands::list_audio_devices,
            commands::app_commands::update_settings,
            commands::app_commands::list_models,
            commands::voice::start_voice,
            commands::voice::stop_voice,
            commands::voice::get_voice_status,
            commands::voice::voice_v2_speak,
            commands::voice::voice_v2_abort,
            commands::voice::voice_ptt_release,
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
            commands::mobile_gateway::mobile_gateway_status,
            commands::mobile_gateway::mobile_gateway_start,
            commands::mobile_gateway::mobile_gateway_stop,
            commands::mobile_gateway::mobile_begin_pairing,
            commands::mobile_gateway::mobile_list_devices,
            commands::mobile_gateway::mobile_revoke_device,
            commands::mobile_gateway::remote_desktop_status,
            commands::mobile_gateway::remote_desktop_kill,
            commands::mobile_gateway::get_mobile_config,
            commands::mobile_gateway::set_mobile_config,
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
            commands::n8n::run_n8n_production_audit,
            commands::n8n::get_n8n_production_audit_summary,
            commands::n8n::export_n8n_production_audit_bundle,
            commands::n8n::repair_n8n_audit_finding,
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
            commands::n8n::archive_n8n_workflow,
            commands::n8n::restore_n8n_workflow,
            commands::n8n::list_archived_n8n_workflows,
            commands::n8n::remove_n8n_workflow_from_kria,
            commands::n8n::delete_n8n_workflow_permanently,
            commands::n8n::restore_n8n_workflow_from_backup,
            commands::n8n::get_n8n_workflow_crud_operations,
            commands::n8n::continue_n8n_workflow_crud_operation,
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
            commands::n8n::route_n8n_chat_prompt,
            commands::n8n::prepare_n8n_workflow_input,
            commands::n8n::invoke_n8n_workflow_from_ui,
            commands::n8n::validate_n8n_workflow_draft,
            commands::n8n::dry_run_n8n_workflow_validation,
            commands::n8n::backup_n8n_workflow,
            commands::n8n::rollback_n8n_workflow_backup,
            commands::n8n::create_or_update_n8n_workflow_draft,
            commands::n8n::analyze_n8n_workflow_authoring_request,
            commands::n8n::generate_n8n_workflow_draft_plan,
            commands::n8n::create_n8n_workflow_draft_in_n8n,
            commands::n8n::preview_n8n_workflow_update_diff,
            commands::n8n::create_n8n_workflow_updated_copy,
            commands::n8n::apply_n8n_workflow_update_after_confirmation,
            commands::n8n::test_n8n_workflow_draft,
            commands::n8n::approve_n8n_workflow_draft,
            commands::n8n::reject_n8n_workflow_draft,
            commands::n8n::cleanup_n8n_workflow_draft,
            commands::n8n::get_n8n_workflow_authoring_sessions,
            commands::n8n::continue_n8n_workflow_authoring_operation,
            commands::n8n::rollback_n8n_workflow_authoring_update,
            commands::n8n::list_n8n_credential_summaries,
            commands::n8n::save_n8n_authoring_credential_mapping,
            commands::colab::get_colab_tier_status,
            commands::colab::connect_colab_tier,
            commands::colab::disconnect_colab_tier,
            commands::colab::set_colab_selected_notebook,
            commands::google_workspace::get_google_workspace_status,
            commands::briefing::get_briefing_config,
            commands::briefing::set_briefing_config,
            commands::tasks::task_list,
            commands::tasks::task_add,
            commands::tasks::task_update_status,
            commands::tasks::task_delete,
            commands::tasks::task_stats,
            commands::tasks::reminder_list,
            commands::tasks::reminder_set,
            commands::tasks::task_edit,
            commands::tasks::task_complete,
            commands::tasks::reminder_snooze,
            commands::tasks::reminder_cancel,
            commands::tasks::plan_my_day,
            commands::google_workspace::set_google_workspace_account,
            commands::google_workspace::connect_google_workspace,
            commands::google_workspace::disconnect_google_workspace,
            commands::runtime_status::get_orchestrator_status,
            commands::runtime_status::get_hra_diagnostics,
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
            commands::voice_diagnostics::voice_turn_diagnostics,
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
            commands::openclaw::openclaw_get_settings,
            commands::openclaw::openclaw_update_settings,
            commands::openclaw::install_skill_bundle,
            commands::openclaw::uninstall_skill_bundle,
            commands::openclaw::openclaw_generate_skill,
            commands::openclaw::openclaw_recommend_skills,
            commands::openclaw::openclaw_capability_manager,
            commands::openclaw::openclaw_execution_logs,
            commands::openclaw::openclaw_capability_graph,
            commands::openclaw::openclaw_list_grants,
            commands::openclaw::openclaw_revoke_grant,
            commands::openclaw::openclaw_get_developer_mode,
            commands::openclaw::openclaw_set_developer_mode,
            commands::capability::cpp_status,
            commands::capability::cpp_list_providers,
            commands::capability::cpp_discover,
            commands::capability::cpp_catalog,
            commands::capability::cpp_recommend,
            commands::capability::cpp_quarantined,
            commands::capability::cpp_release_quarantine,
            commands::capability::list_quarantined_tools,
            commands::capability::approve_quarantined_tool,
            commands::capability::reject_quarantined_tool,
            commands::capability::cpp_health,
            commands::capability::cpp_proposals,
            commands::capability::cpp_proposal_apply,
            commands::capability::cpp_proposal_undo,
            commands::capability::cpp_get_autonomy,
            commands::capability::cpp_set_autonomy,
            commands::capability::cpp_synthesis_preview,
            commands::capability::cpp_synthesize,
            commands::capability::cpp_discovery_status,
            commands::capability::cpp_discovery_scan,
            commands::capability::cpp_jobs,
            commands::capability::cpp_job_submit,
            commands::capability::cpp_job_control,
            commands::capability::cpp_descriptor,
            commands::capability::cpp_list_grants,
            commands::capability::cpp_revoke_grant,
            commands::capability::cpp_authorize,
            commands::capability::cpp_approve,
            commands::capability::cpp_execute,
            commands::capability::cpp_timeline,
            commands::gui_automation_control::get_gui_automation_status,
            commands::gui_automation_control::set_gui_automation_enabled,
            commands::gui_automation_control::get_gui_cognition_readiness_bypass,
            commands::gui_automation_control::set_gui_cognition_readiness_bypass,
            commands::gui_automation_control::get_grounding_status,
            commands::gui_automation_control::cancel_gui_cognition_turn,
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
            // Core state → OS tray glyph (kria-ui-redesign task 2.3, Req 3.4/18.2).
            // Enhancement with in-app fallback; degrades silently when no tray.
            tray::set_tray_core_state,
            // Capped detachable presentation surfaces (task 12.3) plus two
            // optional Mini companions (task 12.4). Stable labels enforce one
            // window per supported kind; none owns runtime authority.
            windows::open_detached_surface,
            windows::open_companion,
            windows::mirror_approval_presentation,
            windows::get_pending_approval_presentations,
            windows::sync_approval_presentation,
            // Summon: focus the main window (tray item + KRIA Mini call this);
            // the webview opens the Command Palette (kria-ui-redesign task 2.5,
            // Req 2.5/18.2).
            summon::summon,
            // Workflow runtime controls (kria-ui-redesign task 4.2 / design.md
            // §3.3 contract change b, Req 11.6): register the previously-
            // unregistered `workflow_*` commands so the Approval Center's
            // workflow HITL / cancel / continuation controls are no longer
            // inert. Cancellation propagation is preserved by the runtime.
            commands::workflow::workflow_hitl_respond,
            commands::workflow::workflow_cancel,
            commands::workflow::workflow_continuation,
            commands::workflow::workflow_runtime_status,
        ])
        .build(tauri::generate_context!());

    // Graceful boot-error fallback (task 0.6 / design.md §11.4): if the very
    // first (accelerated) boot fails to build the webview, and we are NOT
    // already in safe mode, relaunch ourselves in safe mode so a Wayland+NVIDIA
    // blank-screen/crash is self-healing instead of a dead white window.
    let app = match app {
        Ok(app) => app,
        Err(e) => {
            eprintln!("[KRIA] failed to build the application webview: {e}");
            #[cfg(target_os = "linux")]
            {
                if !boot_env.safe_mode_requested {
                    // Relaunches with KRIA_SAFE_MODE=1 and exits; only returns
                    // false if the relaunch could not be spawned.
                    safe_mode::relaunch_in_safe_mode();
                }
                eprintln!(
                    "[KRIA] already in safe mode (or relaunch failed). See docs/LINUX_GRAPHICS.md for manual env-flag guidance."
                );
            }
            std::process::exit(1);
        }
    };

    app.run(|app_handle, event| {
        if let tauri::RunEvent::ExitRequested { .. } = event {
            if RUNTIME_SHUTDOWN_REQUESTED.swap(true, Ordering::SeqCst) {
                return;
            }

            tauri::async_runtime::block_on(commands::shutdown_runtime(app_handle));
        }
    });
}
