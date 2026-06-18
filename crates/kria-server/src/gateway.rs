//! Phone-facing gateway: the reusable router that powers the mobile PWA path
//! (Phase 4.5) and remote desktop (Phase 4.6).
//!
//! This is shared by both the standalone `kria-server` and the desktop app
//! (`kria-desktop`), so a phone always talks to the *same* agent + safety stack
//! regardless of which host is running. The desktop builds a [`PhoneGatewayState`]
//! from its real `AppState` (live AgentLoop, memory, managers) and mounts
//! [`phone_gateway_router`] on a mesh/LAN listener it controls via Tauri.

use axum::Router;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::{ServeDir, ServeFile};

/// State shared by every phone-facing route. Field names match the legacy
/// `ServerState` so the route handlers are state-agnostic.
pub struct PhoneGatewayState {
    pub config: kria_core::config::KriaConfig,
    pub agent_loop: Option<Arc<kria_core::agent::AgentLoop>>,
    pub turn_admission: Arc<kria_core::agent::TurnAdmission>,
    pub device_registry: Option<Arc<kria_core::mobile::DeviceRegistry>>,
    pub notifier: Option<Arc<kria_core::notify::NtfyClient>>,
    pub session_store: Option<Arc<kria_core::memory::MemoryStore>>,
    pub remote_desktop: Option<Arc<kria_core::remote_desktop::RemoteDesktopManager>>,
    /// Portal + WebRTC capture backend driving the desktop stream (Phase 4.6 v3).
    /// Shares the same object the manager holds; `None` when remote desktop is off.
    pub remote_desktop_backend: Option<Arc<crate::desktop_stream::PortalWebRtcBackend>>,
}

/// Resolve the directory holding the built PWA (`ui/dist`).
///
/// Honours `KRIA_UI_DIST`; otherwise defaults to `ui/dist` relative to the
/// current working directory.
pub fn ui_dist_dir() -> PathBuf {
    match std::env::var("KRIA_UI_DIST") {
        Ok(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => PathBuf::from("ui/dist"),
    }
}

/// Build the phone-facing router (pairing + agent WS + remote desktop + PWA),
/// serving the static shell with an explicit 200 for the `/m` entry route.
pub fn phone_gateway_router(state: Arc<PhoneGatewayState>) -> Router {
    let dynamic = Router::new()
        .merge(crate::mobile_routes::mobile_routes())
        .merge(crate::remote_desktop_routes::remote_desktop_routes())
        .merge(crate::ws::ws_routes())
        .with_state(state);

    let dist = ui_dist_dir();
    let index = dist.join("index.html");
    let static_service = ServeDir::new(&dist).not_found_service(ServeFile::new(index.clone()));

    dynamic
        .route_service("/m", ServeFile::new(index))
        .fallback_service(static_service)
}
