pub mod auth;
pub mod intelligence_routes;
pub mod inventory;
pub mod routes;
pub mod ws;

use axum::Router;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub struct ServerState {
    pub config: kria_core::config::KriaConfig,
    pub fleet: Arc<inventory::FleetRuntime>,
    /// Executive Controller sender — present only when `executive.enabled = true`.
    pub executive_sender: Option<kria_core::agent::executive::ExecutiveSender>,
}

/// Build the full application router (used by both main and integration tests).
pub fn build_router(state: Arc<ServerState>) -> Router {
    Router::new()
        .merge(routes::api_routes())
        .merge(intelligence_routes::intelligence_routes())
        .merge(ws::ws_routes())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
