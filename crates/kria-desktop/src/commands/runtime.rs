use super::*;

pub async fn init_runtime(handle: &AppHandle) -> anyhow::Result<()> {
    // Initialize logging first so startup diagnostics are filterable.
    let bootstrap_paths = kria_core::platform::paths::KriaPaths::resolve();
    kria_core::infra::logging::setup_logging(&bootstrap_paths.logs_dir);

    let mut config = KriaConfig::load(None)?;
    let paths = config.resolve_paths()?;

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

    // Initialize model router from config
    let model_router = Arc::new(ModelRouter::from_config(&config));

    // EventBus (tokio broadcast channels)
    let event_bus = Arc::new(EventBus::new(256));

    // Health registry (created early so sidecar spawn can update it)
    let health = Arc::new(HealthRegistry::new());
    health.register("sidecar");
    health.update("sidecar", ServiceStatus::Starting, None);
    health.register("ocr_dependency");
    health.update(
        "ocr_dependency",
        ServiceStatus::Starting,
        Some("Probing OCR dependency readiness".into()),
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
        // 1. ~/.kria/models/llm/
        let p = paths.llm_models.join(filename);
        if p.exists() {
            return p.to_string_lossy().to_string();
        }
        // 2. Walk up from CWD to find workspace models/llm/ (Tauri dev runs from a sub-crate)
        if let Ok(cwd) = std::env::current_dir() {
            let mut dir = Some(cwd.as_path());
            while let Some(d) = dir {
                let candidate = d.join("models").join("llm").join(filename);
                if candidate.exists() {
                    return candidate.to_string_lossy().to_string();
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
    let (orch_model_path, orch_mmproj_path, orch_config, orch_enabled, selected_model_name) =
        if config.orchestrator.enabled {
            use kria_core::llm::orchestrator::tier_strategy::{
                derive_model_profile, select_model_for_tier, SelectionReason,
            };

            let model_exists = |file: &str| -> bool {
                let resolved = resolve_model_file(file);
                std::path::Path::new(&resolved).exists()
            };

            let choice = select_model_for_tier(
                hardware_info.tier,
                hardware_info.total_ram_mb,
                hardware_info.vram_mb,
                &config.llm.active_model,
                &config.llm.models,
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

    let _ = selected_model_name; // currently used only for logging above

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
    let tool_registry_inner = registry::build_registry_full(
        Some(memory_store.clone()),
        Some(rag_engine.clone()),
        Some(proactive_engine.clone()),
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
    tracing::info!("[MCP] loading MCP server configs from mcp_servers.json");
    {
        let mut cfg = config.clone();
        kria_core::config::load_mcp_servers(&mut cfg);
        config = cfg;
    }
    sync_telegram_mcp_server_config(&mut config);
    sync_google_workspace_server_config(&mut config, None);
    apply_google_runtime_env_from_config(&config);
    let total_servers = config.mcp.servers.len();
    let enabled_servers = config.mcp.servers.iter().filter(|s| s.enabled).count();
    tracing::info!(
        "[MCP] {} total MCP server(s) configured, {} enabled",
        total_servers,
        enabled_servers
    );
    for s in &config.mcp.servers {
        tracing::info!(
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

    // Wrap registry in Arc immediately — thread-safe for background MCP registration
    let tool_registry = Arc::new(tool_registry_inner);
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
    let hitl = Arc::new(HitlGateway::new(30));

    let policy_engine = Arc::new(PolicyEngine::new());

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
        routing_config,
        routing_cache_dir,
        router_tool_descriptions,
    )
    .await;

    // Build the agent loop
    let max_tool_rounds = config.agent.max_tool_rounds.max(1);
    let min_confidence_to_act = config.agent.min_confidence_to_act;
    let clarify_threshold = config.agent.clarify_threshold;
    let agent_loop = Arc::new(
        AgentLoop::new(
            model_router.clone(),
            tool_registry.clone(),
            mount_mgr,
            policy_engine,
            hitl.clone(),
            audit_logger,
            rollback_mgr,
        )
        .with_semantic_router(semantic_router)
        .with_max_tool_rounds(max_tool_rounds)
        .with_confidence_thresholds(min_confidence_to_act, clarify_threshold)
        .with_hardware_tier(hardware_info.tier.as_str()),
    );

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
    let config = Arc::new(RwLock::new(config));
    {
        let mr = model_router.clone();
        let health_mr = health.clone();
        let config_for_probe = config.clone();
        tokio::spawn(async move {
            let status = mr.status().await;
            let healthy = status["local_healthy"].as_bool().unwrap_or(false);
            if healthy {
                // Try to detect the actual model loaded on the server
                let model_name = match mr.detect_server_model().await {
                    Some(name) => {
                        // Update the config's active_model with the detected name
                        config_for_probe.write().await.llm.active_model = name.clone();
                        name
                    }
                    None => status["local_model"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string(),
                };
                health_mr.update(
                    "model_router",
                    ServiceStatus::Healthy,
                    Some(format!("model: {}", model_name)),
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
    // Sidecar/OCR dependency start as "starting" — updated when probes complete.
    health.update("sidecar", ServiceStatus::Starting, None);
    health.update(
        "ocr_dependency",
        ServiceStatus::Starting,
        Some("Waiting for sidecar OCR capability probe".into()),
    );

    // Automation subsystems
    let automation_dir = paths.data_dir.join("automation");
    let _ = std::fs::create_dir_all(&automation_dir);
    // Load persisted macros and workflows
    let mut macro_rec_inner = MacroRecorder::new();
    let _ = macro_rec_inner.load_from_file(&automation_dir.join("macros.json"));
    let mut workflow_engine = WorkflowEngine::new();
    let _ = workflow_engine.load_from_file(&automation_dir.join("workflows.json"));

    let scheduler_arc = Arc::new(RwLock::new(AutomationScheduler::new()));
    let macro_recorder_arc = Arc::new(RwLock::new(macro_rec_inner));
    let workflow_engine_arc = Arc::new(RwLock::new(workflow_engine));

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

    let state = AppState {
        config,
        model_router,
        agent_loop,
        tool_registry: tool_registry.clone(),
        memory_store,
        hitl,
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
        workflow_engine: workflow_engine_arc,
        started_at: std::time::Instant::now(),
        hardware_info,
        proactive: proactive_engine,
        telegram_bridge,
        mcp_manager: mcp_manager.clone(),
        gw_client_ref: gw_client_ref.clone(),
        colab_runtime: colab_runtime.clone(),
        ironclad_reset: ironclad_reset.clone(),
        ironclad_forensic_log: ironclad_forensic_log.clone(),
        fleet_runtime: fleet_runtime.clone(),
        fleet_control_runtime,
        orchestrator: orch_cell.clone(),
        orchestrator_active_turns: orchestrator_active_turns.clone(),
        orchestrator_last_activity_at: orchestrator_last_activity_at.clone(),
        image_orchestrator,
    };

    if handle.state::<AppStateCell>().set(state).is_err() {
        tracing::error!("[INIT] AppState was already initialized — this is a bug");
    }

    tracing::info!("[INIT] AppState set — frontend is now unblocked");

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
        local_api_host,
        local_api_port,
        local_api_responder,
        fleet_control_runtime_for_bridge,
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
            tracing::info!("[MCP] starting MCP servers in background (parallel)");
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
                "[MCP] background startup complete — {} tools available",
                tool_reg_bg.len()
            );

            // Start MCP health heartbeat (pings servers every 30s, auto-restarts on failure)
            drop(mgr);
            McpServerManager::spawn_health_heartbeat(mcp_mgr_bg, tool_reg_bg, 30);
        });
    }

    Ok(())
}

pub async fn shutdown_runtime(handle: &AppHandle) {
    let state_cell: tauri::State<'_, AppStateCell> = handle.state();
    let Some(state) = state_cell.get() else {
        tracing::info!("shutdown requested before runtime initialization finished");
        return;
    };

    tracing::info!("runtime shutdown started");

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
        manager.stop_all().await;
    }

    if let Err(e) = state.sidecar.shutdown().await {
        tracing::warn!("shutdown: failed to stop sidecar cleanly: {e}");
    }

    if let Some(orchestrator) = state.orchestrator.read().await.as_ref().cloned() {
        orchestrator.shutdown().await;
    }

    tracing::info!("runtime shutdown completed");
}
