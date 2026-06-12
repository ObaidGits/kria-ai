use super::*;
use crate::commands::colab::migrate_legacy_colab_server_command;
use std::collections::HashMap;

pub async fn init_runtime(handle: &AppHandle) -> anyhow::Result<()> {
    // Initialize logging first so startup diagnostics are filterable.
    let startup_started = std::time::Instant::now();
    let bootstrap_paths = kria_core::platform::paths::KriaPaths::resolve();
    kria_core::infra::logging::setup_logging(&bootstrap_paths.logs_dir);

    let mut config = KriaConfig::load(None)?;
    let paths = config.resolve_paths()?;
    if let Some(path) = config
        .n8n
        .migrate_literal_api_key_to_file()
        .map_err(|error| anyhow::anyhow!("failed to migrate n8n API key: {error}"))?
    {
        config.save()?;
        tracing::info!(
            target: "n8n_config",
            path = %path.display(),
            "migrated literal n8n API key into owner-only secret file during startup"
        );
    }

    // Resolve hardware tier with precedence: env > config > cache > detect.
    let hw_cache_path = paths.data_dir.join("hardware_tier.json");
    let (hw_info, hw_source) = resolve_hardware_info(&config, &hw_cache_path);

    // Cache latest hardware info to JSON.
    if let Ok(json) = serde_json::to_string_pretty(&hw_info) {
        let _ = std::fs::write(&hw_cache_path, json);
    }
    let hardware_info = Arc::new(hw_info);

    // Apply effective tier-aware runtime limits unless explicitly overridden.
    let tier_context_limit = hardware_info.tier.context_window();
    let requested_context_limit = if config.hardware.max_context_tokens > 0 {
        config.hardware.max_context_tokens
    } else {
        config.llm.context_window
    };
    if requested_context_limit == 0 {
        config.llm.context_window = tier_context_limit;
    } else if requested_context_limit > tier_context_limit {
        tracing::warn!(
            requested = requested_context_limit,
            tier_limit = tier_context_limit,
            tier = %hardware_info.tier.as_str(),
            "requested context window exceeded tier capacity; clamping"
        );
        config.llm.context_window = tier_context_limit;
    } else {
        config.llm.context_window = requested_context_limit;
    }

    if config.hardware.threads == 0 {
        config.hardware.threads = hardware_info.tier.thread_count();
    }
    if config.hardware.gpu_layers < 0 {
        config.hardware.gpu_layers = hardware_info.tier.gpu_layers();
    }
    if config.voice.stt_model.eq_ignore_ascii_case("auto") {
        config.voice.stt_model = hardware_info.tier.stt_model().to_string();
    }

    tracing::info!(
        source = %hw_source,
        tier = ?hardware_info.tier,
        ram_mb = hardware_info.total_ram_mb,
        vram_mb = ?hardware_info.vram_mb,
        gpu = ?hardware_info.gpu_name,
        cores = hardware_info.cpu_cores,
        "hardware detected"
    );

    // Initialize memory store (SQLite)
    let memory_store_backend = Arc::new(MemoryStore::open(&paths.db_path)?);
    let memory_store: Arc<dyn MemoryRuntime> = memory_store_backend.clone();

    // Initialize OpenClaw subsystem (synchronous — creates skills.db with both
    // `installed_skills` and `audit_log` tables immediately on boot).
    let openclaw_subsystem = match kria_core::openclaw::OpenClawSubsystem::boot(&paths.data_dir) {
        Ok(s) => {
            tracing::info!("[OpenClaw] subsystem ready");
            Some(s)
        }
        Err(e) => {
            tracing::warn!("[OpenClaw] subsystem boot failed (non-fatal): {e}");
            None
        }
    };

    let openclaw_registry: Arc<kria_core::openclaw::registry::SkillRegistry> =
        if let Some(ref s) = openclaw_subsystem {
            s.registry.clone()
        } else {
            let fallback = paths.data_dir.join("skills.db");
            let _ = std::fs::create_dir_all(&paths.data_dir);
            Arc::new(
                kria_core::openclaw::registry::SkillRegistry::open(&fallback)
                    .expect("fallback registry must open"),
            )
        };

    // Boot the ContainerPool only when explicitly enabled in user config.
    // Docker and the substrate image are optional; missing prerequisites should
    // disable OpenClaw cleanly instead of creating repeated background warnings.
    let openclaw_config = config.openclaw.clone();
    let openclaw_pool: Option<Arc<kria_core::openclaw::ContainerPool>> = if !openclaw_config.enabled
    {
        tracing::info!("[OpenClaw] container pool disabled by configuration");
        None
    } else {
        match kria_core::openclaw::ContainerPool::new(openclaw_config.clone()).await {
            Ok(pool) => {
                let pool = Arc::new(pool);
                if let Err(e) = pool.verify_image_available().await {
                    tracing::warn!(
                        image = %openclaw_config.image,
                        "[OpenClaw] container pool disabled: {e}"
                    );
                    None
                } else if let Err(e) = pool.initialize().await {
                    tracing::warn!(
                        "[OpenClaw] container pool disabled after pre-warm failure: {e}"
                    );
                    None
                } else {
                    // Spawn background loop that maintains warm Light containers.
                    kria_core::openclaw::ContainerPool::spawn_prewarm_loop(pool.clone());
                    tracing::info!("[OpenClaw] container pool ready");
                    Some(pool)
                }
            }
            Err(e) => {
                tracing::info!("[OpenClaw] container pool unavailable (Docker not running?): {e}");
                None
            }
        }
    };

    // Initialize model router from config
    let model_router = Arc::new(ModelRouter::from_config(&config));

    // EventBus (tokio broadcast channels)
    let event_bus = Arc::new(EventBus::new(256));

    // Health registry (created early so sidecar spawn can update it)
    let health = Arc::new(HealthRegistry::new());
    health.register("sidecar");
    health.update("sidecar", ServiceStatus::Starting, None);
    health.register("ocr_dependency");
    health.register("gui_uinput_daemon");
    health.register("gui_atspi_bus");
    health.register("gui_vision_ocr");
    health.update(
        "ocr_dependency",
        ServiceStatus::Starting,
        Some("Probing OCR dependency readiness".into()),
    );
    health.update(
        "gui_uinput_daemon",
        ServiceStatus::Starting,
        Some("Probing GUI input sidecar readiness".into()),
    );
    health.update(
        "gui_atspi_bus",
        ServiceStatus::Starting,
        Some("Probing AT-SPI accessibility bus readiness".into()),
    );
    health.update(
        "gui_vision_ocr",
        ServiceStatus::Starting,
        Some("Probing OCR/vision fallback readiness".into()),
    );

    // Python sidecar bridge (created early so tools can reference it)
    let venv_path = paths.data_dir.join("python-env");
    let venv_str = venv_path.to_string_lossy().to_string();
    let sidecar = Arc::new(SidecarBridge::new("python3", Some(&venv_str)));
    // Spawn sidecar in background — non-blocking; tools degrade gracefully if unavailable
    let sidecar_clone = sidecar.clone();
    let event_bus_clone = event_bus.clone();
    let health_sidecar = health.clone();
    tokio::spawn(async move {
        match sidecar_clone.spawn().await {
            Ok(()) => {
                tracing::info!("Python sidecar started successfully");
                event_bus_clone.publish(kria_core::infra::event_bus::KriaEvent::SidecarReady);
                health_sidecar.update("sidecar", ServiceStatus::Healthy, None);
                refresh_ocr_dependency_health(&health_sidecar, &sidecar_clone).await;
            }
            Err(e) => {
                tracing::warn!("Python sidecar failed to start (non-fatal): {}", e);
                health_sidecar.update("sidecar", ServiceStatus::Degraded, Some(format!("{e}")));
                health_sidecar.update(
                    "ocr_dependency",
                    ServiceStatus::Degraded,
                    Some("OCR unavailable: sidecar failed to start".into()),
                );
            }
        }
    });

    // ── Hardware Orchestrator (optional, manages llama-server lifecycle) ───────
    // Helper: resolve a model filename against multiple candidate directories.
    // Checks ~/.kria/models/llm/ first, then the workspace models/llm/ (for dev).
    let resolve_model_file = |filename: &str| -> String {
        let filename = filename.trim();
        if filename.is_empty() {
            return filename.to_string();
        }
        let candidates = if filename.to_ascii_lowercase().ends_with(".gguf") {
            vec![filename.to_string()]
        } else {
            vec![filename.to_string(), format!("{filename}.gguf")]
        };

        let direct = std::path::PathBuf::from(filename);
        if direct.is_absolute() {
            if direct.exists() {
                return direct.to_string_lossy().to_string();
            }
            if direct.extension().is_none() {
                let with_gguf = direct.with_extension("gguf");
                if with_gguf.exists() {
                    return with_gguf.to_string_lossy().to_string();
                }
            }
        }

        // 1. ~/.kria/models/llm/
        for candidate in &candidates {
            let p = paths.llm_models.join(candidate);
            if p.exists() {
                return p.to_string_lossy().to_string();
            }
        }
        // 2. Walk up from CWD to find workspace models/llm/ (Tauri dev runs from a sub-crate)
        if let Ok(cwd) = std::env::current_dir() {
            let mut dir = Some(cwd.as_path());
            while let Some(d) = dir {
                for candidate in &candidates {
                    let path = d.join("models").join("llm").join(candidate);
                    if path.exists() {
                        return path.to_string_lossy().to_string();
                    }
                }
                dir = d.parent();
            }
        }
        // 3. Return as-is (could be an absolute path already)
        filename.to_string()
    };

    // ── Hardware Orchestrator (non-blocking background startup) ───────────────
    // The orchestrator spawns llama-server and waits for /health (up to 120s).
    // We set AppState immediately (with orchestrator = None) so the frontend
    // is never blocked. The background task populates the RwLock when ready.
    let model_router_bg_ref = model_router.clone();
    let orch_cell: Arc<tokio::sync::RwLock<Option<Arc<Orchestrator>>>> =
        Arc::new(tokio::sync::RwLock::new(None));

    // Resolve model paths now (cheap, synchronous) so the background task
    // captures owned Strings rather than borrowing from `config`.
    //
    // The selection is **tier-aware**: on Lite/Standard hardware we pick the
    // smallest existing model (e.g. Phi-4-mini) instead of trying to load a
    // 4.7 GB Qwen2.5-VL and OOM-ing the GPU. On Performance/High hardware we
    // pick the largest fitting model with vision when available.
    //
    // The user can override the selection by setting `[llm].active_model` to
    // a model name from `[[llm.models]]`; that override is honoured iff the
    // GGUF file actually exists on disk.
    //
    // Cloud/external providers (OpenAI, Gemini, Anthropic, OpenRouter, etc.)
    // do not use a local llama-server. Skip the orchestrator entirely so no
    // GPU resources are allocated, no idle-release loop runs, and startup is
    // instant. The orchestrator is only meaningful for local inference.
    let routing_mode_is_cloud = {
        use kria_core::llm::model_router::RoutingMode;
        let mode: RoutingMode = config
            .llm
            .routing_mode
            .parse()
            .unwrap_or(RoutingMode::Local);
        mode != RoutingMode::Local
    };

    let (orch_model_path, orch_mmproj_path, orch_config, orch_enabled, selected_model_name) =
        if routing_mode_is_cloud {
            tracing::info!(
                routing_mode = %config.llm.routing_mode,
                "orchestrator: skipped — cloud/external provider active, no local GPU resources needed"
            );
            let _ = handle.emit(
                "orchestrator:disabled",
                serde_json::json!({
                    "reason": "cloud_provider_active",
                    "routing_mode": config.llm.routing_mode,
                    "message": "Cloud provider active — local model runtime not started.",
                }),
            );
            (
                String::new(),
                None,
                config.orchestrator.clone(),
                false,
                String::new(),
            )
        } else if config.orchestrator.enabled {
            use kria_core::llm::orchestrator::tier_strategy::{
                derive_model_profile, select_model_for_tier, SelectionReason,
            };

            let model_exists = |file: &str| -> bool {
                let resolved = resolve_model_file(file);
                std::path::Path::new(&resolved).exists()
            };
            let configured_active_provider = config.providers.active().cloned();
            let selected_local_model = configured_active_provider
                .as_ref()
                .filter(|provider| {
                    provider.provider_type
                        == kria_core::llm::provider::config::ProviderType::LlamaCpp
                })
                .and_then(|provider| {
                    let model = provider.active_model.trim();
                    (!model.is_empty()).then(|| model.to_string())
                })
                .unwrap_or_else(|| config.llm.active_model.clone());
            let mut startup_models = config.llm.models.clone();

            if !selected_local_model.trim().is_empty()
                && !startup_models.iter().any(|model| {
                    model.name.eq_ignore_ascii_case(&selected_local_model)
                        || model.file.eq_ignore_ascii_case(&selected_local_model)
                        || std::path::Path::new(&model.file)
                            .file_stem()
                            .and_then(|stem| stem.to_str())
                            .map(|stem| stem.eq_ignore_ascii_case(&selected_local_model))
                            .unwrap_or(false)
                })
            {
                let selected_path = resolve_model_file(&selected_local_model);
                let selected_path_ref = std::path::Path::new(&selected_path);
                if selected_path_ref.exists() {
                    let file = selected_path_ref
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or(selected_local_model.as_str())
                        .to_string();
                    let name = selected_path_ref
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or(selected_local_model.as_str())
                        .to_string();
                    let size_bytes = std::fs::metadata(selected_path_ref)
                        .map(|metadata| metadata.len())
                        .unwrap_or(0);
                    let size_gb = (size_bytes as f32 / 1024.0 / 1024.0 / 1024.0).max(1.0);
                    startup_models.push(kria_core::config::LocalModelDef {
                        name: name.clone(),
                        file,
                        display_name: name,
                        context_window: config.llm.context_window.max(2048),
                        max_tokens: config.llm.max_tokens.max(512),
                        vram_estimate_gb: size_gb,
                        capabilities: vec!["chat".to_string()],
                        mmproj_file: None,
                    });
                }
            }

            let choice = select_model_for_tier(
                hardware_info.tier,
                hardware_info.total_ram_mb,
                hardware_info.vram_mb,
                &selected_local_model,
                &startup_models,
                model_exists,
            );

            match choice {
                None => {
                    tracing::warn!(
                        "orchestrator: no models defined in `[[llm.models]]` — \
                     skipping background startup. Add a model entry in \
                     ~/.kria/config.toml or run `scripts/download_models.py`."
                    );
                    let _ = handle.emit(
                        "orchestrator:disabled",
                        serde_json::json!({
                            "reason": "no_models_configured",
                            "message": "No LLM models are defined in config.toml.",
                        }),
                    );
                    (
                        String::new(),
                        None,
                        config.orchestrator.clone(),
                        false,
                        String::new(),
                    )
                }
                Some(c) if matches!(c.reason, SelectionReason::NoModels) => {
                    let searched: Vec<String> = config
                        .llm
                        .models
                        .iter()
                        .map(|m| resolve_model_file(&m.file))
                        .collect();
                    tracing::error!(
                        tier = %hardware_info.tier.as_str(),
                        searched = ?searched,
                        "orchestrator: no GGUF model files found on disk — skipping startup. \
                         Run `scripts/download_models.py` or place the GGUF in ~/.kria/models/llm/"
                    );
                    let _ = handle.emit(
                    "orchestrator:disabled",
                    serde_json::json!({
                        "reason": "model_files_missing",
                        "tier": hardware_info.tier.as_str(),
                        "searched_paths": searched,
                        "message": "No GGUF model files found. Download models or update config.",
                    }),
                );
                    (
                        String::new(),
                        None,
                        config.orchestrator.clone(),
                        false,
                        String::new(),
                    )
                }
                Some(c) => {
                    let model_path = resolve_model_file(&c.model.file);
                    let mmproj_path = c
                        .model
                        .mmproj_file
                        .as_ref()
                        .filter(|_| !c.vision_disabled)
                        .map(|f| resolve_model_file(f));

                    tracing::info!(
                        tier = %hardware_info.tier.as_str(),
                        model = %c.model.name,
                        file = %c.model.file,
                        resolved = %model_path,
                        reason = ?c.reason,
                        vision_disabled = c.vision_disabled,
                        mmproj = ?mmproj_path,
                        "orchestrator: tier-aware model selection complete"
                    );

                    // Override active_model so the model_router and other subsystems
                    // agree on which model is actually loaded.
                    config.llm.active_model = c.model.name.clone();

                    // Derive a tier-appropriate ModelProfile and substitute it
                    // into the orchestrator config. This way each model gets its
                    // own VRAM-budget calculation (layer count, mmproj size, …).
                    let mut orch_cfg = config.orchestrator.clone();
                    orch_cfg.model_profile =
                        derive_model_profile(&c.model, &config.orchestrator.model_profile);

                    // Hardware-tier safety pass: clamps mlock / flash_attention /
                    // batch_size / safety_margin to values the detected machine
                    // can actually handle. Without this, defaults like
                    // `mlock=true` + a 5GB Qwen2.5-VL on a 16GB laptop will OOM
                    // and freeze the system at startup.
                    let model_size_mb = std::fs::metadata(&model_path)
                        .map(|m| m.len() / (1024 * 1024))
                        .unwrap_or((c.model.vram_estimate_gb as u64) * 1024);
                    orch_cfg.tune_for_tier(
                        hardware_info.tier,
                        hardware_info.total_ram_mb,
                        hardware_info.vram_mb,
                        model_size_mb,
                    );

                    tracing::info!(
                        tier = %hardware_info.tier.as_str(),
                        ram_mb = hardware_info.total_ram_mb,
                        vram_mb = ?hardware_info.vram_mb,
                        model_size_mb,
                        mlock = orch_cfg.mlock,
                        flash_attention = orch_cfg.flash_attention,
                        batch_size = orch_cfg.batch_size,
                        safety_margin_mb = orch_cfg.safety_margin_mb,
                        "orchestrator: tuned config for detected hardware tier"
                    );

                    tracing::info!(
                        total_layers = orch_cfg.model_profile.total_layers,
                        per_layer_vram_mb = orch_cfg.model_profile.per_layer_vram_mb,
                        has_vision = orch_cfg.model_profile.has_vision_projector,
                        mmproj_vram_mb = orch_cfg.model_profile.mmproj_vram_mb,
                        max_context = orch_cfg.model_profile.max_context,
                        "orchestrator: derived model profile"
                    );

                    let _ = handle.emit(
                        "orchestrator:selected",
                        serde_json::json!({
                            "tier": hardware_info.tier.as_str(),
                            "model": c.model.name,
                            "display_name": c.model.display_name,
                            "vram_estimate_gb": c.model.vram_estimate_gb,
                            "vision_enabled": !c.vision_disabled
                                && c.model.capabilities.iter().any(|x| x == "vision"),
                        }),
                    );

                    let model_name = c.model.name.clone();
                    (model_path, mmproj_path, orch_cfg, true, model_name)
                }
            }
        } else {
            tracing::info!("orchestrator: disabled in config (orchestrator.enabled = false)");
            let _ = handle.emit(
                "orchestrator:disabled",
                serde_json::json!({ "reason": "config_disabled" }),
            );
            (
                String::new(),
                None,
                config.orchestrator.clone(),
                false,
                String::new(),
            )
        };

    if !selected_model_name.trim().is_empty() {
        model_router_bg_ref
            .set_active_local_model_label(selected_model_name.clone())
            .await;
    }

    // ── Batch 1: PSDG — Initialize WorldModelStore EARLY ─────────────────────
    // Must happen before AgentLoop construction so PsdgHandle can be wired in.
    // Opens WorldModelStore against the same db_path as MemoryStore (WAL-safe).
    let world_model_early: Option<kria_core::agent::PsdgHandle> =
        match kria_core::agent::PsdgHandle::open(&paths.db_path) {
            Ok(handle) => {
                tracing::info!("[INIT] PSDG: WorldModelStore opened (WAL, same db as MemoryStore)");
                Some(handle)
            }
            Err(e) => {
                tracing::warn!("[INIT] PSDG: WorldModelStore open failed (degraded): {}", e);
                None
            }
        };

    // Initialize embedding model and vector index for fact extraction
    let embeddings = Arc::new(EmbeddingModel::load(384).unwrap_or_else(|e| {
        tracing::warn!("embedding model load error (using fallback): {}", e);
        EmbeddingModel::load(384).expect("fallback always succeeds")
    }));
    let vectors_path = paths.data_dir.join("vectors.bin");
    let vectors = Arc::new(
        VectorIndex::open(&vectors_path, 384).unwrap_or_else(|_| VectorIndex::in_memory(384)),
    );

    // Build the full tool registry (60+ tools + 6 precognitive) with MemoryStore, RAG, and Proactive
    let rag_engine = Arc::new(kria_core::memory::RagEngine::new(
        memory_store_backend.clone(),
        vectors.clone(),
        embeddings.clone(),
    ));
    let proactive_engine = Arc::new(kria_core::automation::ProactiveEngine::new(
        kria_core::automation::proactive::HealthThresholds::default(),
    ));
    let tool_registry_inner = registry::build_registry_full_with_psdg(
        Some(memory_store.clone()),
        Some(rag_engine.clone()),
        Some(proactive_engine.clone()),
        world_model_early.clone(),
    );
    kria_core::tools::precognitive::register(&tool_registry_inner, sidecar.clone());
    kria_core::tools::news::register(&tool_registry_inner, sidecar.clone());
    // Re-register vision tools with sidecar (overrides the None-sidecar registration from build_registry)
    kria_core::tools::vision::register(
        &tool_registry_inner,
        Some(sidecar.clone()),
        Some(GpuLeaseManager::shared(
            std::time::Duration::from_secs(120),
            std::time::Duration::from_secs(15),
        )),
    );

    // ── Image generation orchestrator ─────────────────────────────────────────
    let image_cfg = config.image_generation.clone();
    let image_orchestrator = ImageOrchestrator::new(image_cfg, &paths.data_dir);
    {
        // Build an EventEmitter that forwards image/voice events to the Tauri frontend.
        let handle_img = handle.clone();
        let img_emit_fn: std::sync::Arc<dyn Fn(&str, serde_json::Value) + Send + Sync + 'static> =
            std::sync::Arc::new(move |event_name: &str, payload: serde_json::Value| {
                let _ = handle_img.emit(event_name, payload);
            });
        kria_core::tools::image_generation::register(
            &tool_registry_inner,
            image_orchestrator.clone(),
            img_emit_fn,
            orch_cell.clone(),
        );
    }
    tracing::info!("[INIT] image generation orchestrator ready");

    // ── MCP server startup ────────────────────────────────────────────────────
    // Load MCP server configs from mcp_servers.json (supplements TOML config)
    tracing::debug!("[MCP] loading MCP server configs from mcp_servers.json");
    {
        let mut cfg = config.clone();
        kria_core::config::load_mcp_servers(&mut cfg);
        config = cfg;
    }
    let mut config_dirty = false;
    for server in config.mcp.servers.iter_mut() {
        if server.name == COLAB_DEFAULT_SERVER_NAME && migrate_legacy_colab_server_command(server) {
            tracing::info!(
                "[MCP] migrated legacy Colab MCP launch config to current uvx --from entrypoint format"
            );
            config_dirty = true;
        }
    }
    sync_telegram_mcp_server_config(&mut config);
    sync_google_workspace_server_config(&mut config, None);
    apply_google_runtime_env_from_config(&config);
    if config_dirty {
        if let Err(error) = config.save() {
            tracing::warn!(error = %error, "failed to persist migrated MCP config");
        }
    }
    let total_servers = config.mcp.servers.len();
    let enabled_servers = config.mcp.servers.iter().filter(|s| s.enabled).count();
    tracing::info!(
        target: "mcp_config",
        configured = total_servers,
        enabled = enabled_servers,
        disabled = total_servers.saturating_sub(enabled_servers),
        "[MCP] {} total MCP server(s) configured, {} enabled",
        total_servers,
        enabled_servers
    );
    for s in &config.mcp.servers {
        tracing::debug!(
            target: "mcp_config",
            server = %s.name,
            enabled = s.enabled,
            command = %s.command,
            args = ?s.args,
            "[MCP]   server='{}' enabled={} command='{}' args={:?}",
            s.name,
            s.enabled,
            s.command,
            s.args
        );
    }

    // Create the lazy Google Workspace client ref BEFORE starting servers.
    // This is passed to register() so gw_* tools exist in the registry
    // regardless of whether the MCP server connects successfully.
    let gw_client_ref = gw::new_client_ref();
    tracing::info!("[GW] created lazy GwClientRef — registering Google Workspace tools now");
    gw::register(&tool_registry_inner, gw_client_ref.clone(), sidecar.clone());

    let workflow_registry_path = kria_core::n8n::default_workflow_registry_store_path();
    let (workflow_registry_store, migrated_legacy_n8n_workflows) =
        match kria_core::n8n::migrate_toml_workflows_to_registry_at(
            &workflow_registry_path,
            &config.n8n.workflows,
        ) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(
                    target: "n8n_workflow_registry",
                    error = %error,
                    path = %workflow_registry_path.display(),
                    "[n8n] failed to migrate legacy TOML workflow registry"
                );
                (
                    kria_core::n8n::load_workflow_registry_store_at(&workflow_registry_path)
                        .unwrap_or_default(),
                    0,
                )
            }
        };
    if migrated_legacy_n8n_workflows > 0 {
        tracing::warn!(
            target: "n8n_workflow_registry",
            migrated = migrated_legacy_n8n_workflows,
            path = %workflow_registry_path.display(),
            "[n8n] legacy TOML workflow entries detected; using migrated workflow_registry.json instead"
        );
    } else if !config.n8n.workflows.is_empty() {
        tracing::warn!(
            target: "n8n_workflow_registry",
            legacy_toml_workflows = config.n8n.workflows.len(),
            registry_workflows = workflow_registry_store.workflows.len(),
            "[n8n] legacy TOML workflow entries detected; workflow_registry.json remains source of truth"
        );
    }
    let workflow_registry_workflows =
        kria_core::n8n::workflow_registry_workflows(&workflow_registry_store);
    let mut n8n_runtime_config = config.n8n.clone();
    n8n_runtime_config.workflows = workflow_registry_workflows.clone();

    match kria_core::n8n::register_into_tool_registry(
        &tool_registry_inner,
        n8n_runtime_config.clone(),
    ) {
        Ok(Some(_client)) => {
            tracing::info!(
                workflows = workflow_registry_workflows.len(),
                "[n8n] registered n8n_invoke_workflow tool from workflow_registry.json"
            );
        }
        Ok(None) => {
            tracing::debug!("[n8n] integration disabled; n8n tools not registered");
        }
        Err(error) => {
            tracing::warn!(error = %error, "[n8n] integration configuration invalid; n8n tools not registered");
        }
    }
    let n8n_catalog = Arc::new(RwLock::new(if config.n8n.enabled {
        match kria_core::n8n::N8nCatalog::new(n8n_runtime_config.with_resolved_secret()) {
            Ok(catalog) => Some(Arc::new(catalog)),
            Err(error) => {
                tracing::warn!(error = %error, "[n8n] callback catalog unavailable");
                None
            }
        }
    } else {
        None
    }));
    let n8n_state_store = Arc::new(kria_core::n8n::N8nWorkflowStateStore::default());
    let n8n_inbox_path = paths.data_dir.join("n8n").join("callback_inbox.jsonl");
    let n8n_audit_path = paths.data_dir.join("n8n").join("governance_audit.jsonl");
    let n8n_governance_log = Arc::new(RwLock::new(
        Vec::<kria_core::n8n::N8nGovernanceDecision>::new(),
    ));
    let n8n_hitl_responses = Arc::new(RwLock::new(HashMap::<String, serde_json::Value>::new()));
    match replay_n8n_inbox(n8n_inbox_path.as_path(), &n8n_state_store).await {
        Ok(count) if count > 0 => {
            tracing::info!(count, "[n8n] replayed durable callback inbox");
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(error = %error, path = %n8n_inbox_path.display(), "[n8n] failed to replay callback inbox");
        }
    }

    // ── n8n Background Maintenance Task ──────────────────────────────────────
    // Periodically checks for:
    // 1. Timed-out workflow runs (no callback within deadline)
    // 2. Stale HITL responses (expire after 10 min)
    // 3. Old completed runs (evict from memory after 1 hour)
    {
        let state_store = n8n_state_store.clone();
        let hitl_responses = n8n_hitl_responses.clone();
        let handle_n8n_timeout = handle.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;

                // 1. Check for timed-out runs (use Background deadline: 5 min)
                let timed_out = state_store.check_timeouts(300_000);
                if !timed_out.is_empty() {
                    tracing::warn!(
                        target: "n8n_maintenance",
                        count = timed_out.len(),
                        "Marked n8n runs as timed out (no callback within deadline)"
                    );
                    for run in &timed_out {
                        let _ = handle_n8n_timeout.emit(
                            "n8n:workflow_timeout",
                            serde_json::json!({
                                "event_type": "n8n:workflow_timeout",
                                "workflow_id": run.workflow_id,
                                "workflow_version": run.workflow_version,
                                "correlation_id": run.correlation_id,
                                "status": "timed_out",
                                "timestamp_ms": std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_millis() as u64)
                                    .unwrap_or(0),
                                "user_visible_summary": "Workflow timed out while waiting for an n8n terminal callback.",
                            }),
                        );
                    }
                }

                // 2. Expire old HITL responses (>10 min)
                {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    let mut responses = hitl_responses.write().await;
                    let before = responses.len();
                    responses.retain(|_, v| {
                        v.get("decided_at_unix_ms")
                            .and_then(|t| t.as_u64())
                            .map(|t| now_ms.saturating_sub(t) < 600_000) // 10 min
                            .unwrap_or(true) // keep if no timestamp
                    });
                    let removed = before - responses.len();
                    if removed > 0 {
                        tracing::debug!(target: "n8n_maintenance", removed, "Expired old HITL responses");
                    }
                }

                // 3. Evict completed runs older than 1 hour from memory
                let evicted = state_store.evict_old_runs(3_600_000);
                if evicted > 0 {
                    tracing::debug!(target: "n8n_maintenance", evicted, "Evicted old completed n8n runs");
                }
            }
        });
        tracing::info!("[n8n] background maintenance task started (timeout check + cleanup)");
    }

    let fleet_control_runtime = match DesktopFleetControlRuntime::initialize(
        paths.data_dir.as_path(),
    )
    .await
    {
        Ok(runtime) => Arc::new(runtime),
        Err(error) => {
            tracing::warn!(error = %error, "desktop fleet-control runtime initialization failed; using empty runtime");
            Arc::new(DesktopFleetControlRuntime::empty())
        }
    };
    register_fleet_runtime_tools(&tool_registry_inner, fleet_control_runtime.clone());

    if let Some(catalog) = n8n_catalog.read().await.clone() {
        n8n::register_n8n_adapter_tool_handler(
            &tool_registry_inner,
            n8n::N8nAdapterRuntime {
                catalog,
                catalog_slot: Some(n8n_catalog.clone()),
                n8n_state_store: n8n_state_store.clone(),
                n8n_inbox_path: n8n_inbox_path.clone(),
                n8n_audit_path: n8n_audit_path.clone(),
                n8n_governance_log: n8n_governance_log.clone(),
                app_handle: Some(handle.clone()),
                fleet_control_runtime: Some(fleet_control_runtime.clone()),
            },
        );
        tracing::info!(
            "[n8n] registered adapter-backed n8n_invoke_workflow tool for chat/local tool dispatch"
        );
    }

    // Wrap registry in Arc immediately — thread-safe for background MCP registration
    let tool_registry = Arc::new(tool_registry_inner);

    // Register active OpenClaw skills as oc_* tools (requires pool to be ready).
    if let (Some(ref subsystem), Some(ref pool)) = (&openclaw_subsystem, &openclaw_pool) {
        subsystem.register_into_tool_registry(&tool_registry, pool.clone());
    }

    tracing::info!(
        tools = tool_registry.len(),
        "[INIT] base tool registry ready ({} tools, MCP tools will be added in background)",
        tool_registry.len()
    );

    // Create MCP manager (servers not started yet — will launch in background)
    let mcp_configs = config.mcp.servers.clone();
    let mcp_manager: Arc<tokio::sync::Mutex<McpServerManager>> =
        Arc::new(tokio::sync::Mutex::new(McpServerManager::new(mcp_configs)));

    // Build tool mount manager (controls which tool groups are visible to the LLM)
    let mount_mgr = Arc::new(tokio::sync::RwLock::new(
        mount_manager::build_default_mount_manager(),
    ));

    // Safety subsystems
    // HITL timeout: 5 minutes (300s). Previous value (30s) was too short — users
    // need time to read the prompt, evaluate the action, and respond. With the
    // longer timeout the system feels collaborative rather than rushed.
    let hitl = Arc::new(HitlGateway::new(300));

    let policy_engine = Arc::new(PolicyEngine::new());

    // ── Intent Dispatcher + App Lifecycle tools (full dispatcher path) ────────
    // Wire the IntentDispatcher so browser_search, open_application, open_url,
    // and send_message use the full policy/rate-limit/schema-validation pipeline
    // instead of the legacy fallback handlers registered by build_registry_full.
    // This also ensures correct X11/Wayland handling via LinuxBackend.
    {
        use kria_core::platform::app_registry::InstalledAppRegistry;
        use kria_core::platform::intent::dispatcher::IntentDispatcher;
        use kria_core::platform::intent::linux::LinuxBackend;

        // build_async uses spawn_blocking for the filesystem scan — safe inside async context.
        // build_sync would block_on inside an async context causing a deadlock.
        let app_registry = InstalledAppRegistry::build_async().await;
        let linux_backend = Arc::new(LinuxBackend::new(app_registry.clone()));
        let intent_dispatcher = Arc::new(IntentDispatcher::new(
            linux_backend,
            app_registry.clone(),
            policy_engine.clone(),
        ));

        // Re-register app lifecycle tools with the full dispatcher.
        // This overrides the legacy handlers registered by build_registry_full.
        // tool_registry is already wrapped in Arc at this point — deref to get &ToolRegistry.
        kria_core::tools::app_lifecycle::register_with_dispatcher(
            &*tool_registry,
            Some(intent_dispatcher),
            Some(app_registry),
        );

        tracing::info!("[INIT] app lifecycle tools re-registered with full IntentDispatcher (X11/Wayland aware)");

        // Run startup validation to detect stale binary / missing tool registrations.
        // This catches the production/eval divergence where tools return
        // "tool does not implement execute" instead of working correctly.
        let failed_tools =
            kria_core::agent::gui_wiring::validate_gui_tool_registry(&tool_registry).await;
        if !failed_tools.is_empty() {
            tracing::error!(
                "[INIT] GUI tool registry validation FAILED for: {:?}. \
                 GUI automation will not work. Rebuild the production binary.",
                failed_tools
            );
        }

        // Run accessibility capability detection at startup.
        // This detects whether AT-SPI is enabled and surfaces remediation if not.
        // Also attempts automatic enablement if toolkit-accessibility is disabled.
        {
            let caps = kria_core::agent::atspi_engine::detect_capabilities().await;
            if caps.accessibility_operational {
                tracing::info!("[INIT] Accessibility: OPERATIONAL — AT-SPI interaction enabled");
            } else {
                tracing::warn!(
                    "[INIT] Accessibility: NOT OPERATIONAL — semantic UI interaction disabled. \
                     toolkit_accessibility_enabled={}, atspi_bus_available={}, accessible_apps_detected={}. \
                     Fix: {}",
                    caps.toolkit_accessibility_enabled,
                    caps.atspi_bus_available,
                    caps.accessible_apps_detected,
                    caps.remediation.first().map(|s| s.as_str()).unwrap_or("see accessibility_doctor tool")
                );

                // Attempt automatic enablement if toolkit-accessibility is the only issue
                if !caps.toolkit_accessibility_enabled && caps.atspi_bus_available {
                    tracing::info!(
                        "[INIT] Attempting automatic accessibility enablement via gsettings..."
                    );
                    let enable_result = tokio::process::Command::new("gsettings")
                        .args([
                            "set",
                            "org.gnome.desktop.interface",
                            "toolkit-accessibility",
                            "true",
                        ])
                        .output()
                        .await;
                    match enable_result {
                        Ok(out) if out.status.success() => {
                            tracing::info!(
                                "[INIT] Accessibility auto-enabled successfully. \
                                 AT-SPI interaction will be available for new app launches."
                            );
                        }
                        Ok(out) => {
                            let stderr = String::from_utf8_lossy(&out.stderr);
                            tracing::warn!(
                                "[INIT] Accessibility auto-enable failed: {}. \
                                 Run manually: gsettings set org.gnome.desktop.interface toolkit-accessibility true",
                                stderr.trim()
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[INIT] gsettings not available: {}. \
                                 Run manually: gsettings set org.gnome.desktop.interface toolkit-accessibility true",
                                e
                            );
                        }
                    }
                }
            }
        }
    }

    let audit_db = rusqlite::Connection::open(&paths.db_path)?;
    let audit_logger = Arc::new(AuditLogger::new(audit_db));

    let rollback_mgr = Arc::new(RollbackManager::new(
        paths.rollback_dir.clone(),
        24,  // retention hours
        512, // max storage MB
    ));

    let routing_config = config.routing.clone();
    let mut router_tool_descriptions: Vec<(String, String, String)> = tool_registry
        .list_defs()
        .into_iter()
        .map(|def| (def.name, def.description, def.category))
        .collect();
    router_tool_descriptions.sort_by(|a, b| a.0.cmp(&b.0));
    let routing_cache_dir = {
        let configured = PathBuf::from(&routing_config.cache_dir);
        if configured.is_absolute() {
            configured
        } else {
            paths.data_dir.join(configured)
        }
    };
    let (semantic_router, _router_event_tx) = kria_core::routing::Router::new(
        routing_config.clone(),
        routing_cache_dir,
        router_tool_descriptions.clone(),
    )
    .await;

    // Phase 3: Build tool-level semantic index
    let tool_defs_for_index: Vec<kria_core::tools::registry::ToolDef> =
        tool_registry.list_defs().to_vec();
    let tool_index = kria_core::routing::tool_index::SharedToolIndex::new(
        tool_defs_for_index,
        routing_config.clone(),
    )
    .await;
    tracing::info!("Tool semantic index initialized");

    // Phase 5: Build feedback collector
    let feedback_collector = Arc::new(tokio::sync::Mutex::new(
        kria_core::routing::feedback::FeedbackCollector::default_config(),
    ));

    // Build the agent loop
    let max_tool_rounds = config.agent.max_tool_rounds.max(1);
    let min_confidence_to_act = config.agent.min_confidence_to_act;
    let clarify_threshold = config.agent.clarify_threshold;
    let doc_store = Arc::new(kria_core::preprocessing::SessionVectorStore::new(
        paths.data_dir.join("uploads"),
        5,
    ));

    // Wire BoundedExecutionVerifier (Batch 1 Phase 1.5)
    let execution_verifier: Arc<dyn kria_core::agent::execution_verifier::ExecutionVerifier> =
        Arc::new(kria_core::agent::execution_verifier_bounded::BoundedExecutionVerifier::new());

    let mut agent_loop_builder = AgentLoop::new(
        model_router.clone(),
        tool_registry.clone(),
        mount_mgr,
        policy_engine.clone(),
        hitl.clone(),
        audit_logger.clone(),
        rollback_mgr,
    )
    .with_semantic_router(semantic_router)
    .with_tool_index(tool_index)
    .with_feedback_collector(feedback_collector)
    .with_doc_store(doc_store)
    .with_max_tool_rounds(max_tool_rounds)
    .with_confidence_thresholds(min_confidence_to_act, clarify_threshold)
    .with_hardware_tier(hardware_info.tier.as_str())
    .with_execution_verifier(execution_verifier);

    // Wire PSDG handle into AgentLoop (Batch 1 Phase 1.1 + 3.12)
    if let Some(ref psdg) = world_model_early {
        agent_loop_builder = agent_loop_builder.with_world_model(psdg.clone());
        tracing::info!("[INIT] PSDG: PsdgHandle wired into AgentLoop (context injection active)");
    }

    // Wire RuleIntentCompiler (Batch 1 Finalization: replaces NoopIntentCompiler).
    // RuleIntentCompiler uses deterministic pattern matching — no LLM calls, no I/O.
    // Falls back to Verb::Other for unrecognised patterns, which routes to LLM HTN planner.
    {
        let rule_compiler =
            std::sync::Arc::new(kria_core::agent::intent_compiler_rule::RuleIntentCompiler);
        agent_loop_builder = agent_loop_builder.with_intent_compiler(rule_compiler);
        tracing::info!(
            "[INIT] IntentCompiler: RuleIntentCompiler wired (NoopIntentCompiler retired)"
        );
    }

    // Wire SessionManager for ReAct checkpoint persistence (Batch 1 Phase 3)
    {
        let session_mgr = Arc::new(kria_core::agent::workflow_session::SessionManager::new());

        // Enforce session limits on startup to prevent unbounded accumulation.
        // Caps: 7-day age limit + maximum 200 sessions retained.
        // Eval scripts and frequent automation users would otherwise accumulate
        // thousands of session checkpoints over time.
        session_mgr.enforce_session_limits(168, 200); // 168h = 7 days, 200 sessions

        agent_loop_builder = agent_loop_builder.with_session_manager(session_mgr);
        tracing::info!("[INIT] SessionManager: ReAct checkpoint persistence wired");
    }

    // Wire HealthRegistry for runtime observability event counting (Batch 1 Phase 5)
    agent_loop_builder = agent_loop_builder.with_health_registry(health.clone());

    // Wire ExecutionTransparencyLayer for ReAct lineage tracing (Batch 1 Phase 5 hardening)
    {
        let transparency =
            kria_core::agent::execution_transparency::ExecutionTransparencyLayer::new(
                world_model_early.clone(),
            );
        agent_loop_builder = agent_loop_builder.with_transparency_layer(transparency);
        tracing::info!("[INIT] ExecutionTransparencyLayer: ReAct lineage tracing wired");
    }

    // ── Batch 2: Human-Aligned Cognition Runtime engines ─────────────────────────
    // Phase 1: ObservableCompletionEngine — verifies human-visible outcomes post-turn.
    {
        let oce = std::sync::Arc::new(
            kria_core::agent::observable_completion::ObservableCompletionEngine::new(
                world_model_early.clone(),
            ),
        );
        agent_loop_builder = agent_loop_builder.with_observable_completion(oce);
        tracing::info!(
            "[INIT] Batch2 ObservableCompletionEngine: human-visible outcome verification wired"
        );
    }
    // Phase 2: WorkflowExpectationEngine — workflow category classification + expectation.
    {
        let wee = std::sync::Arc::new(
            kria_core::agent::workflow_expectation::WorkflowExpectationEngine::new(
                world_model_early.clone(),
            ),
        );
        agent_loop_builder = agent_loop_builder.with_workflow_expectation(wee);
        tracing::info!(
            "[INIT] Batch2 WorkflowExpectationEngine: semantic workflow classification wired"
        );
    }
    // Phase 3: CollaborativeAutonomyEngine — per-turn autonomy advisory notices.
    {
        let cae = std::sync::Arc::new(
            kria_core::agent::collaborative_autonomy::CollaborativeAutonomyEngine::new(
                world_model_early.clone(),
            ),
        );
        agent_loop_builder = agent_loop_builder.with_collaborative_autonomy(cae);
        tracing::info!("[INIT] Batch2 CollaborativeAutonomyEngine: autonomy advisory wired");
    }
    // Phase 4: WorkflowContinuationRuntime — interruption classification + recovery planning.
    let workflow_continuation_runtime = std::sync::Arc::new(
        kria_core::agent::workflow_continuation::WorkflowContinuationRuntime::new(
            world_model_early.clone(),
        ),
    );
    agent_loop_builder =
        agent_loop_builder.with_continuation_runtime(workflow_continuation_runtime.clone());
    tracing::info!("[INIT] Batch2 WorkflowContinuationRuntime: interruption-aware recovery wired");

    let agent_loop = Arc::new(agent_loop_builder);

    // Spawn periodic runtime health reporter (Batch 1 Phase 5)
    kria_core::infra::health::RuntimeHealthReporter::spawn(health.clone(), 30);

    tracing::info!("KRIA runtime initialized — agent loop active");

    // Build voice pipeline (v1 — always built so the legacy code path keeps
    // working). When `voice.engine = "v2"` we ALSO build the v2 pipeline
    // alongside and store it as `ActivePipeline::Streaming`.
    let voice_pipeline = build_voice_pipeline(&config, &paths);
    let (active_voice_init, voice_v2_telemetry_init) =
        if config.voice.engine.eq_ignore_ascii_case("v2") {
            match build_v2_pipeline(&config, &paths, hardware_info.tier) {
                Ok((v2, _state_rx, telemetry_rx)) => {
                    tracing::info!(engine = "v2", "voice v2 pipeline constructed");
                    (
                        kria_core::voice::v2::ActivePipeline::Streaming(v2),
                        Some(telemetry_rx),
                    )
                }
                Err(e) => {
                    tracing::warn!(error = %e, "v2 pipeline build failed; falling back to v1");
                    (
                        kria_core::voice::v2::ActivePipeline::Legacy(voice_pipeline.clone()),
                        None,
                    )
                }
            }
        } else {
            (
                kria_core::voice::v2::ActivePipeline::Legacy(voice_pipeline.clone()),
                None,
            )
        };

    // Health registry — register all subsystems
    health.register("memory_store");
    health.register("model_router");
    health.register("tool_registry");
    health.register("agent_loop");
    health.register("voice_pipeline");
    health.register("embeddings");
    health.register("vectors");
    // Mark core services as healthy
    health.update("memory_store", ServiceStatus::Healthy, None);
    // model_router: probe the actual LLM server asynchronously
    health.update(
        "model_router",
        ServiceStatus::Starting,
        Some("probing LLM server...".into()),
    );
    health.update(
        "tool_registry",
        ServiceStatus::Healthy,
        Some(format!("{} tools", tool_registry.len())),
    );
    health.update("agent_loop", ServiceStatus::Healthy, None);
    health.update("voice_pipeline", ServiceStatus::Healthy, None);
    health.update("embeddings", ServiceStatus::Healthy, None);
    health.update("vectors", ServiceStatus::Healthy, None);
    // MCP servers start in background — mark as starting
    health.register("mcp_servers");
    health.update(
        "mcp_servers",
        ServiceStatus::Starting,
        Some("connecting to MCP servers...".into()),
    );

    // Async probe of the LLM server — updates health once result is known
    // Wrap config in Arc<RwLock> early so both the probe and AppState share it
    let n8n_enabled_for_startup = config.n8n.enabled;
    let hardware_tier_for_startup = hardware_info.tier.as_str().to_string();
    let approved_workflows_for_startup = config
        .n8n
        .workflows
        .iter()
        .filter(|workflow| matches!(workflow.status, kria_core::n8n::N8nWorkflowStatus::Approved))
        .count();
    let config = Arc::new(RwLock::new(config));
    {
        let mr = model_router.clone();
        let health_mr = health.clone();
        let config_for_probe = config.clone();
        tokio::spawn(async move {
            let status = mr.status().await;
            // Use active_healthy: true when the *current routing mode's* backend
            // is reachable. For cloud/external modes this checks the cloud API,
            // not the local llama-server (which won't be running).
            let healthy = status["active_healthy"]
                .as_bool()
                .or_else(|| status["local_healthy"].as_bool())
                .unwrap_or(false);
            let mode = status["mode"].as_str().unwrap_or("local");
            if healthy {
                // For cloud modes, use the configured model ID directly.
                // detect_server_model() only works for local llama.cpp servers.
                let model_name = if mode == "local" {
                    match mr.detect_server_model().await {
                        Some(name) => {
                            config_for_probe.write().await.llm.active_model = name.clone();
                            name
                        }
                        None => status["local_model"]
                            .as_str()
                            .unwrap_or("unknown")
                            .to_string(),
                    }
                } else {
                    let cfg = config_for_probe.read().await;
                    let model = status["active_model"]
                        .as_str()
                        .filter(|value| !value.trim().is_empty() && *value != "unknown")
                        .map(str::to_string)
                        .unwrap_or_else(|| cfg.llm.cloud_model_id.clone());
                    let provider = cfg.llm.cloud_provider.clone();
                    drop(cfg);
                    if model.is_empty() {
                        format!("{} (cloud)", provider)
                    } else {
                        model
                    }
                };
                health_mr.update(
                    "model_router",
                    ServiceStatus::Healthy,
                    Some(format!("{}: {}", mode, model_name)),
                );
            } else {
                health_mr.update(
                    "model_router",
                    ServiceStatus::Degraded,
                    Some("LLM server not reachable".into()),
                );
            }
        });
    }
    // Automation subsystems
    let automation_dir = paths.data_dir.join("automation");
    let _ = std::fs::create_dir_all(&automation_dir);
    // Load persisted macros and workflows
    let mut macro_rec_inner = MacroRecorder::new();
    let _ = macro_rec_inner.load_from_file(&automation_dir.join("macros.json"));

    let scheduler_arc = Arc::new(RwLock::new(AutomationScheduler::new()));
    let macro_recorder_arc = Arc::new(RwLock::new(macro_rec_inner));

    tracing::info!("Automation subsystems initialized");

    // Store state in Tauri
    let telegram_bridge: Arc<RwLock<Option<TelegramBridge>>> = Arc::new(RwLock::new(None));

    // Auto-start Telegram bridge if configured.
    // If an enabled `telegram` MCP server is present, skip the built-in bridge
    // to avoid competing getUpdates long polls on the same bot token.
    let (telegram_config, telegram_mcp_enabled) = {
        let cfg = config.read().await;
        (
            cfg.telegram.clone(),
            cfg.mcp
                .servers
                .iter()
                .any(|s| s.enabled && s.name.eq_ignore_ascii_case("telegram")),
        )
    };
    if telegram_config.enabled
        && !telegram_config.bot_token.is_empty()
        && telegram_config.auto_start
    {
        if telegram_mcp_enabled {
            tracing::warn!(
                "Skipping built-in Telegram bridge auto-start because enabled MCP server 'telegram' already handles polling"
            );
        } else {
            tracing::info!("Auto-starting Telegram bridge");
            let bridge = TelegramBridge::spawn(
                telegram_config,
                agent_loop.clone(),
                memory_store.clone(),
                tool_registry.clone(),
                embeddings.clone(),
                vectors.clone(),
                hardware_info.tier.as_str().to_string(),
                orch_cell.clone(),
            );
            *telegram_bridge.write().await = Some(bridge);
        }
    }

    let (local_api_host, local_api_port) = {
        let cfg = config.read().await;
        (cfg.server.host.clone(), cfg.server.port)
    };
    let local_api_responder: Arc<dyn LocalApiResponder> = Arc::new(AgentLoopLocalApiResponder {
        agent_loop: agent_loop.clone(),
        memory_store: memory_store.clone(),
        tool_registry: tool_registry.clone(),
        embeddings: embeddings.clone(),
        vectors: vectors.clone(),
        hw_tier: hardware_info.tier.as_str().to_string(),
        orchestrator: orch_cell.clone(),
    });

    let voice_active = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let orchestrator_active_turns = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let orchestrator_last_activity_at =
        Arc::new(tokio::sync::Mutex::new(std::time::Instant::now()));

    let (colab_enabled, colab_server_name) = {
        let cfg = config.read().await;
        (cfg.colab.enabled, cfg.colab.mcp_server_name.clone())
    };
    let colab_runtime = Arc::new(RwLock::new(ColabRuntimeSnapshot::new(
        if colab_enabled {
            ColabRuntimeState::SidecarStarting
        } else {
            ColabRuntimeState::Disconnected
        },
        colab_server_name.clone(),
    )));
    let mcp_failure_history = Arc::new(RwLock::new(std::collections::HashMap::<
        String,
        Vec<McpFailureRecord>,
    >::new()));
    let ironclad_reset = Arc::new(RwLock::new(IroncladResetSnapshot::default()));
    let ironclad_forensic_log = Arc::new(RwLock::new(Vec::<IroncladForensicRecord>::new()));
    let (_, ironclad_system_config) = load_ironclad_system_config_with_path();
    let fleet_runtime_root = fleet_runtime_root_from_data_dir(paths.data_dir.as_path());
    std::fs::create_dir_all(&fleet_runtime_root)?;
    let fleet_qos = Arc::new(AdaptiveQosScheduler::new(&ironclad_system_config));
    let target_pool = Arc::new(TargetPool::new(
        &ironclad_system_config,
        SelectionWeights::default(),
        fleet_qos,
    ));
    target_pool.register_default_probes().await;
    let fleet_runtime = Arc::new(FleetRuntimeState::new(
        target_pool,
        ironclad_system_config,
        fleet_runtime_root,
    ));

    if colab_enabled {
        let colab_server_configured = {
            let cfg = config.read().await;
            cfg.mcp
                .servers
                .iter()
                .any(|s| s.enabled && s.name == colab_server_name)
        };

        if !colab_server_configured {
            let mut runtime = colab_runtime.write().await;
            runtime.state = ColabRuntimeState::Degraded;
            runtime.last_error = Some(format!(
                "Configured MCP server '{}' is missing or disabled",
                runtime.sidecar_server_name
            ));
        }
    }

    let fleet_control_runtime_for_bridge = fleet_control_runtime.clone();

    // ── Batch 1: PSDG — Wire PerceptionLoop + PsdgCoordinator ────────────────
    //
    // The WorldModelStore was opened earlier (world_model_early). Here we:
    // 1. Register PSDG health
    // 2. Start PerceptionLoop (inotify + D-Bus perception events)
    // 3. Subscribe PsdgCoordinator to PerceptionBus
    // 4. Schedule periodic fact decay
    let world_model = world_model_early;

    if let Some(ref psdg) = world_model {
        use kria_core::agent::perception::{PerceptionConfig, PerceptionLoop};
        use kria_core::agent::psdg::coordinator::{PsdgCoordinator, PsdgCoordinatorConfig};

        // Register PSDG health indicator
        health.register("psdg");
        health.update(
            "psdg",
            ServiceStatus::Healthy,
            Some("WorldModelStore + PerceptionLoop ready".into()),
        );

        // Create PerceptionLoop with default config (watches home dir + project dirs)
        let perception_loop = PerceptionLoop::new(PerceptionConfig::default());

        // Subscribe coordinator BEFORE starting the loop (order matters for broadcast)
        let perception_rx = perception_loop.bus().subscribe();
        let coordinator_cancel = tokio_util::sync::CancellationToken::new();

        // Spawn PsdgCoordinator
        let coordinator = PsdgCoordinator::new(psdg.clone(), PsdgCoordinatorConfig::default());
        let coord_handle = coordinator.spawn(perception_rx, coordinator_cancel.clone());
        tokio::spawn(async move {
            match coord_handle.await {
                Ok(()) => tracing::debug!("[PSDG] PsdgCoordinator exited"),
                Err(e) => tracing::warn!("[PSDG] PsdgCoordinator panicked: {}", e),
            }
        });

        // Spawn PerceptionLoop (drives desktop/fs/dbus events into PsdgCoordinator)
        let perception_cancel = tokio_util::sync::CancellationToken::new();
        let perception_cancel_clone = perception_cancel.clone();
        tokio::spawn(async move {
            perception_loop.run(perception_cancel_clone).await;
        });

        tracing::info!("[INIT] PSDG: PerceptionLoop + PsdgCoordinator started");

        // Schedule periodic PSDG fact decay (once per hour, background)
        let psdg_for_decay = psdg.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3600));
            interval.tick().await; // skip first tick (run at t=1h, not t=0)
            loop {
                interval.tick().await;
                psdg_for_decay.run_decay();
            }
        });
    }

    // RFC 008: Spawn the unified GUI service orchestrator (vision sidecar + uinput daemon).
    // Best-effort: if binaries are missing or sudo isn't configured, we set the field to
    // None and rely on the GlobalSafetyHalt (engaged by the orchestrator on failure) to
    // prevent any unsafe automation.
    let gui_orchestrator = match kria_core::orchestrator::OrchestratorConfig::auto_detect() {
        Ok(cfg) => {
            let orch = Arc::new(kria_core::orchestrator::ServiceOrchestrator::new(cfg));
            // start() is best-effort: individual service spawn failures are logged
            // and retried by the health monitor. We always store the orchestrator
            // so the health monitor can run and auto-restart failed services.
            if let Err(e) = orch.start().await {
                tracing::warn!(
                    "[INIT] GUI service orchestrator start warning: {e} — health monitor will retry"
                );
            } else {
                tracing::info!("[INIT] GUI service orchestrator started");
            }
            Some(orch)
        }
        Err(e) => {
            tracing::warn!("[INIT] GUI orchestrator auto-detect failed: {e} — automation disabled");
            kria_core::safety::engage_halt("orchestrator unavailable");
            None
        }
    };

    let decision_store =
        Arc::new(kria_core::agent::collaborative_decision::DecisionStore::default_persistent());
    let resume_executor = Arc::new(kria_core::agent::resume_executor::ResumeExecutor::new(
        tool_registry.clone(),
        policy_engine.clone(),
        decision_store.clone(),
        audit_logger.clone(),
    ));
    let continuation_reentry = Arc::new(
        kria_core::agent::continuation_reentry::ContinuationReentryService::new(
            decision_store.clone(),
            workflow_continuation_runtime.clone(),
        ),
    );
    let state = AppState {
        config,
        model_router,
        agent_loop,
        tool_registry: tool_registry.clone(),
        memory_store,
        hitl: hitl.clone(),
        decision_store: decision_store.clone(),
        policy_engine,
        resume_executor,
        continuation_reentry,
        workflow_continuation: workflow_continuation_runtime.clone(),
        event_bus: event_bus.clone(),
        sidecar,
        embeddings,
        vectors,
        current_session_id: Arc::new(RwLock::new(uuid::Uuid::new_v4().to_string())),
        voice_active: voice_active.clone(),
        voice_pipeline: Arc::new(RwLock::new(voice_pipeline)),
        active_voice: Arc::new(RwLock::new(active_voice_init)),
        voice_v2_telemetry: Arc::new(tokio::sync::Mutex::new(voice_v2_telemetry_init)),
        health: health.clone(),
        scheduler: scheduler_arc,
        macro_recorder: macro_recorder_arc,
        started_at: std::time::Instant::now(),
        hardware_info,
        proactive: proactive_engine,
        telegram_bridge,
        mcp_manager: mcp_manager.clone(),
        gw_client_ref: gw_client_ref.clone(),
        colab_runtime: colab_runtime.clone(),
        mcp_failure_history: mcp_failure_history.clone(),
        ironclad_reset: ironclad_reset.clone(),
        ironclad_forensic_log: ironclad_forensic_log.clone(),
        fleet_runtime: fleet_runtime.clone(),
        fleet_control_runtime,
        orchestrator: orch_cell.clone(),
        llm_runtime_apply_lock: Arc::new(tokio::sync::Mutex::new(())),
        llm_runtime_apply_status: Arc::new(RwLock::new(LlmRuntimeApplySnapshot::default())),
        orchestrator_active_turns: orchestrator_active_turns.clone(),
        orchestrator_last_activity_at: orchestrator_last_activity_at.clone(),
        image_orchestrator,
        skill_registry: openclaw_registry,
        container_pool: openclaw_pool,
        n8n_catalog: n8n_catalog.clone(),
        n8n_state_store: n8n_state_store.clone(),
        n8n_inbox_path: n8n_inbox_path.clone(),
        n8n_audit_path: n8n_audit_path.clone(),
        n8n_governance_log: n8n_governance_log.clone(),
        n8n_hitl_responses: n8n_hitl_responses.clone(),
        gui_cognition_hitl_proposals: Arc::new(RwLock::new(
            kria_core::agent::gui_cognition::safety_hitl::GuiHitlProposalStore::default(),
        )),
        gui_orchestrator,
        gui_cognition_observation_cache: Arc::new(tokio::sync::Mutex::new(None)),
        world_model,
    };

    if handle.state::<AppStateCell>().set(state).is_err() {
        tracing::error!("[INIT] AppState was already initialized — this is a bug");
    }

    tracing::info!("[INIT] AppState set — frontend is now unblocked");

    {
        let handle_status = handle.clone();
        let health_status = health.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                interval.tick().await;
                let health_snapshot = health_status.snapshot();
                let diagnostics_summary = kria_core::infra::diagnostics::diagnostics_summary();
                let recent_diagnostics =
                    kria_core::infra::diagnostics::recent_diagnostics(25, Some("warn"));

                let payload = serde_json::json!({
                    "emitted_at": chrono::Utc::now(),
                    "health": health_snapshot,
                    "diagnostics": {
                        "summary": diagnostics_summary,
                        "recent": recent_diagnostics,
                    },
                });

                if let Err(err) = handle_status.emit("runtime:status", payload) {
                    tracing::debug!(error = %err, "runtime status emit failed");
                }
            }
        });
    }

    // Restore previously enrolled targets into live TargetPool runtime.
    let handle_restore = handle.clone();
    let orch_for_restore = orch_cell.clone();
    let reset_for_restore = ironclad_reset.clone();
    let forensic_for_restore = ironclad_forensic_log.clone();
    let fleet_runtime_restore = fleet_runtime.clone();
    let registry_path_restore = paths
        .data_dir
        .join(TARGET_ENROLLMENT_REGISTRY_DIR)
        .join(TARGET_ENROLLMENT_REGISTRY_FILE);
    tokio::spawn(async move {
        let registry = match load_fleet_enrollment_registry(registry_path_restore.as_path()) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    code = ?error.code,
                    message = %error.message,
                    detail = ?error.detail,
                    "fleet runtime: failed to load enrollment registry during restore"
                );
                return;
            }
        };

        if registry.targets.is_empty() {
            return;
        }

        let mut admitted = 0usize;
        let mut skipped = 0usize;
        let mut failures: Vec<String> = Vec::new();

        for target in &registry.targets {
            match admit_enrolled_target_to_fleet_runtime(&fleet_runtime_restore, target).await {
                Ok(true) => admitted += 1,
                Ok(false) => skipped += 1,
                Err(error) => {
                    failures.push(format!("{} ({})", target.target_id, error));
                }
            }
        }

        if let Some(orch) = orch_for_restore.read().await.clone() {
            if let Err(error) = configure_orchestrator_fleet_bridge(&orch, &fleet_runtime_restore) {
                tracing::warn!(
                    error = %error,
                    "fleet runtime: failed to wire orchestrator bridge after restore"
                );
            } else {
                pulse_target_pool_telemetry(&fleet_runtime_restore.target_pool).await;
            }
        }

        if admitted > 0 || skipped > 0 {
            append_ironclad_forensic_record(
                &forensic_for_restore,
                &handle_restore,
                "fleet_runtime",
                "info",
                format!(
                    "Fleet runtime restore finished: admitted={} skipped={}",
                    admitted, skipped
                ),
                format!(
                    "registry_path={}; total_targets={}",
                    registry_path_restore.to_string_lossy(),
                    registry.targets.len()
                ),
                "desktop.fleet",
            )
            .await;
        }

        if !failures.is_empty() {
            append_ironclad_forensic_record(
                &forensic_for_restore,
                &handle_restore,
                "fleet_runtime",
                "warn",
                "Some enrolled targets failed runtime admission during restore".to_string(),
                failures.join(" | "),
                "desktop.fleet",
            )
            .await;
        }

        let status_payload = collect_ironclad_status_from_parts(
            &orch_for_restore,
            &reset_for_restore,
            &forensic_for_restore,
        )
        .await;
        let _ = handle_restore.emit("ironclad:status", status_payload);
    });

    // Emit periodic Ironclad status snapshots for non-blocking UI updates.
    let handle_ironclad_status = handle.clone();
    let orch_for_status = orch_cell.clone();
    let reset_for_status = ironclad_reset.clone();
    let forensic_for_status = ironclad_forensic_log.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(
            IRONCLAD_STATUS_EMIT_INTERVAL_SECS,
        ));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let payload = collect_ironclad_status_from_parts(
                &orch_for_status,
                &reset_for_status,
                &forensic_for_status,
            )
            .await;
            let _ = handle_ironclad_status.emit("ironclad:status", payload);
        }
    });

    // ── Background orchestrator startup (non-blocking) ────────────────────────
    // Spawning llama-server and waiting for /health can take 30-180 seconds.
    // We do it after AppState.set() so the UI is immediately responsive.
    if orch_enabled {
        let orch_cell_bg = orch_cell.clone();
        let model_router_bg = model_router_bg_ref.clone();
        let health_bg = health.clone();
        let event_bus_bg = event_bus.clone();
        let active_turns_bg = orchestrator_active_turns.clone();
        let last_activity_bg = orchestrator_last_activity_at.clone();
        let voice_active_bg = voice_active.clone();
        let handle_bg = handle.clone();
        let fleet_runtime_bg = fleet_runtime.clone();

        tokio::spawn(async move {
            tracing::info!("orchestrator: starting in background");
            match Orchestrator::start(
                orch_config,
                orch_model_path,
                orch_mmproj_path,
                event_bus_bg.clone(),
                health_bg.clone(),
            )
            .await
            {
                Ok(orch) => {
                    if let Err(error) =
                        configure_orchestrator_fleet_bridge(&orch, &fleet_runtime_bg)
                    {
                        tracing::warn!(
                            error = %error,
                            "orchestrator: failed to wire fleet runtime bridge"
                        );
                    } else {
                        pulse_target_pool_telemetry(&fleet_runtime_bg.target_pool).await;
                    }

                    // orch is Arc<Orchestrator> from Orchestrator::start()
                    // Wire server manager into model router (uses OnceLock — idempotent).
                    model_router_bg.attach_server_manager(orch.server_manager.clone());
                    tracing::info!(
                        backend = ?orch.backend,
                        api_url = %orch.api_url(),
                        "orchestrator: started and attached to model router"
                    );

                    // Publish to the UI that the LLM runtime is up.
                    let _ = handle_bg.emit(
                        "orchestrator:ready",
                        serde_json::json!({
                            "api_url": orch.api_url(),
                            "backend": format!("{:?}", orch.backend),
                        }),
                    );

                    // Start idle-release monitor if enabled.
                    if orch.config.idle_release_enabled {
                        let idle_after_secs = orch.config.idle_release_after_secs.max(30);
                        let check_interval_secs =
                            orch.config.idle_release_check_interval_secs.max(1);
                        let active_turns = active_turns_bg.clone();
                        let last_activity = last_activity_bg.clone();
                        let voice_active_idle = voice_active_bg.clone();
                        let handle_idle = handle_bg.clone();
                        let orch_idle = orch.clone();

                        tracing::info!(
                            idle_after_secs,
                            check_interval_secs,
                            "orchestrator: idle release monitor enabled"
                        );

                        tokio::spawn(async move {
                            let idle_after = std::time::Duration::from_secs(idle_after_secs);
                            let check_interval =
                                std::time::Duration::from_secs(check_interval_secs);
                            loop {
                                tokio::time::sleep(check_interval).await;
                                if voice_active_idle.load(std::sync::atomic::Ordering::Relaxed) {
                                    continue;
                                }
                                if active_turns.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                                    continue;
                                }
                                if orch_idle.server_manager.is_swapping() {
                                    continue;
                                }
                                if orch_idle.server_manager.current_params().0 == 0 {
                                    continue;
                                }
                                let idle_for = {
                                    let lock = last_activity.lock().await;
                                    lock.elapsed()
                                };
                                if idle_for < idle_after {
                                    continue;
                                }
                                if !orch_idle.server_manager.has_live_process().await {
                                    continue;
                                }
                                match orch_idle.release_if_idle("desktop_idle_timeout").await {
                                    Ok(true) => {
                                        let _ = handle_idle.emit(
                                            "orchestrator:idle_released",
                                            serde_json::json!({
                                                "idle_for_secs": idle_for.as_secs(),
                                                "mode": "unloaded"
                                            }),
                                        );
                                        touch_orchestrator_activity(&last_activity).await;
                                    }
                                    Ok(false) => {}
                                    Err(e) => {
                                        tracing::warn!(
                                            ?e,
                                            "orchestrator: idle release attempt failed"
                                        );
                                        touch_orchestrator_activity(&last_activity).await;
                                    }
                                }
                            }
                        });
                    }

                    // Start orchestrator event forwarder.
                    {
                        let handle_orch = handle_bg.clone();
                        let mut rx = event_bus_bg.subscribe();
                        tokio::spawn(async move {
                            use kria_core::infra::event_bus::KriaEvent;
                            loop {
                                match rx.recv().await {
                                    Ok(KriaEvent::LlmSwapStarted {
                                        from_ngl,
                                        to_ngl,
                                        emergency,
                                    }) => {
                                        let _ = handle_orch.emit(
                                            "orchestrator:swap_started",
                                            serde_json::json!({
                                                "from_ngl": from_ngl,
                                                "to_ngl": to_ngl,
                                                "emergency": emergency,
                                            }),
                                        );
                                    }
                                    Ok(KriaEvent::LlmSwapCompleted {
                                        new_ngl,
                                        new_context,
                                        duration_ms,
                                    }) => {
                                        let _ = handle_orch.emit(
                                            "orchestrator:swap_completed",
                                            serde_json::json!({
                                                "new_ngl": new_ngl,
                                                "new_context": new_context,
                                                "duration_ms": duration_ms,
                                            }),
                                        );
                                    }
                                    Ok(KriaEvent::LlmDegradationChanged { level }) => {
                                        let _ = handle_orch.emit(
                                            "orchestrator:degradation_changed",
                                            serde_json::json!({ "level": level }),
                                        );
                                    }
                                    Ok(KriaEvent::LlmStreamInterrupted) => {
                                        let _ = handle_orch.emit(
                                            "orchestrator:stream_interrupted",
                                            serde_json::json!({}),
                                        );
                                    }
                                    Ok(KriaEvent::VramPressure { free_vram_mb }) => {
                                        let _ = handle_orch.emit(
                                            "orchestrator:vram_pressure",
                                            serde_json::json!({ "free_vram_mb": free_vram_mb }),
                                        );
                                    }
                                    Ok(_) => {}
                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                        tracing::debug!(
                                            "orchestrator event forwarder lagged by {n}"
                                        );
                                    }
                                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                                }
                            }
                        });
                    }

                    // Finally, store the orchestrator in the shared cell so
                    // command handlers can access it via state.orchestrator.
                    *orch_cell_bg.write().await = Some(orch);
                }
                Err(e) => {
                    tracing::error!("orchestrator: failed to start (non-fatal): {e}");
                    health_bg.register("orchestrator");
                    health_bg.update(
                        "orchestrator",
                        ServiceStatus::Degraded,
                        Some(format!("{e}")),
                    );
                    let _ = handle_bg.emit(
                        "orchestrator:error",
                        serde_json::json!({ "error": e.to_string() }),
                    );
                }
            }
        });
    }

    start_local_api_bridge(
        local_api_host.clone(),
        local_api_port,
        local_api_responder,
        fleet_control_runtime_for_bridge,
        n8n_catalog.clone(),
        n8n_state_store.clone(),
        n8n_inbox_path.clone(),
        n8n_audit_path.clone(),
        n8n_governance_log.clone(),
        n8n_hitl_responses.clone(),
        hitl.clone(),
        decision_store.clone(),
        handle.clone(),
        health.clone(),
    );

    // ── Background MCP server startup (non-blocking) ──────────────────────────
    // MCP servers (especially npx-based ones) can take minutes to start.
    // They run in background and dynamically register tools into the thread-safe registry.
    {
        let tool_reg_bg = tool_registry.clone();
        let mcp_mgr_bg = mcp_manager.clone();
        let gw_ref_bg = gw_client_ref.clone();
        let colab_runtime_bg = colab_runtime.clone();
        let health_bg = health.clone();
        let handle_bg = handle.clone();
        tokio::spawn(async move {
            tracing::info!(
                target: "mcp_startup",
                "[MCP] Background provider startup scheduled"
            );
            let mut mgr = mcp_mgr_bg.lock().await;
            mgr.start_all(&tool_reg_bg).await;

            // Wire GW client if gworkspace server started successfully
            if let Some(live_client) = mgr.get_client("gworkspace") {
                gw::set_client(&gw_ref_bg, live_client.clone()).await;
                tracing::info!(
                    "[GW] GwClientRef populated — Google Workspace tools are now active"
                );
                let _ = handle_bg.emit("gw:connected", serde_json::json!({}));
            } else {
                tracing::warn!(
                    "[GW] gworkspace MCP server not available. \
                     Google Workspace tools will return 'not connected' errors."
                );
            }

            let statuses = mgr.status().await;

            let colab_server_name = {
                let runtime = colab_runtime_bg.read().await;
                runtime.sidecar_server_name.clone()
            };
            {
                let mut runtime = colab_runtime_bg.write().await;
                if runtime.state != ColabRuntimeState::Disconnected {
                    match statuses.iter().find(|s| s.name == colab_server_name) {
                        Some(status) if status.state == McpServerState::Running => {
                            let has_notebook = runtime
                                .selected_notebook
                                .as_ref()
                                .map(|value| !value.trim().is_empty())
                                .unwrap_or(false);

                            runtime.state = if status.tool_count == 0 {
                                runtime.selected_notebook = None;
                                ColabRuntimeState::AwaitingBrowserConnection
                            } else if has_notebook {
                                ColabRuntimeState::Ready
                            } else {
                                ColabRuntimeState::NotebookSelectionRequired
                            };
                            runtime.last_error = None;
                        }
                        Some(status) => {
                            runtime.state = ColabRuntimeState::Degraded;
                            runtime.last_error = status.error.clone().or_else(|| {
                                Some(format!(
                                    "MCP server '{}' is {}",
                                    colab_server_name,
                                    mcp_state_name(status.state)
                                ))
                            });
                        }
                        None => {
                            runtime.state = ColabRuntimeState::Degraded;
                            runtime.last_error = Some(format!(
                                "MCP server '{}' not found in runtime status",
                                colab_server_name
                            ));
                        }
                    }
                }
            }

            let running = statuses.iter().filter(|s| s.tool_count > 0).count();
            health_bg.update(
                "mcp_servers",
                ServiceStatus::Healthy,
                Some(format!(
                    "{}/{} servers running, {} total tools",
                    running,
                    statuses.len(),
                    tool_reg_bg.len()
                )),
            );

            let _ = handle_bg.emit(
                "mcp:ready",
                serde_json::json!({
                    "running": running,
                    "total": statuses.len(),
                    "tools": tool_reg_bg.len(),
                }),
            );

            {
                let runtime = colab_runtime_bg.read().await;
                let _ = handle_bg.emit(
                    "colab:status",
                    serde_json::json!({
                        "state": runtime.state.as_str(),
                        "server": runtime.sidecar_server_name,
                        "selected_notebook": runtime.selected_notebook,
                        "last_error": runtime.last_error,
                    }),
                );
            }

            tracing::info!(
                target: "mcp_startup",
                tools = tool_reg_bg.len(),
                "[MCP] background startup complete — {} tools available",
                tool_reg_bg.len()
            );

            // Start MCP health heartbeat (pings servers every 30s, auto-restarts on failure)
            drop(mgr);
            McpServerManager::spawn_health_heartbeat(mcp_mgr_bg, tool_reg_bg, 30);
        });
    }

    let startup_ms = startup_started.elapsed().as_millis();
    tracing::info!(
        target: "startup_summary",
        version = env!("CARGO_PKG_VERSION"),
        startup_ms,
        hardware_tier = %hardware_tier_for_startup,
        mcp_configured = total_servers,
        mcp_enabled = enabled_servers,
        approved_workflows = approved_workflows_for_startup,
        local_api = %format!("http://{}:{}", local_api_host, local_api_port),
        n8n_enabled = n8n_enabled_for_startup,
        "══════════════════════════════════\nKRIA Startup Summary\nVersion: {}\nSubsystems:\n✓ GUI Cognition\n✓ n8n Integration: {}\n✓ MCP Manager: {} enabled / {} configured\n✓ Local API: http://{}:{}\nWorkflows: {} approved\nHardware Tier: {}\nStartup Time: {}ms\n══════════════════════════════════",
        env!("CARGO_PKG_VERSION"),
        if n8n_enabled_for_startup { "enabled" } else { "disabled" },
        enabled_servers,
        total_servers,
        local_api_host,
        local_api_port,
        approved_workflows_for_startup,
        hardware_tier_for_startup,
        startup_ms
    );

    Ok(())
}

pub async fn shutdown_runtime(handle: &AppHandle) {
    let state_cell: tauri::State<'_, AppStateCell> = handle.state();
    let Some(state) = state_cell.get() else {
        tracing::info!("shutdown requested before runtime initialization finished");
        return;
    };

    tracing::info!("runtime shutdown started");

    // RFC 008: Engage global halt + shut down GUI orchestrator early so no further
    // automation tool calls can fire during the rest of the shutdown sequence.
    kria_core::safety::engage_halt("runtime shutdown");
    if let Some(gui_orch) = state.gui_orchestrator.as_ref() {
        gui_orch.shutdown().await;
    }

    state
        .voice_active
        .store(false, std::sync::atomic::Ordering::SeqCst);

    {
        let voice_pipeline = state.voice_pipeline.read().await.clone();
        voice_pipeline.stop().await;
    }

    {
        let mut bridge_guard = state.telegram_bridge.write().await;
        if let Some(bridge) = bridge_guard.take() {
            bridge.stop();
            tracing::info!("shutdown: telegram bridge stopped");
        }
    }

    {
        let mut manager = state.mcp_manager.lock().await;
        manager.stop_all(&state.tool_registry).await;
    }

    if let Err(e) = state.sidecar.shutdown().await {
        tracing::warn!("shutdown: failed to stop sidecar cleanly: {e}");
    }

    if let Some(orchestrator) = state.orchestrator.read().await.as_ref().cloned() {
        orchestrator.shutdown().await;
    }

    if let Some(pool) = state.container_pool.as_ref() {
        if let Err(e) = pool.shutdown().await {
            tracing::warn!("shutdown: container pool cleanup failed: {e}");
        } else {
            tracing::info!("shutdown: OpenClaw container pool destroyed");
        }
    }

    tracing::info!("runtime shutdown completed");
}

async fn replay_n8n_inbox(
    path: &Path,
    store: &kria_core::n8n::N8nWorkflowStateStore,
) -> anyhow::Result<usize> {
    if !path.exists() {
        return Ok(0);
    }

    let contents = tokio::fs::read_to_string(path).await?;
    let mut count = 0usize;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let record: kria_core::n8n::N8nInboxRecord = serde_json::from_str(trimmed)?;
        let _ = store.ingest(record.envelope);
        count += 1;
    }

    Ok(count)
}
