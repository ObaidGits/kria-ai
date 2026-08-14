use super::*;
use crate::commands::colab::migrate_legacy_colab_server_command;
use std::collections::HashMap;

pub(super) fn spawn_executive_event_forwarding(
    app: AppHandle,
    mut events: tokio::sync::broadcast::Receiver<kria_core::agent::executive::ControllerEvent>,
) {
    use kria_core::agent::executive::ControllerEvent;
    tokio::spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    tracing::warn!(
                        count,
                        "Executive UI event bridge lagged; snapshot remains authoritative"
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let now = || chrono::Utc::now().to_rfc3339();
            match event {
                ControllerEvent::TaskStarted {
                    task_id,
                    priority,
                    source,
                    description,
                    ts,
                } => {
                    let _ = app.emit(
                        "executive:task_started",
                        serde_json::json!({
                            "task_id": task_id, "priority": priority, "source": source,
                            "description": description, "ts": ts,
                        }),
                    );
                }
                ControllerEvent::TaskCompleted {
                    task_id,
                    success,
                    duration_ms,
                    output_summary,
                    error,
                    ts,
                } => {
                    let _ = app.emit(
                        "executive:task_completed",
                        serde_json::json!({
                            "task_id": task_id, "success": success, "duration_ms": duration_ms,
                            "output_summary": output_summary, "error": error, "ts": ts,
                        }),
                    );
                }
                ControllerEvent::TaskPreempted {
                    victim_id,
                    victim_priority,
                    replacement_id,
                    replacement_priority,
                    ts,
                } => {
                    let _ = app.emit("executive:preemption", serde_json::json!({
                        "victim_id": victim_id, "victim_priority": victim_priority,
                        "replacement_id": replacement_id, "replacement_priority": replacement_priority,
                        "ts": ts,
                    }));
                }
                ControllerEvent::TaskRejected { task_id, reason } => {
                    let _ = app.emit(
                        "executive:task_completed",
                        serde_json::json!({
                            "task_id": task_id, "success": false, "duration_ms": 0,
                            "output_summary": null, "error": reason, "ts": now(),
                        }),
                    );
                }
                ControllerEvent::GpuLeaseAcquired { task_id } => {
                    let _ = app.emit(
                        "executive:gpu_lease",
                        serde_json::json!({
                            "task_id": task_id, "action": "acquired", "ts": now(),
                        }),
                    );
                }
                ControllerEvent::GpuLeaseReleased { task_id } => {
                    let _ = app.emit(
                        "executive:gpu_lease",
                        serde_json::json!({
                            "task_id": task_id, "action": "released", "ts": now(),
                        }),
                    );
                }
                ControllerEvent::VramMaintenanceStarted { .. }
                | ControllerEvent::VramMaintenanceCompleted { .. } => {}
            }
        }
    });
}

pub(super) fn spawn_memory_cognition_task(
    memory_system: Arc<kria_core::memory::api::MemorySystem>,
    app: AppHandle,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let monitor = std::sync::Arc::new(
            kria_core::memory::scheduler::DefaultResourceMonitor::new(512),
        );
        let scheduler = memory_system.cognitive_scheduler(monitor, None);
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(300));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut changes = memory_system.subscribe_changes();

        let emit = |change: &kria_core::memory::api::MemoryChange| {
            let _ = app.emit(&format!("memory://{}", change.kind), change.detail.clone());
            let _ = app.emit(
                "memory://changed",
                serde_json::json!({ "kind": change.kind, "detail": change.detail }),
            );
        };

        loop {
            let woke_by_change = tokio::select! {
                _ = tick.tick() => false,
                result = changes.recv() => match result {
                    Ok(change) => {
                        emit(&change);
                        while let Ok(more) = changes.try_recv() {
                            emit(&more);
                        }
                        true
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        tracing::warn!(lagged = count, "memory change subscriber lagged");
                        let _ = app.emit(
                            "memory://changed",
                            serde_json::json!({ "kind": "lagged" }),
                        );
                        true
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
            };

            if woke_by_change {
                tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
                while changes.try_recv().is_ok() {}
            }

            if !memory_system.is_enabled() {
                continue;
            }
            let ran = scheduler.run_ready().await;
            if ran > 0 {
                tracing::debug!(
                    jobs = ran,
                    woke_by_change,
                    "cognitive scheduler ran background jobs"
                );
            }
        }
    })
}

/// One-time migration (settings-config-revamp Task 5): if the SQLite config
/// store is empty and a legacy `~/.kria/config.toml` exists, import its
/// user-layer deviations into field-level rows and back up the file as
/// `config.toml.bak`. Idempotent: does nothing once the store is populated.
fn maybe_import_toml_into_store(
    store: &dyn kria_core::config::ConfigStore,
    secrets: Option<&kria_core::config::SecretStore>,
    paths: &kria_core::platform::paths::KriaPaths,
) {
    let is_empty = store.all().map(|r| r.is_empty()).unwrap_or(false);
    if !is_empty {
        return;
    }
    let toml_path = paths.user_config();
    if !toml_path.exists() {
        return;
    }
    let text = match std::fs::read_to_string(&toml_path) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "config import: could not read legacy config.toml");
            return;
        }
    };
    let user_cfg: kria_core::config::KriaConfig = match toml::from_str(&text) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "config import: could not parse legacy config.toml; skipping");
            return;
        }
    };
    // Migrate plaintext secrets first. Failure leaves both SQLite and the
    // legacy TOML untouched, allowing a clean retry on next startup.
    if let Some(secrets) = secrets {
        if let Err(e) = secrets.persist(&user_cfg) {
            tracing::error!(error = %e, "config import: failed to persist secrets; leaving config.toml intact");
            return;
        }
    }
    match user_cfg.write_user_layer_diff(store, "import") {
        Ok(()) => {
            let bak = toml_path.with_extension("toml.bak");
            if let Err(e) = std::fs::rename(&toml_path, &bak) {
                tracing::warn!(error = %e, "config import: rows written but backup rename failed");
            } else {
                tracing::info!(
                    backup = %bak.display(),
                    "config import: migrated ~/.kria/config.toml into SQLite config store"
                );
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "config import: failed to write user-layer rows; leaving config.toml intact");
        }
    }
}

pub async fn init_runtime(handle: &AppHandle) -> anyhow::Result<()> {
    // Initialize logging first so startup diagnostics are filterable.
    let startup_started = std::time::Instant::now();
    let bootstrap_paths = kria_core::platform::paths::KriaPaths::resolve();
    kria_core::infra::logging::setup_logging(&bootstrap_paths.logs_dir);

    // Durable settings-routing diagnostics (Task 11 observability): every settings
    // intent decision is appended as JSONL, bounded, surviving restart for debugging.
    kria_core::config::nl::diagnostics::set_persist_path(
        bootstrap_paths
            .logs_dir
            .join("settings_intent_routing.jsonl"),
    );

    // settings-config-revamp: backend-aware config load. Default `toml` keeps
    // legacy behaviour byte-for-byte. `sqlite` resolves the layered
    // (code < default.toml < DB < env) config and, on first run, imports the
    // existing ~/.kria/config.toml into field-level rows (Task 4/5).
    let config_backend = kria_core::config::ConfigBackend::from_env();
    let config_store: Option<std::sync::Arc<dyn kria_core::config::ConfigStore>> =
        match config_backend {
            kria_core::config::ConfigBackend::Sqlite => {
                match kria_core::config::SqliteConfigStore::open(&bootstrap_paths.db_path) {
                    Ok(store) => {
                        let store: std::sync::Arc<dyn kria_core::config::ConfigStore> =
                            std::sync::Arc::new(store);
                        Some(store)
                    }
                    Err(e) => {
                        // Fail closed: fall back to the TOML loader (Req 1.6).
                        tracing::error!(error = %e, "config: SQLite backend open failed; falling back to TOML");
                        None
                    }
                }
            }
            kria_core::config::ConfigBackend::Toml => None,
        };

    // Vault-backed secret store (Task 6) — only under the SQLite backend, where
    // secrets are kept out of the config rows and hydrated into the live config.
    let secret_store: Option<std::sync::Arc<kria_core::config::SecretStore>> = if config_store
        .is_some()
    {
        match kria_core::config::SecretStore::open_default() {
            Ok(s) => Some(std::sync::Arc::new(s)),
            Err(e) => {
                tracing::warn!(error = %e, "config: secret vault unavailable; secrets will not persist this session");
                None
            }
        }
    } else {
        None
    };

    // One-time import of legacy ~/.kria/config.toml into the SQLite store +
    // vault (runs after both are open so secrets migrate into the vault).
    if let Some(store) = &config_store {
        maybe_import_toml_into_store(store.as_ref(), secret_store.as_deref(), &bootstrap_paths);
    }

    let mut config = match &config_store {
        Some(store) => {
            let mut cfg = kria_core::config::KriaConfig::resolve_from_store(store.as_ref());
            if let Some(secrets) = &secret_store {
                secrets.hydrate(&mut cfg);
            }
            cfg
        }
        None => KriaConfig::load(None)?,
    };
    let paths = config.resolve_paths()?;
    // Apply GPU policy tunables (redesign G1/G2) from config at startup so Settings-UI values are
    // live from boot. Env vars override at read time.
    kria_core::llm::orchestrator::gpu_policy::apply_settings(
        config.orchestrator.gpu_autoscale,
        config.orchestrator.cuda_reserve_mb,
        config.orchestrator.vram_volatility_cap_mb,
    );
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

    // Initialize the unified memory authority backend. Conversations, derived
    // facts, snippets, and RAG chunks all live in the single authority DB
    // (`kria_memory.db`) via `KriaMemoryRuntime`, replacing the legacy
    // `MemoryStore` engine and eliminating the chat-vs-memory data split.
    let memory_backend = Arc::new(kria_core::memory::KriaMemoryRuntime::open(
        &paths.data_dir.join("kria_memory.db"),
    )?);
    let memory_store: Arc<dyn MemoryRuntime> = memory_backend.clone();

    // ── Durable reminder scheduler (Phase 2.3) ────────────────────────────────
    // Polls the persistent `reminders` table; fires due reminders via notify-send.
    // Survives restart: overdue reminders fire on the first poll after boot.
    match kria_core::tasks::TaskStore::open(&paths.db_path) {
        Ok(reminder_store) => {
            let dbus_addr = std::env::var("DBUS_SESSION_BUS_ADDRESS")
                .or_else(|_| {
                    std::env::var("XDG_RUNTIME_DIR").map(|d| format!("unix:path={}/bus", d))
                })
                .unwrap_or_else(|_| "unix:path=/run/user/1000/bus".to_string());
            let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":1".to_string());
            kria_core::tasks::spawn_reminder_scheduler(
                Arc::new(reminder_store),
                move |reminder| {
                    let _ = std::process::Command::new("notify-send")
                        .env("DBUS_SESSION_BUS_ADDRESS", &dbus_addr)
                        .env("DISPLAY", &display)
                        .args([
                            "-a",
                            "KRIA",
                            "-u",
                            "critical",
                            "-t",
                            "0",
                            "--icon=alarm",
                            "\u{23f0} KRIA Reminder",
                            &reminder.message,
                        ])
                        .spawn();
                },
                std::time::Duration::from_secs(30),
            );
            tracing::info!("[reminders] durable reminder scheduler armed (30s poll)");
        }
        Err(e) => {
            tracing::warn!(error = %e, "[reminders] durable scheduler disabled");
        }
    }

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
            match kria_core::openclaw::registry::SkillRegistry::open(&fallback) {
                Ok(r) => Arc::new(r),
                Err(e) => {
                    // Never crash the whole app because the OpenClaw registry DB can't
                    // open (e.g. unwritable data dir). Degrade to an in-memory registry
                    // so KRIA still boots; skills just won't persist this session.
                    tracing::error!(
                        error = %e,
                        "[OpenClaw] persistent skill registry unavailable — using in-memory \
                         registry (skills will not persist until the data dir is writable)"
                    );
                    Arc::new(
                        kria_core::openclaw::registry::SkillRegistry::open(std::path::Path::new(
                            ":memory:",
                        ))
                        .unwrap_or_else(|e2| {
                            panic!("in-memory skill registry must open (SQLite broken): {e2}")
                        }),
                    )
                }
            }
        };

    // Boot the ContainerPool only when explicitly enabled in user config.
    // Docker and the substrate image are optional; missing prerequisites should
    // disable OpenClaw cleanly instead of creating repeated background warnings.
    let openclaw_config = config.openclaw.clone();
    // TrustConfig enforcement fix (product gap 6/8): seed the live,
    // process-wide trust-config snapshot from the loaded config at boot, so
    // execute_semantic reads the correct persisted value even before the
    // user ever opens Settings (openclaw_update_settings keeps it hot after).
    kria_core::openclaw::trust_runtime::set_live_trust_config(openclaw_config.trust.clone());
    let openclaw_pool: Option<Arc<kria_core::openclaw::ContainerPool>> = if !openclaw_config.enabled
    {
        tracing::info!("[OpenClaw] container pool disabled by configuration");
        None
    } else {
        // Bounded boot retry: the Docker daemon is frequently still coming up when
        // KRIA launches (login/autostart race), which previously left the pool
        // permanently `None` for the whole session ("sometimes it starts, sometimes
        // it doesn't"). Retry a few times with short backoff so a transient
        // daemon-not-ready window self-heals without materially delaying boot. Uses
        // the (previously dead) `max_restart_attempts` config knob.
        let max_attempts = openclaw_config.max_restart_attempts.max(1);
        let mut booted: Option<Arc<kria_core::openclaw::ContainerPool>> = None;
        for attempt in 1..=max_attempts {
            match kria_core::openclaw::ContainerPool::new(openclaw_config.clone()).await {
                Ok(pool) => {
                    let pool = Arc::new(pool);
                    if let Err(e) = pool.verify_image_available().await {
                        tracing::warn!(
                            image = %openclaw_config.image,
                            attempt,
                            "[OpenClaw] container pool image unavailable: {e}"
                        );
                        // A missing image will not appear on retry — stop early.
                        break;
                    } else if let Err(e) = pool.initialize().await {
                        tracing::warn!(attempt, "[OpenClaw] container pool pre-warm failed: {e}");
                        // Pre-warm failure is often a transient daemon hiccup — retry.
                    } else {
                        kria_core::openclaw::ContainerPool::spawn_prewarm_loop(pool.clone());
                        tracing::info!(attempt, "[OpenClaw] container pool ready");
                        booted = Some(pool);
                        break;
                    }
                }
                Err(e) => {
                    tracing::info!(
                        attempt,
                        "[OpenClaw] container pool not ready yet (Docker starting?): {e}"
                    );
                }
            }
            if attempt < max_attempts {
                // Short backoff (0.75s, 1.5s, …) — bounded so boot is never blocked long.
                tokio::time::sleep(std::time::Duration::from_millis(750 * attempt as u64)).await;
            }
        }
        if booted.is_none() {
            tracing::warn!(
                attempts = max_attempts,
                "[OpenClaw] container pool unavailable after {max_attempts} attempt(s); \
                 use Settings → OpenClaw → Restart Substrate once Docker is up"
            );
        }
        booted
    };
    let openclaw_pool_slot = Arc::new(RwLock::new(openclaw_pool.clone()));

    // Initialize model router from config
    let model_router = Arc::new(ModelRouter::from_config(&config));

    // EventBus (tokio broadcast channels)
    let event_bus = Arc::new(EventBus::new(256));

    // Health registry (created early so sidecar spawn can update it)
    let health = Arc::new(HealthRegistry::new());
    health.register("sidecar");
    health.update("sidecar", ServiceStatus::Starting, None);
    // Unified health for the OpenClaw substrate — only surfaced when the user has
    // enabled it, so it never adds noise for users who don't use OpenClaw.
    if openclaw_config.enabled {
        health.register("openclaw");
        if openclaw_pool.is_some() {
            health.update(
                "openclaw",
                ServiceStatus::Healthy,
                Some("Substrate running".into()),
            );
        } else {
            health.update(
                "openclaw",
                ServiceStatus::Degraded,
                Some("Substrate unavailable — check Docker, then restart the substrate".into()),
            );
        }
    }
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
    let orchestrator_tasks: Arc<tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

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
    // Opens WorldModelStore against the shared kria.db path (WAL-safe).
    let world_model_early: Option<kria_core::agent::PsdgHandle> =
        match kria_core::agent::PsdgHandle::open(&paths.db_path) {
            Ok(handle) => {
                tracing::info!("[INIT] PSDG: WorldModelStore opened (WAL, shared kria.db)");
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
    // ── Unified cognitive Memory System (the intelligence backbone) ───────────
    // Shares the SINGLE authority DB handle with the conversation store / runtime
    // backend (one connection pool, L10) and reuses the already-loaded embedding
    // model (no double ONNX load). Every subsystem records observations/outcomes
    // and retrieves context through this, always via the Write Policy.
    let configured_memory_mode: kria_core::memory::types::MemoryMode = config
        .memory
        .modes
        .default
        .parse()
        .expect("MemoryMode parsing is infallible");
    let default_memory_mode = match configured_memory_mode {
        kria_core::memory::types::MemoryMode::Other(ref value) => {
            tracing::warn!(mode = %value, "unknown memory default mode; using permanent");
            kria_core::memory::types::MemoryMode::Permanent
        }
        mode => mode,
    };
    // Wire the desktop to the ONE memory composition root (`MemorySystem::compose`,
    // design §19.1 / F1.2.4). The authority `Database` handle is INJECTED from
    // the already-open backend, so the composition root — not the adapter — owns
    // every store/policy/retriever/scheduler over that single handle. Because the
    // handle is injected, `MemoryConfig.db_path` is unused here (it only applies
    // to the standalone self-opening path); we let it default rather than
    // duplicating the path ownership (retires the dual config/path smell).
    let memory_system = kria_core::memory::api::MemorySystem::compose(
        memory_backend.database(),
        kria_core::memory::api::MemoryConfig {
            enabled: config.memory.enabled,
            device_id: "local-desktop".to_string(),
            default_mode: default_memory_mode,
            admission_debounce: std::time::Duration::from_millis(
                config.memory.admission_debounce_ms,
            ),
            default_token_budget: config.memory.token_budget.max(1),
            enrichment_queue_capacity: config.memory.enrichment_queue_capacity.max(1),
            enrichment_catchup_interval: std::time::Duration::from_secs(
                config.memory.enrichment_catchup_secs.max(1),
            ),
            change_channel_capacity: config.memory.change_channel_capacity.max(1),
            ..Default::default()
        },
        Arc::new(kria_core::memory::embedding::OnnxEmbedder::from_model(
            embeddings.clone(),
        )),
        true,
    )?;
    memory_system.set_enabled(config.memory.enabled);

    // Distinct authenticated caller construction at the desktop adapter boundary
    // (F1.2.4). The desktop is an in-process, single-user laptop, so its caller
    // is the locally-trusted device. The server adapter constructs its own
    // `AuthenticatedRemote` caller at its own boundary — both share the SAME core
    // composition root wired above, but each authenticates its own caller. Full
    // per-operation caller threading through the governed write/read path lands
    // with the caller/policy model (F1.4).
    let caller = kria_core::memory::model::CallerContext::local_desktop(
        "local-desktop",
        kria_core::memory::model::PolicyPartition::new("user", "chat", 0)
            .expect("static desktop caller partition is valid"),
    )
    .expect("static desktop caller identity is valid");
    tracing::info!(
        enabled = config.memory.enabled,
        caller_origin = %caller.origin(),
        caller_partition = %caller.partition_key(),
        "[INIT] Memory System authority ready (one composition root; desktop caller at boundary)"
    );

    // Build the full tool registry (60+ tools + 6 precognitive) with the memory
    // runtime, unified MemorySystem, and Proactive. RAG/library tools are wired
    // from the MemorySystem itself (single retrieval pipeline; no RagEngine).
    let proactive_engine = Arc::new(kria_core::automation::ProactiveEngine::new(
        kria_core::automation::proactive::HealthThresholds::default(),
    ));
    let tool_registry_inner = registry::build_registry_full_with_memory(
        Some(memory_store.clone()),
        Some(proactive_engine.clone()),
        world_model_early.clone(),
        None,
        Some(memory_system.clone()),
    );
    // ── Durable OS audit ledger (OSC-007) ──────────────────────────────────────
    // Install BEFORE any OS action can be admitted. Without this the ledger falls
    // back to in-memory, which loses the record of an interrupted action so it
    // can never be reconciled after a restart. Installed unconditionally (not
    // behind `os-control-live`) so read-side admissions are durable too.
    {
        let audit_path = paths.data_dir.join("os_control_audit.db");
        match rusqlite::Connection::open(&audit_path) {
            Ok(conn) => {
                if kria_core::os_control::governed::init_audit_store(conn) {
                    tracing::info!(
                        target: "authority_trace",
                        path = %audit_path.display(),
                        "durable OS audit ledger installed"
                    );
                } else {
                    tracing::warn!(
                        target: "authority_trace",
                        "OS audit ledger already installed; keeping the first one"
                    );
                }
            }
            Err(error) => tracing::error!(
                target: "authority_trace",
                path = %audit_path.display(),
                error = %error,
                "could not open the durable OS audit ledger; OS actions will use a \
                 NON-durable in-memory ledger and interrupted actions cannot be reconciled"
            ),
        }
    }

    kria_core::tools::precognitive::register(&tool_registry_inner, sidecar.clone());
    kria_core::tools::news::register(&tool_registry_inner, sidecar.clone());

    // ── LIVE OS-control ignition (opt-in, `os-control-live` feature) ───────────
    // Without this the registry keeps its default `OsControlRuntime::detached()`
    // and every canonical OS action answers `Unavailable` — safe, but inert.
    // Composing the live aggregate here (the ONE composition root) is what lets a
    // prompt actually reach the host. Domains whose backend is absent stay
    // uncomposed and keep answering `Unavailable` rather than degrading to a
    // shell. Built only when the feature is enabled, and that feature is a hard
    // `compile_error!` alongside `os-control-test`, so no test binary links it.
    #[cfg(feature = "os-control-live")]
    {
        use kria_core::os_control::live::LiveHostOsControl;
        use kria_core::os_control::runtime::{HostOsControl, OsControlRuntime};

        // Probe the host over D-Bus first: a bus-backed domain composes only when
        // its service actually has an owner, so an absent NetworkManager or logind
        // reports Unavailable rather than failing on first call.
        let live_host = Arc::new(LiveHostOsControl::compose_probed().await);
        let domains = live_host.composed_domains().join(",");
        let revision = live_host
            .capability_snapshot()
            .map_or(0, |snapshot| snapshot.revision.0);
        tool_registry_inner.set_os_runtime(Arc::new(OsControlRuntime::with_host(live_host)));
        tracing::info!(
            target: "authority_trace",
            domains = %domains,
            capability_snapshot_revision = revision,
            "live OS-control aggregate composed and injected into the tool registry"
        );
    }
    // Re-register vision tools with sidecar (overrides the None-sidecar registration from build_registry)
    // ── Single shared GPU lease arbiter (HRA Tasks 13/14/15) ───────────────────
    // ONE process-wide lease manager shared by every GPU consumer (image, vision, and — via
    // `global_gpu_lease()` — voice/speech). This is the single-authority fix (Gap G1): consumers
    // contend on the SAME arbiter and can no longer collide on the GPU unknowingly.
    let shared_gpu_lease = kria_core::resource::gpu_lease::global_gpu_lease();

    // ── Single telemetry hub (HRA Phase A1 — telemetry unification) ────────────
    // ONE process-wide VRAM/RAM sampler owning the single device (NVML/ROCm) context. Every reader
    // — the shared lease's recovery telemetry, the HRA snapshot/admission path, and the dashboard —
    // borrows this hub's profiler / reads its published snapshot instead of opening its own context.
    // Created BEFORE the lease telemetry so `SharedResourceTelemetry::new()` binds to the hub. RAM
    // total is detected by the hub itself on its first sample (via sysinfo in kria-core); 0 is just
    // the pre-first-sample fallback.
    let telemetry_hub = kria_core::resource::TelemetryHub::new(0);
    kria_core::resource::set_global_telemetry_hub(telemetry_hub.clone());
    {
        // Background sampler: the single periodic VRAM poll for the whole process.
        let hub_run = telemetry_hub.clone();
        tokio::spawn(async move {
            hub_run.run(std::time::Duration::from_secs(5)).await;
        });
    }

    // Wire REAL resource telemetry into the shared lease so recovery/reconciliation verifies the
    // GPU actually freed after a release (instead of assuming). Prevents the lease from ever
    // self-degrading and blocking image/voice/vision (HRA production item 2). Sources the single
    // hub profiler (no second device context).
    shared_gpu_lease.set_resource_telemetry(std::sync::Arc::new(
        kria_core::resource::shared_telemetry::SharedResourceTelemetry::new(),
    ));

    kria_core::tools::vision::register(
        &tool_registry_inner,
        Some(sidecar.clone()),
        Some(shared_gpu_lease.clone()),
    );

    // ── Image generation orchestrator ─────────────────────────────────────────
    let image_cfg = config.image_generation.clone();
    let image_orchestrator =
        ImageOrchestrator::new_with_lease(image_cfg, &paths.data_dir, shared_gpu_lease.clone());
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
    let github_client_ref = gw::new_github_client_ref();
    tracing::info!("[GW] created lazy GwClientRef — registering Google Workspace tools now");
    gw::register(
        &tool_registry_inner,
        gw_client_ref.clone(),
        github_client_ref.clone(),
        sidecar.clone(),
    );

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

    // Register the semantic OpenClaw handler. Pass the model router so RC1
    // schema-driven argument generation can translate natural-language requests
    // into each skill's typed `inputSchema` arguments.
    if let Some(_subsystem) = &openclaw_subsystem {
        // ── M12 (Option A): the ONE execution pipeline. The `openclaw` chat tool
        //    and `list_installed_skills` are served entirely by the Capability
        //    Provider Platform — discover → permission (one engine + one durable
        //    grant store) → provider execution. The legacy SemanticOpenClawHandler
        //    / SemanticSkillRouter / ApprovalCache path is no longer wired.
        {
            use kria_core::capability::acl::openclaw::OpenClawProvider;
            use kria_core::capability::events::CapabilityEventBus;
            use kria_core::capability::grants::GrantStore as CapGrantStore;
            use kria_core::capability::index::{InMemoryFederatedIndex, MemoryEmbedder};
            use kria_core::capability::platform::CapabilityPlatform;
            use kria_core::capability::registry::ProviderRegistry as CapProviderRegistry;
            use kria_core::openclaw::runtime::SkillRuntime;
            use kria_core::safety::RiskLevel;
            use kria_core::tools::capability_dispatch::{
                CapabilityDispatchHandler, CapabilityListHandler, MarketplaceInstallHandler,
                MarketplaceSearchHandler,
            };
            use kria_core::tools::registry::{ParamDef, ToolDef};

            // Reuse the shared embedding model (no second backend).
            let cap_embedder = Arc::new(MemoryEmbedder::from_model(embeddings.clone(), 384));
            let cap_index = Arc::new(InMemoryFederatedIndex::new(cap_embedder));
            let cap_registry = Arc::new(CapProviderRegistry::new(cap_index));
            let cap_runtime: Arc<dyn SkillRuntime> = Arc::new(
                super::feature_controls::HotSwapDockerRuntime::new(openclaw_pool_slot.clone()),
            );
            let mut oc = OpenClawProvider::new(openclaw_registry.clone(), cap_runtime);
            let store_dir = paths.data_dir.join("openclaw_skills");
            let _ = std::fs::create_dir_all(&store_dir);
            if let Ok(audit) = kria_core::openclaw::audit::AuditLedger::open(
                &paths.data_dir.join("skills.db"),
                b"kria-openclaw-dev-audit-key-0001".to_vec(),
            ) {
                oc = oc.with_lifecycle(
                    openclaw_config.registry.index_url.clone(),
                    openclaw_config.registry.allowed_hosts.clone(),
                    Arc::new(audit),
                    store_dir,
                );
            }
            cap_registry.register(Arc::new(oc));
            let cap_bus = Arc::new(CapabilityEventBus::new(512));
            let mut cap_platform = CapabilityPlatform::new(cap_registry).with_events(cap_bus);
            // P1: wire the durable Capability Knowledge Base when enabled. Flag OFF
            // ⇒ platform behaves exactly as before (flag-off parity). When ON, every
            // execution outcome is recorded for reuse/ranking/grounding (spec R1).
            let mut jobs_store: Option<
                Arc<kria_core::capability::intelligence::SqliteCapabilityKnowledge>,
            > = None;
            if config.capability.intelligence.ckb {
                match kria_core::capability::intelligence::SqliteCapabilityKnowledge::open(
                    &paths.data_dir.join("cpp_knowledge.db"),
                ) {
                    Ok(ckb) => {
                        let ckb = Arc::new(ckb);
                        cap_platform = cap_platform.with_knowledge(ckb.clone());
                        jobs_store = Some(ckb.clone());
                        // P8: expose the same CKB as its EvolutionStore facet when
                        // evolution is enabled, so health/proposals persist to the
                        // one learned layer (no parallel store).
                        if config.capability.intelligence.evolution {
                            cap_platform = cap_platform.with_evolution_store(ckb.clone());
                            tracing::info!(
                                "[CPP] Evolution store wired (health + benchmarks + proposals, spec R6/R18)"
                            );
                        }
                        tracing::info!(
                            "[CPP] Capability Knowledge Base wired (cpp_knowledge.db) — learning enabled"
                        );
                    }
                    Err(e) => tracing::warn!("[CPP] CKB open failed (continuing without): {e}"),
                }
            }
            // P6: wire Wave 6 marketplace intelligence when enabled. Flag OFF ⇒
            // legacy index-only recommendation (flag-off parity). When ON, catalog
            // recommendations fuse neutral trust/quality/cost/adoption signals and
            // provider catalogs are TTL-cached (spec R8).
            if config.capability.intelligence.marketplace_v2 {
                use kria_core::capability::intelligence::{CatalogRanker, CatalogRankingPolicy};
                cap_platform = cap_platform.with_marketplace_v2(
                    CatalogRanker::new(CatalogRankingPolicy::default()),
                    std::time::Duration::from_secs(300),
                );
                tracing::info!(
                    "[CPP] Marketplace intelligence v2 wired — neutral catalog ranking + TTL cache (spec R8)"
                );
            }
            // P9: register the synthesizing provider + enable synthesis
            // fall-through so a real chat turn with no candidate can GENERATE a
            // capability (spec R7/Wave 9). Flag-gated + off critical path.
            if config.capability.intelligence.synthesis {
                let syn_store = paths.data_dir.join("cpp_synthesis");
                match kria_core::capability::acl::synthesis::SynthesisProvider::new(
                    "synthesis",
                    &syn_store,
                ) {
                    Ok(p) => {
                        cap_platform.registry().register(Arc::new(p));
                        cap_platform = cap_platform.with_synthesis("synthesis");
                        // W9-R11: LLM-assisted IR proposer (flag-gated); validator
                        // + golden gate own correctness; deterministic fallback.
                        if config.capability.intelligence.synthesis_llm {
                            let generator = crate::commands::capability::SynthesisLlmGenerator::new(
                                model_router.clone(),
                            );
                            cap_platform = cap_platform.with_ir_proposer(Arc::new(
                                kria_core::capability::intelligence::LlmIrProposer::new(generator)
                                    .with_code(config.capability.intelligence.synthesis_code),
                            ));
                            tracing::info!("[CPP] LLM-assisted IR proposer wired (synthesis_llm)");
                        }
                        if config.capability.intelligence.synthesis_code {
                            cap_platform = cap_platform.with_code_runner(Arc::new(
                                kria_core::capability::acl::code_sandbox::CodeSandbox::default(),
                            ));
                            tracing::info!("[CPP] Tier-3 code sandbox wired (synthesis_code)");
                        }
                        tracing::info!(
                            "[CPP] Synthesis provider wired — Brain can generate capabilities (spec R7)"
                        );
                    }
                    Err(e) => tracing::warn!("[CPP] synthesis provider unavailable: {e}"),
                }
            }
            let platform = Arc::new(cap_platform);
            platform.refresh().await;

            // Wave 11: wire the durable job manager at boot + resume active jobs
            // (restart recovery, spec R28). Idempotent global.
            if config.capability.intelligence.jobs {
                if let Some(store) = &jobs_store {
                    crate::commands::capability::ensure_jobs_spawned(
                        platform.clone(),
                        store.clone(),
                        8,
                    );
                }
            }

            // Wave 10: spawn the continuous discovery/maintenance loop at boot
            // (background, off-by-default, autonomy-gated). Idempotent global.
            if config.capability.intelligence.continuous_discovery {
                let autonomy = kria_core::capability::intelligence::AutonomyLevel::parse(
                    &config.capability.intelligence.autonomy_level,
                )
                .unwrap_or(kria_core::capability::intelligence::AutonomyLevel::ProposeOnly);
                crate::commands::capability::ensure_discovery_spawned(platform.clone(), autonomy);
            }

            // `list_installed_skills` — CPP-backed, provider-neutral.
            let list_def = ToolDef {
                name: "list_installed_skills".to_string(),
                description: "List capabilities that are actually installed/available right now \
                    across all providers (OpenClaw, MCP, ...). Use this to answer ANY question \
                    about whether a capability is installed — never answer from memory."
                    .to_string(),
                category: "openclaw".to_string(),
                parameters: vec![ParamDef {
                    name: "filter".to_string(),
                    param_type: "string".to_string(),
                    description: "all|enabled|disabled (default all)".to_string(),
                    required: false,
                    default: None,
                }],
                default_tier: RiskLevel::Green,
                min_tier: "lite",
            };
            tool_registry.register(
                list_def,
                Arc::new(CapabilityListHandler::new(platform.clone())),
            );

            // `search_marketplace` — provider-neutral marketplace search (remote,
            // installable capabilities). This is what "search the marketplace for
            // X" / "find a tool that does X" resolves to — NOT the OS package
            // manager, NOT the installed list.
            let search_def = ToolDef {
                name: "search_marketplace".to_string(),
                description: "Search the capability MARKETPLACE for installable skills/tools that \
                    are NOT yet installed (e.g. a PDF extractor, a zip compressor, an OCR tool). \
                    Use this whenever the user wants to find, discover, or look for a new \
                    tool/skill/capability to add. Returns ranked remote candidates; install one \
                    with `install_capability`. This is NOT the OS package manager and NOT the \
                    installed-skills list."
                    .to_string(),
                category: "openclaw".to_string(),
                parameters: vec![ParamDef {
                    name: "query".to_string(),
                    param_type: "string".to_string(),
                    description: "What kind of capability to look for (natural language)"
                        .to_string(),
                    required: true,
                    default: None,
                }],
                default_tier: RiskLevel::Green,
                min_tier: "lite",
            };
            tool_registry.register(
                search_def,
                Arc::new(MarketplaceSearchHandler::new(platform.clone())),
            );

            // `install_capability` — install the best marketplace match for a
            // natural-language goal, then make it immediately usable. No skill
            // names required from the user.
            let install_def = ToolDef {
                name: "install_capability".to_string(),
                description: "Install a new skill/tool/capability from the MARKETPLACE by \
                    describing what it should do (e.g. 'install a PDF extractor', 'install a zip \
                    compressor', 'add an OCR tool'). The best marketplace match is installed and \
                    becomes immediately available — the user does NOT need to know the exact skill \
                    name. Use this for any 'install/add/get me a tool that ...' request. This is \
                    NOT the OS package manager (that is `install_package`)."
                    .to_string(),
                category: "openclaw".to_string(),
                parameters: vec![ParamDef {
                    name: "query".to_string(),
                    param_type: "string".to_string(),
                    description: "Describe the capability to install (natural language)"
                        .to_string(),
                    required: true,
                    default: None,
                }],
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
            };
            tool_registry.register(
                install_def,
                Arc::new(MarketplaceInstallHandler::new(platform.clone())),
            );

            match CapGrantStore::open(&paths.data_dir.join("cpp_grants.db")) {
                Ok(store) => {
                    let dispatcher = CapabilityDispatchHandler::new(platform, Arc::new(store))
                        .with_arg_llm(model_router.clone())
                        .with_reasoner(config.capability.intelligence.reasoner);
                    let def = ToolDef {
                        name: "openclaw".to_string(),
                        description: "Run a capability to actually DO a task that needs execution \
                            rather than just an answer (calculation, parsing/converting files, \
                            fetching/searching the web, processing data, etc.). Automatically \
                            discovers and runs the best-matching capability across all providers \
                            through the Capability Provider Platform, gated by one unified \
                            permission engine, and returns a real verified result or an honest \
                            'no matching capability'."
                            .to_string(),
                        category: "openclaw".to_string(),
                        parameters: vec![ParamDef {
                            name: "query".to_string(),
                            param_type: "string".to_string(),
                            description: "Describe what you want to accomplish".to_string(),
                            required: true,
                            default: None,
                        }],
                        default_tier: RiskLevel::Green,
                        min_tier: "lite",
                    };
                    tool_registry.register(def, Arc::new(dispatcher));
                    tracing::info!(
                        "[CPP] chat tools (`openclaw`, `list_installed_skills`) served by CapabilityPlatform (Option-A single pipeline)"
                    );
                }
                Err(e) => tracing::warn!(
                    "[CPP] grant store unavailable ({e}); openclaw tool not registered"
                ),
            }
        }

        // RC2: synchronize the registry from the container's authoritative
        // `tools/list` so EVERY baked/installed skill is routable with its real
        // schema. Background + non-fatal: needs Docker and adds container
        // latency, so it must never block or fail boot — OpenClaw keeps
        // whatever the registry already had if the container is unreachable.
        if let Some(sync_pool) = openclaw_pool.clone() {
            let sync_registry = openclaw_registry.clone();
            tokio::spawn(async move {
                match kria_core::openclaw::init::sync_registry_from_container(
                    &sync_registry,
                    sync_pool,
                )
                .await
                {
                    Ok(n) => {
                        tracing::info!(changed = n, "[OpenClaw] registry↔container sync complete")
                    }
                    Err(e) => tracing::warn!("[OpenClaw] registry↔container sync skipped: {e}"),
                }
            });
        }
    }

    tracing::info!(
        tools = tool_registry.len(),
        "[INIT] base tool registry ready ({} tools, MCP tools will be added in background)",
        tool_registry.len()
    );

    // Create MCP manager with effective startup gates. Persisted per-server
    // preferences remain unchanged; disabled masters/integrations spawn nothing.
    let mut mcp_configs = config.mcp.servers.clone();
    for server in &mut mcp_configs {
        let integration_enabled = if server.name.eq_ignore_ascii_case("telegram") {
            config.telegram.enabled
        } else if server.name == config.colab.mcp_server_name {
            config.colab.enabled
        } else {
            true
        };
        server.enabled = server.enabled && config.mcp.enabled && integration_enabled;
    }
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

    // Phase 3: Build tool-level semantic index.
    // C6 (startup optimization): create the index EMPTY (instant) and populate it on a background
    // task, so the ~seconds-long embedding build no longer blocks startup / the LLM spawn. While it
    // builds, semantic tool matching returns None and routing falls back to the lexical/ONNX path —
    // self-healing once the background rebuild completes.
    let tool_defs_for_index: Vec<kria_core::tools::registry::ToolDef> =
        tool_registry.list_defs().to_vec();
    let tool_index = kria_core::routing::tool_index::SharedToolIndex::empty();
    {
        let ti_bg = tool_index.clone();
        let cfg_bg = routing_config.clone();
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            match ti_bg.rebuild(tool_defs_for_index, cfg_bg).await {
                Ok(()) => tracing::info!(
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "Tool semantic index built in background — semantic routing now active (C6)"
                ),
                Err(e) => tracing::warn!(
                    error = %e,
                    "Tool semantic index background build failed — routing uses lexical fallback"
                ),
            }
        });
    }
    tracing::info!("Tool semantic index: building in background (non-blocking startup, C6)");

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
    .with_execution_verifier(execution_verifier)
    .with_memory_system(memory_system.clone());

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
    // settings-config-revamp: single config reader/writer over the SAME handle
    // + event bus. Uses the SQLite store when the backend is `sqlite`, else the
    // whole-file TOML persist. Behaviourally inert unless KRIA_CONFIG_SERVICE
    // routes reads/writes through it.
    let config_service = Arc::new(match config_store.clone() {
        Some(store) => kria_core::config::ConfigService::with_store_and_secrets(
            config.clone(),
            event_bus.clone(),
            store,
            secret_store.clone(),
        ),
        None => kria_core::config::ConfigService::new(config.clone(), event_bus.clone()),
    });
    // settings-config-revamp Task 15: durably record every committed config change
    // into the hash-chained audit ledger (in addition to the in-memory undo ring).
    config_service.set_audit_sink(audit_logger.clone());

    // Clone for the Task 8 effect-executor subscription (config_service itself is
    // moved into AppState below).
    let config_service_for_effects = config_service.clone();
    // Wire the ConfigService into the tool registry so the `config_patch` agent
    // tool can read/apply config from its ToolContext.
    tool_registry.set_config_service(config_service.clone());
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
            let gui_enabled = config.read().await.gui_cognition.enabled;
            if gui_enabled {
                // start() is best-effort: individual service spawn failures are logged
                // and retried by the health monitor.
                if let Err(e) = orch.start().await {
                    tracing::warn!(
                        "[INIT] GUI service orchestrator start warning: {e} — health monitor will retry"
                    );
                } else {
                    tracing::info!("[INIT] GUI service orchestrator started");
                }
            } else {
                orch.set_automation_enabled(false).await.ok();
                tracing::info!("[INIT] GUI Cognition disabled; sidecars skipped");
            }
            Some(orch)
        }
        Err(e) => {
            tracing::warn!("[INIT] GUI orchestrator auto-detect failed: {e} — automation disabled");
            kria_core::safety::engage_halt("orchestrator unavailable");
            None
        }
    };

    // OS-action decisions are SQLite-durable (OSC-001.9): bind the decision
    // store's native-OS authority to the same shared `kria.db` the audit log
    // uses, so a UI approval is ineffective until its resolution commits.
    let decision_store = Arc::new(
        kria_core::agent::collaborative_decision::DecisionStore::default_persistent_with_db(
            &paths.db_path,
        ),
    );
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
    // Conversation store is vended by the single memory composition root
    // (`MemorySystem`), so chat/session/preference/media writes and
    // cognitive-memory writes flow through one authority handle instead of the
    // adapter constructing a store independently (F1.2.4 — one authority per
    // process, all memory access derived from the core root).
    let conversation = std::sync::Arc::new(memory_system.conversation());

    // ── Event-driven cognition loop + live UI bridge (design §20/§25, P8) ─────
    // The Cognitive Scheduler owns consolidation/reflection/dreaming. It is
    // resource-gated (suspends on battery / memory pressure) and single-flight
    // (one task, `run_ready()` called once per iteration). Instead of a pure
    // timer it now WAKES on memory-change events: every committed write, delete,
    // update, relationship, goal/plan change, or cognition completion flows
    // through `MemorySystem::subscribe_changes()`. Each change is (1) bridged to
    // the frontend as a `memory://<kind>` + `memory://changed` Tauri event for
    // instant live UI updates, and (2) coalesced (~1.2s) before waking cognition
    // to avoid event storms. The 300s tick remains as an idle fallback so
    // idle-only jobs (dreaming) still fire when nothing is happening.
    let memory_cognition_task = memory_system
        .is_enabled()
        .then(|| spawn_memory_cognition_task(memory_system.clone(), handle.clone()));
    let quarantine_registry = Arc::new(
        kria_core::tools::quarantine::QuarantineRegistry::open_path(&paths.db_path)?,
    );
    let executive_settings = config.read().await.executive.clone();
    let executive_sender = if executive_settings.enabled {
        let executive_config = kria_core::agent::executive::ExecutiveConfig {
            max_background_tasks: executive_settings.max_background_tasks,
            preemption_grace_ms: executive_settings.preemption_grace_ms,
            ..Default::default()
        };
        let policy_gate: Arc<dyn kria_core::safety::policy_gate::PolicyGate> =
            Arc::new(kria_core::safety::policy_gate::CapabilityPolicyGate::new());
        let (mut controller, sender) = kria_core::agent::executive::ExecutiveController::new(
            executive_config,
            shared_gpu_lease.clone(),
            policy_gate,
        );
        spawn_executive_event_forwarding(handle.clone(), controller.subscribe_events());
        tokio::spawn(async move { controller.run().await });
        tracing::info!("ExecutiveController enabled — desktop dispatch loop started");
        Some(sender)
    } else {
        tracing::info!("ExecutiveController disabled — desktop uses AgentLoop authority");
        None
    };

    let state = AppState {
        config,
        config_service,
        audit_logger: audit_logger.clone(),
        model_router,
        agent_loop,
        executive_sender: Arc::new(RwLock::new(executive_sender)),
        tool_registry: tool_registry.clone(),
        quarantine_registry,
        memory_store,
        conversation,
        memory_system,
        caller,
        memory_cognition_task: tokio::sync::Mutex::new(memory_cognition_task),
        cold_start_cancel: Arc::new(std::sync::Mutex::new(None)),
        hitl: hitl.clone(),
        decision_store: decision_store.clone(),
        policy_engine,
        resume_executor,
        continuation_reentry,
        workflow_continuation: workflow_continuation_runtime.clone(),
        event_bus: event_bus.clone(),
        sidecar,
        embeddings,
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
        gpu_lease: shared_gpu_lease.clone(),
        proactive: proactive_engine,
        telegram_bridge,
        mcp_manager: mcp_manager.clone(),
        mcp_heartbeat: tokio::sync::Mutex::new(None),
        gw_client_ref: gw_client_ref.clone(),
        colab_runtime: colab_runtime.clone(),
        mcp_failure_history: mcp_failure_history.clone(),
        ironclad_reset: ironclad_reset.clone(),
        ironclad_forensic_log: ironclad_forensic_log.clone(),
        fleet_runtime: fleet_runtime.clone(),
        fleet_control_runtime,
        orchestrator: orch_cell.clone(),
        orchestrator_tasks: orchestrator_tasks.clone(),
        llm_runtime_apply_lock: Arc::new(tokio::sync::Mutex::new(())),
        llm_runtime_apply_status: Arc::new(RwLock::new(LlmRuntimeApplySnapshot::default())),
        orchestrator_active_turns: orchestrator_active_turns.clone(),
        orchestrator_last_activity_at: orchestrator_last_activity_at.clone(),
        image_orchestrator,
        skill_registry: openclaw_registry.clone(),
        container_pool: openclaw_pool_slot,
        feature_controls: Arc::new(super::feature_controls::FeatureControlRuntime::new()),
        n8n_maintenance: tokio::sync::Mutex::new(None),
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
        world_model,
    };

    if handle.state::<AppStateCell>().set(state).is_err() {
        tracing::error!("[INIT] AppState was already initialized — this is a bug");
    }

    tracing::info!("[INIT] AppState set — frontend is now unblocked");
    super::feature_controls::initialize(handle).await;

    // Task 13.4 (R10.2): push-sync event bridge for the frozen `RegistryEvent`
    // stream. Wired here (not in `main.rs::setup`) because `RegistryEvent` is
    // emitted per-registry-instance (unlike the process-global bundle/execution
    // buses), so it needs the live `ProductionSkillRegistry` handle that only
    // exists after runtime init. Subscribes to the SAME frozen registry
    // broadcast the registry already emits to — no second event system. The UI
    // reconciles any missed event by polling the authoritative list commands.
    crate::commands::openclaw::spawn_openclaw_registry_forwarding(
        handle.clone(),
        openclaw_registry.clone(),
    );

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
        let hra_data_dir_bg = paths.data_dir.clone();
        let orchestrator_tasks_bg = orchestrator_tasks.clone();

        let startup_task = tokio::spawn(async move {
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

                    // Staged readiness (redesign G8): the LLM is the ONLY critical-path
                    // subsystem. Emit `core_llm_ready` independently so the UI can become
                    // interactive immediately, without waiting on the background-loaded
                    // tool index (C6), voice warmup, or MCP providers. Those subsystems
                    // emit their own readiness events when they finish loading.
                    let _ = handle_bg.emit(
                        "runtime:core_llm_ready",
                        serde_json::json!({
                            "stage": "core_llm",
                            "backend": format!("{:?}", orch.backend),
                            "critical_path": true,
                        }),
                    );

                    // ── HRA (Hardware & Resource Authority) — SHADOW MODE ─────────────
                    // Additive: construct the Resource Authority service and run it in
                    // shadow (records decisions + compares to the legacy path, emits
                    // status for the UI) WITHOUT gating real admission. The legacy
                    // orchestrator/lease paths remain authoritative. This is the safe
                    // first cutover step (HRA Tasks 3/10/12-shadow/37). Flipping a
                    // consumer to honor the RA is done later via the per-consumer bypass
                    // switch once the shadow comparator gate is clean.
                    {
                        use kria_core::resource::authority::{
                            ConsumerId, HraService, PolicyProfile,
                        };

                        let hw = tokio::task::spawn_blocking(
                            kria_core::infra::hardware_profiler::profile_hardware,
                        )
                        .await
                        .ok();
                        let (gpu_total_vram, ram_total) = hw
                            .as_ref()
                            .map(|h| (h.info.vram_mb.unwrap_or(0), h.info.total_ram_mb))
                            .unwrap_or((0, 0));

                        let gpus: Vec<(u32, u64)> = if gpu_total_vram > 0 {
                            vec![(0, gpu_total_vram)]
                        } else {
                            vec![]
                        };
                        let hra = HraService::new_persisted(
                            &gpus,
                            512,
                            ram_total,
                            &[],
                            PolicyProfile::Balanced,
                            hra_data_dir_bg.join("hra_journal.bin"),
                        );

                        // On boot, surface any leases recovered from the persisted journal (HRA
                        // Phase D1). These represent residency that a prior (crashed) instance held;
                        // the Reconciler/legacy path reclaims the real GPU processes. Logged for
                        // diagnostics — reclaim is gated by the safety policy (Phase D2).
                        let recovered = hra.authority().recovered_open_leases();
                        if !recovered.is_empty() {
                            tracing::warn!(
                                target: "hra",
                                count = recovered.len(),
                                "HRA journal recovery: prior-instance open leases detected (crash recovery)"
                            );
                        }

                        // Register the L1 LLM as an RA-drivable model (additive; no behavior change).
                        let llm_model = std::sync::Arc::new(
                            kria_core::llm::orchestrator::ra_adapter::OrchestratorModel::new(
                                orch.clone(),
                                "l1-llm",
                                gpu_total_vram.min(4096),
                                2048,
                            ),
                        );
                        hra.residency().register(llm_model).await;

                        // Register the process-wide HRA handle so the legacy GPU watchdog can
                        // consult the authority before scale-ups (real cutover hook).
                        kria_core::resource::authority::set_global_hra(hra.clone());

                        // Enforcement flip (HRA Tasks 12–16 / Session 15 cutover).
                        // DEFAULT = ENFORCE: the Hardware & Resource Authority owns GPU admission
                        // (Co-Residency manager) for every consumer. This is the "run on the new
                        // architecture" cutover the user opted into.
                        // ROLLBACK PARACHUTE: set `KRIA_HRA_ENFORCE=0` (or false/off/no) to fall back
                        // to the legacy shadow path instantly — no code change, no data migration.
                        // Legacy `GpuLeaseManager` is intentionally still present as that fallback;
                        // it is only deleted after this enforce path is proven in real use.
                        let enforce = std::env::var("KRIA_HRA_ENFORCE")
                            .ok()
                            .and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
                                "0" | "false" | "off" | "no" => Some(false),
                                "1" | "true" | "on" | "yes" => Some(true),
                                _ => None,
                            })
                            .unwrap_or(true); // default ON (enforce)
                        hra.set_shadow_only(!enforce);
                        // When enforcing, consumers honor the RA (not bypassed). In shadow, the
                        // bypass flag is irrelevant because request() does not gate.
                        hra.set_bypass(ConsumerId::Llm, false);
                        tracing::info!(
                            enforce,
                            "HRA: enforcement {} (default ENFORCE; set KRIA_HRA_ENFORCE=0 to roll back to legacy shadow)",
                            if enforce { "ON" } else { "SHADOW" }
                        );

                        let _ = handle_bg.emit("resource:hra_status", hra.status_json());
                        tracing::info!(
                            gpu_total_vram_mb = gpu_total_vram,
                            ram_total_mb = ram_total,
                            enforce,
                            "HRA: resource authority constructed ({} mode)",
                            if enforce { "ENFORCE" } else { "SHADOW" }
                        );

                        // Co-residency recovery sweep (HRA Phase B): periodically reclaim leases
                        // whose holder vanished (TTL), freeing reservations + cooling the model.
                        // Bounded, best-effort; runs for the process lifetime.
                        let hra_sweep = hra.clone();
                        let task = tokio::spawn(async move {
                            let mut tick =
                                tokio::time::interval(std::time::Duration::from_secs(30));
                            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                            loop {
                                tick.tick().await;
                                let reclaimed = hra_sweep.co_residency().reclaim_expired().await;
                                if reclaimed > 0 {
                                    tracing::info!(
                                        target: "hra",
                                        reclaimed,
                                        "co-residency sweep reclaimed expired leases"
                                    );
                                }
                            }
                        });
                        orchestrator_tasks_bg.lock().await.push(task);

                        // Periodic shadow telemetry → DeviceTable + UI status, sourced from the
                        // single telemetry hub (HRA Phase A1 — no second device context here).
                        let handle_hra = handle_bg.clone();
                        let task = tokio::spawn(async move {
                            let mut rx = match kria_core::resource::global_telemetry_hub() {
                                Some(hub) => hub.subscribe(),
                                None => return, // hub always set at startup; nothing to do otherwise
                            };
                            loop {
                                // Wait for the hub to publish a fresh snapshot (single sampler).
                                if rx.changed().await.is_err() {
                                    break;
                                }
                                let snap = rx.borrow().clone();
                                hra.apply_snapshot(&snap);
                                let _ = handle_hra.emit("resource:hra_status", hra.status_json());
                                let _ = handle_hra.emit(
                                    "resource:hra_diagnostics",
                                    hra.diagnostics_json_async().await,
                                );
                            }
                        });
                        orchestrator_tasks_bg.lock().await.push(task);
                    }

                    // Start idle-release monitor if enabled (HRA Phase A3 — re-enabled).
                    //
                    // The idle monitor unloads the local llama-server + model after
                    // `idle_release_after_secs` of no activity, freeing VRAM for other
                    // consumers (image/voice/vision) and reducing swap pressure. It is
                    // FOREGROUND-SAFE: the loop below skips a release while voice is
                    // active, while any chat turn is in flight (`active_turns > 0`),
                    // while a swap is running, or when no model is resident — so it can
                    // never unload mid-answer. Driven purely by `orch.config`:
                    //   - `idle_release_enabled` (master switch)
                    //   - `idle_release_after_secs` (dwell, min 30s)
                    //   - `idle_release_check_interval_secs` (poll, min 1s)
                    // Set `idle_release_enabled = false` in config to keep the model
                    // resident for the whole session.
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

                        let task = tokio::spawn(async move {
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
                        orchestrator_tasks_bg.lock().await.push(task);
                    }

                    // Start orchestrator event forwarder.
                    {
                        let handle_orch = handle_bg.clone();
                        let mut rx = event_bus_bg.subscribe();
                        let task = tokio::spawn(async move {
                            use kria_core::infra::event_bus::KriaEvent;
                            loop {
                                match rx.recv().await {
                                    Ok(KriaEvent::LlmSwapStarted {
                                        from_ngl,
                                        to_ngl,
                                        emergency,
                                    }) => {
                                        // G10: name the EXACT action instead of a generic
                                        // "Optimizing GPU layers". Emergency downsizes are a
                                        // safety action; a non-emergency change is a placement
                                        // adjustment (rare — only on an explicit break condition).
                                        let banner = if emergency {
                                            "Reducing GPU use to stay stable…"
                                        } else if to_ngl < from_ngl {
                                            "Freeing GPU memory…"
                                        } else {
                                            "Optimizing GPU placement…"
                                        };
                                        let _ = handle_orch.emit(
                                            "orchestrator:swap_started",
                                            serde_json::json!({
                                                "from_ngl": from_ngl,
                                                "to_ngl": to_ngl,
                                                "emergency": emergency,
                                                "banner": banner,
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
                                    Ok(KriaEvent::LlmSwapFailed { reason }) => {
                                        // C3: forward swap failure so the UI can clear the
                                        // "Optimizing GPU layers" overlay (it was previously
                                        // swallowed by `Ok(_) => {}`, stranding the overlay forever).
                                        let _ = handle_orch.emit(
                                            "orchestrator:swap_failed",
                                            serde_json::json!({ "reason": reason }),
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
                        orchestrator_tasks_bg.lock().await.push(task);
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
        orchestrator_tasks.lock().await.push(startup_task);
    }

    // ── settings-config-revamp Task 3: config-change → frontend forwarder ─────
    // Always-on (not gated on the orchestrator). Forwards `KriaEvent::ConfigChanged`
    // to the Tauri `config-changed` event so the UI can reflect live changes. The
    // bus is a bounded, lossy broadcast: on `Lagged` we emit a wildcard signal so
    // the UI reconciles by re-fetching current settings (Req 2.4 / N6).
    {
        let handle_cfg = handle.clone();
        let mut rx = event_bus.subscribe();
        tokio::spawn(async move {
            use kria_core::infra::event_bus::KriaEvent;
            loop {
                match rx.recv().await {
                    Ok(KriaEvent::ConfigChanged { section, version }) => {
                        let _ = handle_cfg.emit(
                            "config-changed",
                            serde_json::json!({ "section": section, "version": version }),
                        );
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!(
                            "config-change forwarder lagged by {n}; signalling re-fetch"
                        );
                        let _ = handle_cfg.emit(
                            "config-changed",
                            serde_json::json!({ "section": "*", "lagged": true }),
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // ── settings-config-revamp Task 8: infallible config-effect executor ──────
    // Applies infallible, live-reloadable effects when config changes, via the
    // ConfigChanged subscription (design C5 / C1.1). Fallible effects
    // (provider/model swap, MCP reconcile) stay on their dedicated apply paths
    // (apply_provider_selection / update_settings). Reference pattern:
    // gpu_policy::apply_settings (lock-free atomics). On lag, re-apply from
    // current config (idempotent reconciliation, N6).
    {
        let cfg_svc = config_service_for_effects;
        let mut rx = event_bus.subscribe();
        tokio::spawn(async move {
            use kria_core::infra::event_bus::KriaEvent;
            async fn apply_infallible(cfg_svc: &kria_core::config::ConfigService) {
                let cfg = cfg_svc.get().await;
                // Orchestrator GPU-policy tunables (redesign G1/G2) — infallible atomics.
                kria_core::llm::orchestrator::gpu_policy::apply_settings(
                    cfg.orchestrator.gpu_autoscale,
                    cfg.orchestrator.cuda_reserve_mb,
                    cfg.orchestrator.vram_volatility_cap_mb,
                );
                // Google Workspace runtime env (account + config dir) — infallible
                // (just sets process env vars, read by the Google MCP on next spawn).
                // Google config lives under the `mcp` servers section. MCP SERVER
                // reconcile itself is fallible and stays on the dedicated apply path
                // (apply_mcp_runtime_from_config) per design C1.1.
                super::command_helpers::apply_google_runtime_env_from_config(&cfg);
            }
            loop {
                match rx.recv().await {
                    Ok(KriaEvent::ConfigChanged { section, .. }) => {
                        if section == "orchestrator"
                            || section == "mcp"
                            || section == "google_workspace"
                            || section == "*"
                        {
                            apply_infallible(&cfg_svc).await;
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        apply_infallible(&cfg_svc).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
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
        let github_ref_bg = github_client_ref.clone();
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

            // Wire GitHub client if the github server started successfully.
            if let Some(gh_client) = mgr.get_client("github") {
                gw::set_github_client(&github_ref_bg, gh_client.clone()).await;
                tracing::info!("[GH] GhClientRef populated — GitHub MCP is now active");
            } else {
                tracing::info!(
                    "[GH] github MCP server not available (set GITHUB_PERSONAL_ACCESS_TOKEN \
                     and ensure Docker is running). GitHub briefing section will be skipped."
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

            drop(mgr);
            // Heartbeat ownership follows the global MCP feature switch and is
            // started/stopped by feature_controls reconciliation.
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
    if let Some(sender) = state.executive_sender.write().await.take() {
        sender.shutdown();
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

    if let Err(error) = super::n8n::reconcile_n8n_feature(state, false, handle).await {
        tracing::warn!(%error, "shutdown: failed to stop n8n cleanly");
    }

    if let Some(heartbeat) = state.mcp_heartbeat.lock().await.take() {
        heartbeat.abort();
        tracing::info!("shutdown: MCP heartbeat stopped");
    }
    {
        let mut manager = state.mcp_manager.lock().await;
        manager.stop_all(&state.tool_registry).await;
    }

    super::mobile_gateway::shutdown_runtime().await;
    super::capability::stop_discovery();
    state.image_orchestrator.shutdown().await;
    if let Some(task) = state.memory_cognition_task.lock().await.take() {
        task.abort();
    }
    state.memory_system.shutdown();

    if let Err(e) = state.sidecar.shutdown().await {
        tracing::warn!("shutdown: failed to stop sidecar cleanly: {e}");
    }

    super::feature_controls::stop_orchestrator_tasks(state).await;
    if let Some(orchestrator) = state.orchestrator.write().await.take() {
        orchestrator.shutdown().await;
    }

    if let Some(pool) = state.container_pool.write().await.take() {
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
