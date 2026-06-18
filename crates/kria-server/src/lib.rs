pub mod auth;
pub mod desktop_stream;
pub mod gateway;
pub mod intelligence_routes;
pub mod inventory;
pub mod mobile_routes;
pub mod provider_routes;
pub mod remote_desktop_routes;
pub mod routes;
pub mod ws;

use axum::Router;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use gateway::{phone_gateway_router, PhoneGatewayState};

pub struct ServerState {
    pub config: kria_core::config::KriaConfig,
    pub fleet: Arc<inventory::FleetRuntime>,
    /// Executive Controller sender — present only when `executive.enabled = true`.
    pub executive_sender: Option<kria_core::agent::executive::ExecutiveSender>,
    /// Shared turn-admission gate so HTTP/WS cancel endpoints can cancel
    /// in-flight turns without needing a full AgentLoop handle.
    pub turn_admission: Arc<kria_core::agent::TurnAdmission>,
    /// The real agent loop streamed over the WebSocket (Phase 0.4 / 4.5).
    ///
    /// `None` until a headless runtime builder constructs it (see
    /// `planning_docs/foundation_phase0_plan.md`). When `None`, the WS handler
    /// returns a clear "agent runtime not initialized" frame instead of a stub.
    pub agent_loop: Option<Arc<kria_core::agent::AgentLoop>>,
    /// Per-device pairing + token registry (Phase 4.5.4). `None` when the
    /// mobile path is disabled.
    pub device_registry: Option<Arc<kria_core::mobile::DeviceRegistry>>,
    /// ntfy push client (Phase 4.5.5). `None`/disabled is a no-op.
    pub notifier: Option<Arc<kria_core::notify::NtfyClient>>,
    /// Shared conversation store so phone + desktop resume the same sessions
    /// (Phase 4.5.6).
    pub session_store: Option<Arc<kria_core::memory::MemoryStore>>,
    /// Remote desktop view & takeover manager (Phase 4.6). `None` when disabled.
    pub remote_desktop: Option<Arc<kria_core::remote_desktop::RemoteDesktopManager>>,
    /// Portal + WebRTC capture backend (shares the Arc the manager holds).
    pub remote_desktop_backend: Option<Arc<desktop_stream::PortalWebRtcBackend>>,
}

/// Resolve the directory holding the built PWA (`ui/dist`). Re-exported for
/// callers that need the path (delegates to [`gateway::ui_dist_dir`]).
pub fn ui_dist_dir() -> std::path::PathBuf {
    gateway::ui_dist_dir()
}

/// Build a [`PhoneGatewayState`] from the full [`ServerState`] (shares Arcs).
fn gateway_state_from(state: &Arc<ServerState>) -> Arc<PhoneGatewayState> {
    Arc::new(PhoneGatewayState {
        config: state.config.clone(),
        agent_loop: state.agent_loop.clone(),
        turn_admission: state.turn_admission.clone(),
        device_registry: state.device_registry.clone(),
        notifier: state.notifier.clone(),
        session_store: state.session_store.clone(),
        remote_desktop: state.remote_desktop.clone(),
        remote_desktop_backend: state.remote_desktop_backend.clone(),
    })
}

/// Build the full application router (used by both main and integration tests).
pub fn build_router(state: Arc<ServerState>) -> Router {
    let gateway = phone_gateway_router(gateway_state_from(&state));

    let api = Router::new()
        .merge(routes::api_routes())
        .merge(intelligence_routes::intelligence_routes())
        .merge(provider_routes::provider_routes())
        .with_state(state);

    // API/fleet routes take precedence; the gateway adds pairing + agent WS +
    // remote desktop + the static PWA fallback (incl. `/m`).
    api.merge(gateway)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}
