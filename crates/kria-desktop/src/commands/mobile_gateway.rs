//! Desktop control surface for Phase 4.5 (mobile prompt-control) and Phase 4.6
//! (remote desktop). The desktop app hosts the phone-facing gateway itself —
//! reusing `kria_server::gateway` with the desktop's *real* `AgentLoop` — and
//! exposes Tauri commands so all pairing, device management, gateway lifecycle,
//! and the remote-desktop kill switch are driven from the desktop UI.

use super::*;
use std::path::PathBuf;
use std::sync::OnceLock;
use tokio::sync::{oneshot, Mutex as AsyncMutex};

use kria_core::mobile::DeviceRegistry;
use kria_core::notify::NtfyClient;
use kria_core::remote_desktop::RemoteDesktopManager;
use kria_server::gateway::{phone_gateway_router, PhoneGatewayState};

/// Long-lived managers (built once; survive gateway start/stop cycles).
struct GatewayManagers {
    device_registry: Arc<DeviceRegistry>,
    remote_desktop: Arc<RemoteDesktopManager>,
    remote_desktop_backend: Arc<kria_server::desktop_stream::PortalWebRtcBackend>,
    notifier: Option<Arc<NtfyClient>>,
    session_store: Option<Arc<kria_core::memory::conversation::ConversationStore>>,
}

/// Running HTTP listener handle (None when the gateway is stopped).
struct ServerHandle {
    shutdown: oneshot::Sender<()>,
    bound_addr: String,
}

struct GatewayManagersRuntime {
    current: Option<Arc<GatewayManagers>>,
    remote_desktop_monitor: Option<tokio::task::JoinHandle<()>>,
}

static MANAGERS: OnceLock<AsyncMutex<GatewayManagersRuntime>> = OnceLock::new();
static SERVER: OnceLock<AsyncMutex<Option<ServerHandle>>> = OnceLock::new();

fn server_slot() -> &'static AsyncMutex<Option<ServerHandle>> {
    SERVER.get_or_init(|| AsyncMutex::new(None))
}

fn manager_slot() -> &'static AsyncMutex<GatewayManagersRuntime> {
    MANAGERS.get_or_init(|| {
        AsyncMutex::new(GatewayManagersRuntime {
            current: None,
            remote_desktop_monitor: None,
        })
    })
}

/// Build one live manager set from current config. Remote-desktop monitoring is
/// owned by the slot and exists only while that feature is enabled.
async fn managers(config: &KriaConfig) -> Arc<GatewayManagers> {
    let mut runtime = manager_slot().lock().await;
    if let Some(current) = runtime.current.as_ref() {
        return current.clone();
    }

    let paths = kria_core::platform::paths::KriaPaths::resolve();
    let device_registry = kria_core::auth::SecretsVault::open_default()
        .map_err(|e| e.to_string())
        .and_then(|vault| {
            DeviceRegistry::open(paths.data_dir.join("devices.db"), &Arc::new(vault))
                .map_err(|e| e.to_string())
        })
        .map(|reg| {
            Arc::new(
                reg.with_token_ttl(config.mobile.token_ttl_secs)
                    .with_pairing_ttl(config.mobile.pairing_ttl_secs),
            )
        })
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "mobile: device registry init failed");
            Arc::new(
                DeviceRegistry::open(
                    std::env::temp_dir().join("kria_devices_fallback.db"),
                    &Arc::new(
                        kria_core::auth::SecretsVault::open_default()
                            .expect("vault open (fallback)"),
                    ),
                )
                .expect("device registry (fallback)"),
            )
        });

    let audit = kria_core::remote_desktop::audit_logger_at(&paths.data_dir.join("audit.db"));
    let remote_desktop_backend = Arc::new(kria_server::desktop_stream::PortalWebRtcBackend::new(
        config.remote_desktop.clone(),
    ));
    let remote_desktop = Arc::new(RemoteDesktopManager::with_backend(
        config.remote_desktop.clone(),
        remote_desktop_backend.clone(),
        audit,
    ));
    remote_desktop.reconcile_disabled();

    if config.remote_desktop.enabled {
        let manager = remote_desktop.clone();
        runtime.remote_desktop_monitor = Some(tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                manager.enforce_idle();
            }
        }));
    }

    let notifier = config
        .ntfy
        .enabled
        .then(|| Arc::new(NtfyClient::new(config.ntfy.clone())));
    let session_store =
        kria_core::memory::db::Database::open(&paths.data_dir.join("kria_memory.db"))
            .map(|db| {
                Arc::new(kria_core::memory::conversation::ConversationStore::new(
                    Arc::new(db),
                ))
            })
            .ok();

    let managers = Arc::new(GatewayManagers {
        device_registry,
        remote_desktop,
        remote_desktop_backend,
        notifier,
        session_store,
    });
    runtime.current = Some(managers.clone());
    managers
}

async fn reset_managers() {
    let mut runtime = manager_slot().lock().await;
    if let Some(monitor) = runtime.remote_desktop_monitor.take() {
        monitor.abort();
    }
    if let Some(managers) = runtime.current.take() {
        managers.remote_desktop.stop();
    }
}

pub(super) async fn shutdown_runtime() {
    stop_gateway().await;
    reset_managers().await;
}

pub(super) async fn reconcile_auxiliary_managers(app: &AppState) -> Result<(), String> {
    let was_running = gateway_running().await;
    if was_running {
        stop_gateway().await;
    }
    reset_managers().await;
    if was_running && app.config.read().await.mobile.enabled {
        start_gateway(app).await?;
    }
    Ok(())
}

fn cell<'a>(state: &'a State<'_, AppStateCell>) -> Result<&'a AppState, String> {
    state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())
}

/// Compute the bind host for the gateway from config (empty → server.host).
fn bind_host(config: &KriaConfig) -> String {
    if config.mobile.bind_interface.trim().is_empty() {
        config.server.host.clone()
    } else {
        config.mobile.bind_interface.trim().to_string()
    }
}

/// Best-effort primary LAN IP (no packets sent — just route source selection).
fn lan_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    sock.local_addr().ok().map(|a| a.ip().to_string())
}

/// A phone-dialable host. `0.0.0.0`/`::`/empty (wildcard binds) are not
/// reachable, so substitute the machine's real LAN IP for display/pairing.
fn advertised_host(bind: &str) -> String {
    if bind.is_empty() || bind == "0.0.0.0" || bind == "::" {
        lan_ip().unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        bind.to_string()
    }
}

/// Resolve an absolute `ui/dist` directory (the built PWA) by searching up from
/// the current working dir and the executable dir. The desktop app's cwd is not
/// the workspace root, so the gateway's default relative `ui/dist` would 404 —
/// this finds it and is exported via `KRIA_UI_DIST` for the gateway router.
fn resolve_ui_dist() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("KRIA_UI_DIST") {
        let pb = PathBuf::from(p);
        if pb.join("index.html").exists() {
            return Some(pb);
        }
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.to_path_buf());
        }
    }
    for root in roots {
        let mut dir = Some(root.as_path());
        while let Some(d) = dir {
            let cand = d.join("ui").join("dist");
            if cand.join("index.html").exists() {
                return Some(cand);
            }
            dir = d.parent();
        }
    }
    None
}

// ── Tauri commands ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn mobile_gateway_status(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let app = cell(&state)?;
    let config = app.config.read().await.clone();
    let mgrs = managers(&config).await;
    let slot = server_slot().lock().await;
    let running = slot.is_some();
    let bound = slot.as_ref().map(|h| h.bound_addr.clone());
    Ok(serde_json::json!({
        "mobile_enabled": config.mobile.enabled,
        "remote_desktop_enabled": config.remote_desktop.enabled,
        "running": running,
        "bound_addr": bound,
        "device_count": mgrs.device_registry.list_devices().map(|d| d.len()).unwrap_or(0),
        "remote_desktop": mgrs.remote_desktop.status(),
    }))
}

pub(super) async fn gateway_running() -> bool {
    server_slot().lock().await.is_some()
}

pub(super) async fn start_gateway(app: &AppState) -> Result<serde_json::Value, String> {
    let config = app.config.read().await.clone();
    if !config.mobile.enabled {
        return Err("Mobile gateway is disabled in Settings".into());
    }
    let mgrs = managers(&config).await;

    let mut slot = server_slot().lock().await;
    if let Some(h) = slot.as_ref() {
        return Ok(serde_json::json!({
            "status": "already_running",
            "bound_addr": h.bound_addr,
        }));
    }

    let host = bind_host(&config);
    let port = config.mobile.port;
    let addr = format!("{host}:{port}");

    // Resolve the built PWA to an absolute path so `/m` + assets serve from the
    // desktop app regardless of its working directory.
    match resolve_ui_dist() {
        Some(dist) => {
            tracing::info!(dist = %dist.display(), "mobile gateway: serving PWA from");
            std::env::set_var("KRIA_UI_DIST", dist);
        }
        None => {
            tracing::warn!(
                "mobile gateway: ui/dist not found — /m will 404. Build it with \
                 `npm run build` in the ui/ directory."
            );
        }
    }

    let gw_state = Arc::new(PhoneGatewayState {
        config: config.clone(),
        agent_loop: Some(app.agent_loop.clone()),
        turn_admission: Arc::new(kria_core::agent::TurnAdmission::new()),
        device_registry: Some(mgrs.device_registry.clone()),
        notifier: mgrs.notifier.clone(),
        session_store: mgrs.session_store.clone(),
        remote_desktop: Some(mgrs.remote_desktop.clone()),
        remote_desktop_backend: Some(mgrs.remote_desktop_backend.clone()),
    });

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("failed to bind {addr}: {e}"))?;

    // Advertise a phone-dialable address (real LAN IP when bound to 0.0.0.0).
    let bound_addr = format!("{}:{}", advertised_host(&host), port);

    let (tx, rx) = oneshot::channel::<()>();
    let router = phone_gateway_router(gw_state);
    tokio::spawn(async move {
        let server = axum::serve(listener, router).with_graceful_shutdown(async move {
            let _ = rx.await;
        });
        if let Err(e) = server.await {
            tracing::error!(error = %e, "mobile gateway server stopped with error");
        }
    });

    if host == "0.0.0.0" || host == "::" {
        tracing::warn!(
            "mobile gateway bound to {host} (LAN-reachable). For production, set \
             [mobile].bind_interface to a private Tailscale address."
        );
    }

    *slot = Some(ServerHandle {
        shutdown: tx,
        bound_addr: bound_addr.clone(),
    });

    Ok(serde_json::json!({
        "status": "started",
        "bound_addr": bound_addr,
        "pair_url": format!("http://{bound_addr}/pair"),
        "mobile_url": format!("http://{bound_addr}/m"),
    }))
}

#[tauri::command]
pub async fn mobile_gateway_start(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    start_gateway(cell(&state)?).await
}

pub(super) async fn stop_gateway() {
    let mut slot = server_slot().lock().await;
    if let Some(handle) = slot.take() {
        let _ = handle.shutdown.send(());
        tracing::info!("mobile gateway stopped");
    }
}

#[tauri::command]
pub async fn mobile_gateway_stop(_state: State<'_, AppStateCell>) -> Result<(), String> {
    stop_gateway().await;
    Ok(())
}

#[tauri::command]
pub async fn mobile_begin_pairing(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let app = cell(&state)?;
    let config = app.config.read().await.clone();
    let mgrs = managers(&config).await;
    let host = {
        let slot = server_slot().lock().await;
        slot.as_ref()
            .map(|h| h.bound_addr.clone())
            .unwrap_or_else(|| {
                format!(
                    "{}:{}",
                    advertised_host(&bind_host(&config)),
                    config.mobile.port
                )
            })
    };
    let challenge = mgrs.device_registry.begin_pairing(&host);
    Ok(serde_json::json!({
        "code": challenge.code,
        "qr_payload": challenge.qr_payload,
        "expires_at": challenge.expires_at,
        "server_url": format!("http://{host}"),
        "mobile_url": format!("http://{host}/m"),
    }))
}

#[tauri::command]
pub async fn mobile_list_devices(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let app = cell(&state)?;
    let config = app.config.read().await.clone();
    let mgrs = managers(&config).await;
    let devices = mgrs
        .device_registry
        .list_devices()
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "devices": devices }))
}

#[tauri::command]
pub async fn mobile_revoke_device(
    device_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let app = cell(&state)?;
    let config = app.config.read().await.clone();
    let mgrs = managers(&config).await;
    let revoked = mgrs
        .device_registry
        .revoke(&device_id)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "device_id": device_id, "revoked": revoked }))
}

#[tauri::command]
pub async fn remote_desktop_status(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let app = cell(&state)?;
    let config = app.config.read().await.clone();
    let mgrs = managers(&config).await;
    Ok(serde_json::to_value(mgrs.remote_desktop.status()).unwrap_or_default())
}

/// Kill switch: tear down any active remote-desktop session immediately.
#[tauri::command]
pub async fn remote_desktop_kill(state: State<'_, AppStateCell>) -> Result<(), String> {
    let app = cell(&state)?;
    let config = app.config.read().await.clone();
    let mgrs = managers(&config).await;
    mgrs.remote_desktop.stop();
    Ok(())
}

/// Persist mobile / remote-desktop / ntfy config from the Settings UI.
#[tauri::command]
pub async fn set_mobile_config(
    mobile_enabled: bool,
    require_device_auth: bool,
    bind_interface: String,
    remote_desktop_enabled: bool,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let app = cell(&state)?;
    app.config_service
        .patch_batch(
            vec![
                kria_core::config::Change::new(
                    "mobile",
                    "enabled",
                    serde_json::json!(mobile_enabled),
                ),
                kria_core::config::Change::new(
                    "mobile",
                    "require_device_auth",
                    serde_json::json!(require_device_auth),
                ),
                kria_core::config::Change::new(
                    "mobile",
                    "bind_interface",
                    serde_json::json!(bind_interface),
                ),
                kria_core::config::Change::new(
                    "remote_desktop",
                    "enabled",
                    serde_json::json!(remote_desktop_enabled),
                ),
            ],
            kria_core::config::ChangeSource::Ui,
            None,
        )
        .await
        .map_err(|error| error.to_string())?;

    stop_gateway().await;
    reset_managers().await;
    if mobile_enabled {
        start_gateway(app).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_mobile_config(
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let app = cell(&state)?;
    let config = app.config.read().await;
    Ok(serde_json::json!({
        "mobile_enabled": config.mobile.enabled,
        "require_device_auth": config.mobile.require_device_auth,
        "bind_interface": config.mobile.bind_interface,
        "remote_desktop_enabled": config.remote_desktop.enabled,
    }))
}
