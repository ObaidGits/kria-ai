//! Exact browser `Origin` allowlist policy (MGR-003 AC2 "restrictive
//! origins" — F1.6.3).
//!
//! ## Scope and reasoning
//!
//! CORS is a **browser-enforced** boundary: the `Access-Control-Allow-Origin`
//! response header only stops a browser's own fetch/XHR from exposing a
//! cross-origin response back to the calling page's script — it does
//! nothing to stop a non-browser client (curl, a mobile app, another
//! server) from sending the request and reading the response directly, and
//! it does nothing on its own to stop a same-origin `<form>` POST or a
//! request with a forged/absent `Origin` header. A CORS layer alone is
//! therefore NOT sufficient to satisfy "restrictive origins" as a *server-
//! enforced* control — see the task's own note on this.
//!
//! This module has two matching parts, both gated on `remote_enabled`:
//!
//! 1. [`build_cors_layer`] — a real [`CorsLayer`] restricted to the exact
//!    configured `[server].allowed_origins` list (byte-exact match, no
//!    wildcard/subdomain matching) so a legitimate BROWSER-based remote
//!    client only gets `Access-Control-Allow-Origin` for an allowed origin.
//! 2. [`origin_middleware`] — a server-side check that runs on every request
//!    (not just browser-issued cross-origin ones) and DENIES any request
//!    that carries an `Origin` header not present in the allowlist. A
//!    request with NO `Origin` header at all (the normal case for a
//!    non-browser client, e.g. a mobile app or `curl` using the bearer
//!    token directly) is allowed through this check — origin is a browser
//!    concept and a non-browser client cannot forge browser trust by
//!    omitting it; identity/authorization is what actually gates non-browser
//!    callers (`auth_middleware`, F1.6.2). A request that DOES carry an
//!    `Origin` header not on the allowlist is denied here even for a
//!    non-preflight, non-browser-shaped request — closing the gap that a
//!    CORS-only implementation would leave open.
//!
//! An **empty `allowed_origins` in remote mode is fail-closed**: every
//! `Origin`-bearing request is denied (MGR-003 AC2 default reading — an
//! operator who enables remote mode without configuring an allowlist gets
//! "no browser client is trusted" rather than "every browser client is
//! trusted", which `CorsLayer::permissive()` would silently imply).
//!
//! Loopback/default mode is unaffected: this module's middleware is only
//! layered on when `remote_enabled = true` (see `lib.rs::build_router`),
//! matching the precedent already set by `auth_middleware` (F1.6.2) and
//! `bind_security` (F1.6.1) — MGR-003 AC2's origin requirement is scoped to
//! "WHEN a non-loopback bind is configured".

use axum::{
    extract::{Request, State},
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::correlation::correlation_id_of;
use crate::deny::deny_envelope;
use crate::ServerState;

/// Build the CORS layer for the current configuration.
///
/// - Loopback/default mode (`remote_enabled = false`): permissive, matching
///   the existing precedent (F1.6.1/F1.6.2) of not adding friction to the
///   safe default path — MGR-003's origin requirement is scoped to the
///   non-loopback case.
/// - Remote mode with a non-empty `allowed_origins`: restricted to exactly
///   those origins.
/// - Remote mode with an EMPTY `allowed_origins` (fail-closed default):
///   [`AllowOrigin::list`] with an empty list — every actual `Origin` value
///   fails to match, so the browser receives no
///   `Access-Control-Allow-Origin` for any real origin. This is deliberately
///   NOT `CorsLayer::new()`'s bare default (which also sends no header, but
///   for a different, easily-misread reason) — using `list([])` documents
///   the fail-closed intent at the call site.
pub fn build_cors_layer(remote_enabled: bool, allowed_origins: &[String]) -> CorsLayer {
    if !remote_enabled {
        return CorsLayer::permissive();
    }

    let origins: Vec<HeaderValue> = allowed_origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(tower_http::cors::AllowMethods::any())
        .allow_headers(tower_http::cors::AllowHeaders::any())
}

/// The non-revealing deny envelope for an origin the caller's `Origin`
/// header does not match the configured allowlist (MGR-003 AC3 — no
/// protected label/count/topology detail). Delegates to the crate-wide
/// [`deny_envelope`] (F1.6.4) so this shares its exact field set/order/
/// length class with every other boundary's deny path. Includes the
/// correlation ID (not a protected detail — see `correlation` module docs)
/// so a legitimate caller/operator can report the specific denied request.
fn origin_denied(request: &Request) -> Response {
    deny_envelope(StatusCode::FORBIDDEN, "origin_not_allowed", correlation_id_of(request))
}

/// Server-side origin enforcement (see module docs for why the CORS layer
/// alone is not sufficient). Only layered on when `[server].remote_enabled =
/// true` (see `build_router`) — loopback/default mode never runs this.
pub async fn origin_middleware(
    State(state): State<Arc<ServerState>>,
    request: Request,
    next: Next,
) -> Response {
    let origin_header = request
        .headers()
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    if let Some(origin) = origin_header {
        let allowed = state
            .config
            .server
            .allowed_origins
            .iter()
            .any(|allowed| allowed == &origin);
        if !allowed {
            return origin_denied(&request);
        }
    }

    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_mode_yields_permissive_cors() {
        let layer = build_cors_layer(false, &[]);
        // Permissive CORS is opaque to introspect directly; construction not
        // panicking plus the documented behavior contract is what matters
        // here — exercised end-to-end in `tests/integration_api.rs`.
        let _ = layer;
    }

    #[test]
    fn remote_mode_with_empty_allowlist_constructs_fail_closed_layer() {
        let layer = build_cors_layer(true, &[]);
        let _ = layer;
    }

    #[test]
    fn remote_mode_with_configured_origins_constructs_restricted_layer() {
        let layer = build_cors_layer(true, &["https://kria.example.com".to_string()]);
        let _ = layer;
    }
}
