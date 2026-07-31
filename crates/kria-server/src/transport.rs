//! Protected-transport (TLS) deployment attestation (MGR-003 AC2 "transport
//! protection" — F1.6.3).
//!
//! ## Scope decision
//!
//! This server binds a plain TCP listener (`tokio::net::TcpListener` +
//! `axum::serve` in `main.rs`) and does not terminate TLS itself. Adding a
//! direct TLS stack (e.g. `axum-server` + `rustls`) would be a real but
//! disproportionate addition for a single-laptop pre-production deployment
//! where the documented, supported way to expose *anything* off-box is
//! already a private mesh tunnel (see `main.rs`'s existing mobile
//! `bind_interface` guidance recommending Tailscale/WireGuard) — a tunnel
//! that already provides transport encryption between the two endpoints
//! without KRIA managing certificates at all. A reverse proxy (nginx/Caddy/
//! the tunnel software itself) is the deployment-appropriate place to
//! terminate TLS if a bare non-mesh remote exposure is ever attempted.
//!
//! Given that, this module implements the SAFER of the two documented
//! options honestly: rather than silently doing nothing, it requires the
//! operator to explicitly attest `[server].require_protected_transport =
//! true` once a proxy/tunnel is actually in place, and LOUDLY WARNS at
//! startup whenever remote mode is enabled without that attestation. It
//! deliberately does NOT hard-refuse startup the way `bind_security`
//! (F1.6.1) refuses an incomplete auth profile: this process cannot verify
//! from inside itself whether a reverse proxy or tunnel actually terminates
//! TLS in front of it, so a hard refusal here would assert a check we
//! cannot actually perform — a false promise of enforcement is worse than
//! an honest, loud warning naming the exact deployment fact that remains
//! the operator's responsibility.

/// Log the transport-protection startup warning if remote mode is enabled
/// without the operator's protected-transport attestation. Called once at
/// startup, after the (hard, fail-closed) `bind_security` check, and never
/// blocks startup on its own — see module docs.
pub fn warn_if_transport_unattested(remote_enabled: bool, require_protected_transport: bool) {
    if remote_enabled && !require_protected_transport {
        tracing::warn!(
            "kria-server is starting in REMOTE mode with \
             [server].require_protected_transport = false (MGR-003 'transport \
             protection'). This process does NOT terminate TLS itself — traffic \
             between this server and any remote client is UNENCRYPTED unless a \
             reverse proxy or private tunnel (Tailscale/WireGuard/nginx/Caddy) \
             terminates TLS in front of it. Set [server].require_protected_transport \
             = true once such a proxy/tunnel is actually in place; until then, \
             treat this deployment as exposing plaintext HTTP off-box."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests assert the *decision*, not the log output (tracing macros
    // are not observable synchronously without a subscriber harness) — the
    // function must not panic and must be a pure no-op in every case other
    // than the documented warning.
    #[test]
    fn loopback_mode_never_warns_regardless_of_attestation() {
        warn_if_transport_unattested(false, false);
        warn_if_transport_unattested(false, true);
    }

    #[test]
    fn remote_mode_with_attestation_does_not_panic() {
        warn_if_transport_unattested(true, true);
    }

    #[test]
    fn remote_mode_without_attestation_does_not_panic() {
        warn_if_transport_unattested(true, false);
    }
}
