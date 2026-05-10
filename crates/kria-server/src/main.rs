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
    let bind_addr = format!("{}:{}", config.server.host, config.server.port,);

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

        let (mut controller, sender) =
            kria_core::agent::executive::ExecutiveController::new(
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
    let state = Arc::new(ServerState { config, fleet, executive_sender, turn_admission });
    let app = build_router(state);

    tracing::info!("KRIA server listening on {bind_addr}");
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
