//! Bind-address loopback/remote startup gate (MGR-003 AC1/AC2/AC4/AC5 — F1.6.1).
//!
//! The server binds to loopback by default. If the resolved bind host is
//! non-loopback ("remote" case), startup MUST refuse to open the TCP listener
//! unless the operator has explicitly opted in (`[server] remote_enabled =
//! true`) and a minimal security profile is present. This module performs
//! that check atomically, before any socket accept — the check itself
//! performs no I/O and cannot race with an in-flight request.
//!
//! Scope note: this is a **partial** hardening step. It closes the loopback
//! boundary (AC1) and the fail-closed remote-startup boundary (AC4/AC5) only.
//! It intentionally does NOT implement:
//! - validated identity/session/expiry/replay semantics (F1.6.2), or
//! - origin allowlisting, transport protection, or request/rate limits
//!   (F1.6.3).
//!
//! Real deployments binding non-loopback still need F1.6.2/F1.6.3 before the
//! remote surface is safe; this gate only prevents the *worst* case (a
//! non-loopback bind with zero explicit opt-in / zero auth configured).

use std::net::IpAddr;

/// Why a non-loopback ("remote") server startup was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RemoteStartupError {
    /// The bind host does not parse as an IP address (e.g. a bare hostname).
    /// Refused rather than guessed, since we cannot prove it is loopback.
    #[error(
        "server.host '{0}' is not a valid IP address; refusing to determine loopback status. \
         Use an explicit IP (e.g. \"127.0.0.1\") in [server].host."
    )]
    UnresolvableHost(String),

    /// The resolved bind address is non-loopback but `[server].remote_enabled`
    /// was not explicitly set to `true` (MGR-003 AC2/AC5).
    #[error(
        "refusing to bind kria-server to non-loopback address {0}: [server].remote_enabled is \
         not set. Remote server mode requires explicit opt-in — set [server].remote_enabled = \
         true AND complete the remote security profile (enable_auth + jwt_secret at minimum; \
         see MGR-003) before exposing this server beyond localhost. Local Tauri operation is \
         unaffected by this refusal."
    )]
    RemoteNotEnabled(String),

    /// `remote_enabled = true` but the minimal security profile is incomplete
    /// (MGR-003 AC2/AC4). Full origin/transport/rate-limit/replay coverage is
    /// F1.6.2/F1.6.3 scope; this only checks for the presence of the identity
    /// primitives that already exist in `ServerConfig`.
    #[error(
        "refusing to bind kria-server to non-loopback address {0}: [server].remote_enabled is \
         true but the remote security profile is incomplete ({1}). Complete authentication \
         (enable_auth = true, non-empty jwt_secret) before remote startup. Local Tauri \
         operation is unaffected by this refusal."
    )]
    IncompleteSecurityProfile(String, &'static str),
}

/// Determine whether `host` is a loopback address.
///
/// Uses real IP parsing (`IpAddr::is_loopback`) rather than string equality,
/// so `127.0.0.2`, `::1`, and other loopback forms are recognized correctly.
/// Returns `Err` if `host` does not parse as an IP address at all (e.g. a
/// bare hostname) — we never guess loopback status for an unparseable host.
fn is_loopback_host(host: &str) -> Result<bool, RemoteStartupError> {
    host.trim()
        .parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .map_err(|_| RemoteStartupError::UnresolvableHost(host.to_string()))
}

/// Atomically validate the server's bind/security configuration before the
/// TCP listener is opened (MGR-003 AC1, AC2, AC4, AC5).
///
/// - Loopback `bind_host` (127.0.0.1, 127.0.0.2, ::1, …): always allowed, no
///   remote profile required — this is the default-safe path.
/// - Non-loopback `bind_host`: requires `server.remote_enabled == true` AND a
///   minimal security profile (`enable_auth == true` with a non-empty
///   `jwt_secret`); otherwise refuses startup with a typed error the caller
///   must handle by exiting before `axum::serve`, not by panicking — the
///   desktop Tauri app is a separate process and is unaffected either way.
pub fn validate_bind_security(
    bind_host: &str,
    remote_enabled: bool,
    enable_auth: bool,
    jwt_secret: &str,
) -> Result<(), RemoteStartupError> {
    if is_loopback_host(bind_host)? {
        return Ok(());
    }

    if !remote_enabled {
        return Err(RemoteStartupError::RemoteNotEnabled(bind_host.to_string()));
    }

    if !enable_auth {
        return Err(RemoteStartupError::IncompleteSecurityProfile(
            bind_host.to_string(),
            "enable_auth is false",
        ));
    }
    if jwt_secret.trim().is_empty() {
        return Err(RemoteStartupError::IncompleteSecurityProfile(
            bind_host.to_string(),
            "jwt_secret is empty",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_bind_proceeds_without_remote_enabled() {
        assert!(validate_bind_security("127.0.0.1", false, false, "").is_ok());
        assert!(validate_bind_security("::1", false, false, "").is_ok());
        // Non-canonical loopback range (127.0.0.0/8), still loopback per IpAddr.
        assert!(validate_bind_security("127.0.0.2", false, false, "").is_ok());
        assert!(validate_bind_security("127.255.255.255", false, false, "").is_ok());
    }

    #[test]
    fn non_loopback_bind_without_remote_enabled_is_refused() {
        let err = validate_bind_security("0.0.0.0", false, false, "").unwrap_err();
        assert!(matches!(err, RemoteStartupError::RemoteNotEnabled(_)));

        let err = validate_bind_security("192.168.1.5", false, true, "secret").unwrap_err();
        assert!(matches!(err, RemoteStartupError::RemoteNotEnabled(_)));

        let err = validate_bind_security("::", false, false, "").unwrap_err();
        assert!(matches!(err, RemoteStartupError::RemoteNotEnabled(_)));
    }

    #[test]
    fn non_loopback_bind_with_remote_enabled_but_incomplete_profile_is_refused() {
        // remote_enabled but auth disabled.
        let err = validate_bind_security("0.0.0.0", true, false, "").unwrap_err();
        assert!(matches!(
            err,
            RemoteStartupError::IncompleteSecurityProfile(_, "enable_auth is false")
        ));

        // remote_enabled + auth enabled but empty secret.
        let err = validate_bind_security("0.0.0.0", true, true, "").unwrap_err();
        assert!(matches!(
            err,
            RemoteStartupError::IncompleteSecurityProfile(_, "jwt_secret is empty")
        ));

        // whitespace-only secret is still "empty" for this purpose.
        let err = validate_bind_security("0.0.0.0", true, true, "   ").unwrap_err();
        assert!(matches!(
            err,
            RemoteStartupError::IncompleteSecurityProfile(_, "jwt_secret is empty")
        ));
    }

    #[test]
    fn non_loopback_bind_with_complete_minimal_profile_is_allowed() {
        assert!(validate_bind_security("0.0.0.0", true, true, "s3cr3t").is_ok());
        assert!(validate_bind_security("192.168.1.5", true, true, "s3cr3t").is_ok());
        assert!(validate_bind_security("::", true, true, "s3cr3t").is_ok());
    }

    #[test]
    fn unresolvable_host_is_refused_rather_than_guessed() {
        let err = validate_bind_security("localhost", false, false, "").unwrap_err();
        assert!(matches!(err, RemoteStartupError::UnresolvableHost(_)));

        let err = validate_bind_security("example.com", true, true, "s3cr3t").unwrap_err();
        assert!(matches!(err, RemoteStartupError::UnresolvableHost(_)));
    }
}
