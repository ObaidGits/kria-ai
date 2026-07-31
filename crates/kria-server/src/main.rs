use kria_server::{build_router, ServerState};
use std::sync::Arc;
use tokio_postgres::NoTls;

const FLEET_SCHEMA_SQL: &str =
    include_str!("../../kria-connection-control/sql/0001_device_orchestration.sql");

async fn initialize_fleet_schema() -> anyhow::Result<()> {
    let database_url = match std::env::var("KRIA_FLEET_DATABASE_URL") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            tracing::warn!(
                "KRIA_FLEET_DATABASE_URL not set; skipping fleet SQL migration execution"
            );
            return Ok(());
        }
    };

    let (client, connection) = tokio_postgres::connect(database_url.as_str(), NoTls).await?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::error!(error = %error, "fleet database connection terminated");
        }
    });

    client
        .batch_execute(FLEET_SCHEMA_SQL)
        .await
        .map_err(|error| anyhow::anyhow!("fleet migration failed: {error}"))?;

    tracing::info!("fleet SQL migration applied");
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging (shared profile with desktop runtime)
    let paths = kria_core::platform::paths::KriaPaths::resolve();
    kria_core::infra::logging::setup_logging(&paths.logs_dir);

    let config = kria_core::config::KriaConfig::load(None)?;
    initialize_fleet_schema().await?;
    let fleet = Arc::new(kria_server::inventory::FleetRuntime::initialize(&config).await?);

    // ─── Mobile prompt-control bind address (Phase 4.5.4) ─────────────
    // Prefer the private mesh interface when configured; loudly warn if the
    // mobile path is exposed on a wildcard address (whole-machine blast radius).
    let bind_host = if config.mobile.enabled && !config.mobile.bind_interface.trim().is_empty() {
        config.mobile.bind_interface.trim().to_string()
    } else {
        config.server.host.clone()
    };
    if config.mobile.enabled && (bind_host == "0.0.0.0" || bind_host == "::") {
        tracing::warn!(
            "mobile prompt-control is enabled but bound to {bind_host} — this exposes the \
             agent to every network. Bind [mobile].bind_interface to your private \
             Tailscale/WireGuard address instead."
        );
    }
    let bind_addr = format!("{}:{}", bind_host, config.server.port);

    // ─── MGR-003 / F1.6.1 — loopback default, fail-closed remote startup ──
    // Validate atomically BEFORE any listener is opened. A non-loopback bind
    // (the main server bind_addr above, which also covers the mobile path
    // when it inherits `server.host`) without an explicit `remote_enabled`
    // opt-in and minimal auth profile refuses server startup entirely. This
    // is a separate process from the desktop Tauri app, so local operation
    // is unaffected by this exit.
    if let Err(err) = kria_server::bind_security::validate_bind_security(
        &bind_host,
        config.server.remote_enabled,
        config.server.enable_auth,
        &config.server.jwt_secret,
    ) {
        tracing::error!(error = %err, "kria-server remote startup refused (MGR-003)");
        anyhow::bail!(err);
    }

    // MGR-003 / F1.6.3 — transport-protection attestation (loud warning, not
    // a hard refusal — see `transport` module docs for why this cannot be a
    // hard-fail like the auth-profile check above).
    kria_server::transport::warn_if_transport_unattested(
        config.server.remote_enabled,
        config.server.require_protected_transport,
    );

    // ─── Executive Controller (feature-gated) ─────────────────────────
    let executive_sender = if config.executive.enabled {
        let gpu_lease = kria_core::resource::gpu_lease::GpuLeaseManager::shared(
            std::time::Duration::from_secs(180),
            std::time::Duration::from_secs(15),
        );
        let policy_gate: Arc<dyn kria_core::safety::policy_gate::PolicyGate> =
            Arc::new(kria_core::safety::policy_gate::CapabilityPolicyGate::new());

        let executive_config = kria_core::agent::executive::ExecutiveConfig {
            max_background_tasks: config.executive.max_background_tasks,
            preemption_grace_ms: config.executive.preemption_grace_ms,
            ..Default::default()
        };

        let (mut controller, sender) = kria_core::agent::executive::ExecutiveController::new(
            executive_config,
            gpu_lease,
            policy_gate,
        );

        // Spawn the controller's dispatch loop in the background.
        tokio::spawn(async move {
            controller.run().await;
        });

        tracing::info!("ExecutiveController enabled — dispatch loop started");
        Some(sender)
    } else {
        tracing::info!("ExecutiveController disabled — using legacy AgentLoop");
        None
    };

    let turn_admission = Arc::new(kria_core::agent::TurnAdmission::new());
    // Phase 0.4: build the minimal headless agent loop so /ws streams the real
    // agent (chat + core tools), unblocking the mobile PWA path (Phase 4.5).
    // P7: the headless runtime now also brings up the unified MemorySystem over
    // the shared authority DB, so server chat is memory-driven (same retriever /
    // planner / reasoning / graph / cognition as desktop).
    let (agent_loop, memory_system, session_store): (
        Option<Arc<kria_core::agent::AgentLoop>>,
        Option<Arc<kria_core::memory::api::MemorySystem>>,
        Option<Arc<kria_core::memory::conversation::ConversationStore>>,
    ) = match kria_core::agent::headless_runtime::build_minimal(&config) {
        Ok(rt) => {
            tracing::info!(
                memory = rt.memory_system.is_some(),
                "headless agent runtime ready — /ws chat is live"
            );
            (Some(rt.agent_loop), rt.memory_system, rt.session_store)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "headless agent runtime unavailable; /ws chat will report \
                 'agent runtime not initialized'"
            );
            (None, None, None)
        }
    };

    // P7: background cognition on the server (same scheduler as desktop). Event-
    // driven — wakes on memory changes (coalesced) with a 300s idle fallback.
    if let Some(ms) = &memory_system {
        let ms_bg = ms.clone();
        tokio::spawn(async move {
            let monitor = std::sync::Arc::new(
                kria_core::memory::scheduler::DefaultResourceMonitor::new(512),
            );
            let scheduler = ms_bg.cognitive_scheduler(monitor, None);
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(300));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut changes = ms_bg.subscribe_changes();
            loop {
                let woke = tokio::select! {
                    _ = tick.tick() => false,
                    r = changes.recv() => match r {
                        Ok(_) => { while changes.try_recv().is_ok() {} true }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => true,
                        Err(_) => break,
                    },
                };
                if woke {
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    while changes.try_recv().is_ok() {}
                }
                let ran = scheduler.run_ready().await;
                if ran > 0 {
                    tracing::debug!(jobs = ran, woke, "server cognition ran background jobs");
                }
            }
        });
    }

    // ─── Mobile prompt-control wiring (Phase 4.5) ─────────────────────
    // Device registry (4.5.4): reuses the encrypted vault for the token
    // signing key. Built only when the mobile path is enabled.
    let device_registry: Option<Arc<kria_core::mobile::DeviceRegistry>> = if config.mobile.enabled {
        match kria_core::auth::SecretsVault::open_default() {
            Ok(vault) => {
                let vault = Arc::new(vault);
                match kria_core::mobile::DeviceRegistry::open(
                    paths.data_dir.join("devices.db"),
                    &vault,
                ) {
                    Ok(reg) => {
                        let reg = reg
                            .with_token_ttl(config.mobile.token_ttl_secs)
                            .with_pairing_ttl(config.mobile.pairing_ttl_secs);
                        tracing::info!("mobile device registry ready — pairing endpoints live");
                        Some(Arc::new(reg))
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to open device registry");
                        None
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to open secrets vault for mobile auth");
                None
            }
        }
    } else {
        None
    };

    // ntfy push client (4.5.5): no-op when disabled/unconfigured.
    let notifier: Option<Arc<kria_core::notify::NtfyClient>> = if config.ntfy.enabled {
        tracing::info!("ntfy push enabled");
        Some(Arc::new(kria_core::notify::NtfyClient::new(
            config.ntfy.clone(),
        )))
    } else {
        None
    };

    // Shared conversation store (4.5.6 / F1.2.4): vended by the headless runtime
    // over the process's SINGLE authority `Database` handle (the same one the
    // MemorySystem was composed from), so phone + desktop + server resume the
    // same sessions through one connection pool. The server adapter no longer
    // opens the authority itself — even in the degraded no-embedder path the
    // handle is the one already opened by the core composition entry.
    if session_store.is_some() {
        tracing::info!("server session store bound to the core authority DB handle");
    } else {
        tracing::warn!("session store unavailable; /ws history disabled");
    }

    // Remote desktop view & takeover (Phase 4.6): off unless explicitly enabled.
    let mut remote_desktop_backend: Option<Arc<kria_server::desktop_stream::PortalWebRtcBackend>> =
        None;
    let remote_desktop: Option<Arc<kria_core::remote_desktop::RemoteDesktopManager>> =
        if config.remote_desktop.enabled {
            // settings-config-revamp Task 9: unified onto the shared kria.db
            // (`paths.db_path`) to retire the redundant audit.db (WAL-safe).
            let audit = kria_core::remote_desktop::audit_logger_at(&paths.db_path);
            let backend = Arc::new(kria_server::desktop_stream::PortalWebRtcBackend::new(
                config.remote_desktop.clone(),
            ));
            remote_desktop_backend = Some(backend.clone());
            let mgr = Arc::new(
                kria_core::remote_desktop::RemoteDesktopManager::with_backend(
                    config.remote_desktop.clone(),
                    backend,
                    audit,
                ),
            );
            // Idle / kill-switch enforcement loop.
            let mgr_loop = mgr.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
                loop {
                    tick.tick().await;
                    mgr_loop.enforce_idle();
                }
            });
            tracing::warn!(
                "remote desktop ENABLED — highest-risk capability. Sessions are HITL-gated, \
                 idle-expiring, and audited. Ensure the server is bound to the private mesh."
            );
            Some(mgr)
        } else {
            None
        };

    let quarantine_registry = Arc::new(
        kria_core::tools::quarantine::QuarantineRegistry::open_path(&paths.db_path)?,
    );

    // Distinct authenticated caller construction at the server adapter boundary
    // (F1.2.4). The server is a transport-authenticated REMOTE host, so it
    // constructs an `AuthenticatedRemote` caller — distinct from the desktop's
    // in-process `LocalDesktop` caller — even though both adapters wire to the
    // SAME core memory composition root vended by the headless runtime above.
    // Per-request identity authentication and partition narrowing over this
    // boundary context is the remote security profile (F1.6).
    let caller = kria_core::memory::model::CallerContext::authenticated_remote(
        "local-server",
        "local-server",
        kria_core::memory::model::PolicyPartition::new("user", "chat", 0)
            .expect("static server caller partition is valid"),
    )
    .expect("static server caller identity is valid");
    tracing::info!(
        caller_origin = %caller.origin(),
        caller_partition = %caller.partition_key(),
        "server memory caller context constructed at adapter boundary"
    );

    let state = Arc::new(ServerState {
        config,
        fleet,
        executive_sender,
        quarantine_registry,
        turn_admission,
        agent_loop,
        device_registry,
        notifier,
        session_store,
        memory_system,
        caller,
        remote_desktop,
        remote_desktop_backend,
    });
    let app = build_router(state);

    tracing::info!("KRIA server listening on {bind_addr}");
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
