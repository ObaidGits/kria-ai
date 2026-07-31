pub mod auth;
pub mod bind_security;
pub mod correlation;
pub mod deny;
pub mod desktop_stream;
pub mod gateway;
pub mod intelligence_routes;
pub mod inventory;
pub mod memory_routes;
pub mod mobile_routes;
pub mod origin;
pub mod provider_routes;
pub mod rate_limit;
pub mod remote_desktop_routes;
pub mod routes;
pub mod transport;
pub mod ws;

use axum::extract::DefaultBodyLimit;
use axum::Router;
use std::sync::Arc;
use std::time::Duration;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use gateway::{phone_gateway_router, PhoneGatewayState};

pub struct ServerState {
    pub config: kria_core::config::KriaConfig,
    pub fleet: Arc<inventory::FleetRuntime>,
    /// Executive Controller sender — present only when `executive.enabled = true`.
    pub executive_sender: Option<kria_core::agent::executive::ExecutiveSender>,
    /// Durable generated/discovered tool trust gate.
    pub quarantine_registry: Arc<kria_core::tools::quarantine::QuarantineRegistry>,
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
    pub session_store: Option<Arc<kria_core::memory::conversation::ConversationStore>>,
    /// The unified cognitive [`MemorySystem`] over the SAME authority DB the
    /// desktop uses (P7). `Some` whenever the headless runtime brought memory
    /// online. Server chat/retrieval/planner/reasoning and the `/memory/*`
    /// routes all flow through this — one memory architecture, no server
    /// bypass.
    pub memory_system: Option<Arc<kria_core::memory::api::MemorySystem>>,
    /// The authenticated caller identity this server (Axum) adapter constructs
    /// at its boundary (F1.2.4). The server is a transport-authenticated remote
    /// host (`CallerOrigin::AuthenticatedRemote`) — distinct from the desktop's
    /// in-process `LocalDesktop` caller — even though both wire to the SAME core
    /// memory composition root (`memory_system`). This is the FALLBACK identity
    /// used when no per-request identity is available (default loopback mode,
    /// where `auth_middleware` is not layered on — see `build_router`). In
    /// remote mode (`[server].remote_enabled = true`), `auth_middleware`
    /// (F1.6.2) verifies a real signed bearer token per request and inserts a
    /// per-request `CallerContext` built from the token's actual
    /// actor_id/device_id as a request extension; `memory_routes::
    /// effective_caller` prefers that over this field whenever present.
    pub caller: kria_core::memory::model::CallerContext,
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

    // MGR-003 / F1.6.2 — real bearer-token auth only applies in remote mode.
    //
    // Loopback is the default, safe case: MGR-003 AC1/AC2 only impose the
    // authenticated-identity/replay/etc. requirements "WHEN a non-loopback
    // bind is configured" — there is no acceptance criterion requiring token
    // auth for a caller that never opted into remote exposure. Requiring
    // tokens on loopback would be a usability regression the spec does not
    // ask for (the desktop app's own local API calls would need a token too),
    // so `auth_middleware` is layered on ONLY when `[server].remote_enabled =
    // true`. `bind_security::validate_bind_security` (F1.6.1) already
    // guarantees that whenever `remote_enabled` is true, `enable_auth` is
    // true and `jwt_secret` is non-empty before the process even reaches
    // `axum::serve` — so gating this layer on `remote_enabled` alone is
    // sufficient and cannot silently run with an empty secret.
    let remote_enabled = state.config.server.remote_enabled;
    let server_cfg = state.config.server.clone();

    let mut api = Router::new()
        .merge(routes::api_routes())
        .merge(intelligence_routes::intelligence_routes())
        .merge(provider_routes::provider_routes())
        .merge(memory_routes::memory_routes())
        .with_state(state.clone());

    // MGR-003 / F1.6.3 — rate-limit runs innermost of `api`'s own security
    // layers so it only counts requests that already passed origin/auth (a
    // denied request should not consume a legitimate caller's budget), and
    // so it can key on the REAL authenticated `actor_id` `auth_middleware`
    // (layered directly below) already inserted as a `CallerContext`
    // extension — only in remote mode (loopback has no untrusted-caller
    // identity to key on).
    if remote_enabled {
        api = api.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            rate_limit::rate_limit_middleware,
        ));
    }

    // MGR-003 / F1.6.2 — real bearer-token auth only applies in remote mode.
    //
    // Loopback is the default, safe case: MGR-003 AC1/AC2 only impose the
    // authenticated-identity/replay/etc. requirements "WHEN a non-loopback
    // bind is configured" — there is no acceptance criterion requiring token
    // auth for a caller that never opted into remote exposure. Requiring
    // tokens on loopback would be a usability regression the spec does not
    // ask for (the desktop app's own local API calls would need a token too),
    // so `auth_middleware` is layered on ONLY when `[server].remote_enabled =
    // true`. `bind_security::validate_bind_security` (F1.6.1) already
    // guarantees that whenever `remote_enabled` is true, `enable_auth` is
    // true and `jwt_secret` is non-empty before the process even reaches
    // `axum::serve` — so gating this layer on `remote_enabled` alone is
    // sufficient and cannot silently run with an empty secret.
    //
    // Gateway routes (mobile pairing/device-management, agent WS, remote
    // desktop, static PWA) are deliberately NOT wrapped by this bearer-token
    // layer: they carry their OWN device-token boundary
    // (`mobile_routes::authorize_device_management`,
    // `remote_desktop_routes::authorize`, `ws::ws_handler`'s
    // `mobile.require_device_auth` gate) keyed on the phone-pairing
    // `DeviceRegistry`, not the API's bearer-token scheme — a different but
    // equally real credential, appropriate to a phone client that pairs once
    // rather than minting a `krav1` server token. `pair_page`/`pair_begin`/
    // `pair_complete` remain intentionally exempt from BOTH auth schemes (the
    // bootstrap pairing step a not-yet-paired phone must reach — see their
    // own doc comments) and the static PWA fallback is public shell assets
    // (no protected data).
    if remote_enabled {
        api = api.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ));
    }

    // MGR-003 / F1.6.3 — server-side origin enforcement on `api` (beyond
    // what the CORS layer below can do for non-browser clients — see
    // `origin` module docs). Remote mode only, same reasoning as auth/
    // rate-limit above.
    if remote_enabled {
        api = api.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            origin::origin_middleware,
        ));
    }

    // MGR-003 / F1.6.3 / F1.6.6 — the gateway (mobile pairing/device-
    // management, `/rd-signal`, `/ws`, static PWA) gets its OWN origin +
    // rate-limit instance, applied directly to `gateway` BEFORE the merge
    // below — NOT by adding these layers to the merged `api ∪ gateway`
    // router after the fact.
    //
    // F1.6.6 fix: before this fix, `origin`/`rate_limit` were applied only
    // to `api`, so a gateway route was completely unprotected by either
    // check once `remote_enabled = true` — a real gap. The naive fix of
    // moving those two `.layer()` calls to run on the MERGED router (after
    // `api.merge(gateway)`) would have been WRONG: `Router::layer` wraps
    // from the outside in, so a layer added to the merged router runs
    // OUTSIDE every layer `api` already carries — including `auth` — which
    // would put `rate_limit` BEFORE `auth_middleware` for `api` requests
    // too, breaking its documented real-actor-id keying (it would always
    // see no `CallerContext` extension yet and silently fall back to the
    // single static `ServerState::caller` for every request, collapsing
    // every distinct remote caller onto the SAME rate-limit/origin-caller
    // bucket). Giving `gateway` its own separate instance of each
    // middleware, applied directly to `gateway` before the merge, closes
    // the coverage gap without disturbing `api`'s existing, already-correct
    // origin→auth→rate_limit ordering at all. `rate_limit_middleware` has no
    // `CallerContext` extension to key on inside `gateway` (gateway auth is
    // device-token based, not `CallerContext`-based), so it uses its
    // documented fallback — the static `ServerState::caller` actor id — for
    // every gateway request, giving all gateway traffic one coarse shared
    // per-minute budget. Strictly better than the previous zero enforcement.
    let mut gateway = gateway;
    if remote_enabled {
        gateway = gateway.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            rate_limit::rate_limit_middleware,
        ));
        gateway = gateway.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            origin::origin_middleware,
        ));
    }

    // API/fleet routes take precedence; the gateway adds pairing + agent WS +
    // remote desktop + the static PWA fallback (incl. `/m`).
    //
    // MGR-003 / F1.6.3 — CORS is restricted to the exact configured
    // `[server].allowed_origins` allowlist once `remote_enabled = true`
    // (fail-closed: an empty list denies every browser origin); loopback/
    // default mode keeps the permissive CORS this router always had, per the
    // existing F1.6.1/F1.6.2 precedent of not adding friction to the safe
    // default path (`origin::build_cors_layer` docs).
    //
    // Layer order (outermost first, since `Router::layer` wraps from the
    // outside in): correlation ID → deny-envelope normalization → body limit
    // → timeout → concurrency limit → CORS → trace → then EITHER [origin →
    // auth → rate-limit, for an `api`-subtree route] OR [origin →
    // rate-limit, for a `gateway`-subtree route, each only in remote mode]
    // → routes. Body/timeout/concurrency apply UNIVERSALLY (loopback and
    // remote) as basic stability protection — they are not identity/trust-
    // dependent hardening, unlike origin/auth/rate-limit, which only make
    // sense once there is an untrusted remote caller to defend against
    // (MGR-009 boundedness vs. MGR-003 threat boundary).
    //
    // MGR-003 AC3 / F1.6.4 — `deny::normalize_builtin_denies` sits directly
    // inside `correlation::correlation_middleware` (so it can read the
    // correlation id already attached to the request) and directly outside
    // `RequestBodyLimitLayer`/`TimeoutLayer` (so it can rewrite their raw
    // tower-http default bodies into the shared non-revealing envelope
    // before any response leaves the process).
    api.merge(gateway)
        .layer(origin::build_cors_layer(
            remote_enabled,
            &server_cfg.allowed_origins,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(ConcurrencyLimitLayer::new(
            server_cfg.max_concurrent_requests,
        ))
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(server_cfg.request_timeout_secs),
        ))
        .layer(DefaultBodyLimit::max(server_cfg.max_body_bytes))
        .layer(RequestBodyLimitLayer::new(server_cfg.max_body_bytes))
        .layer(axum::middleware::from_fn(deny::normalize_builtin_denies))
        .layer(axum::middleware::from_fn(
            correlation::correlation_middleware,
        ))
}
