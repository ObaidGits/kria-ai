//! Local API authentication via Bearer token.
//!
//! Generates a per-installation token at first use, stores it at
//! `~/.kria/api_token` (mode 0600), and validates incoming requests.
//!
//! # Security model
//!
//! - The local API binds to 127.0.0.1 only, so external network access is impossible.
//! - But on a multi-user system, any local user could connect to the API and
//!   issue arbitrary commands. The Bearer token prevents this.
//! - The token is generated cryptographically (32 bytes random base64-encoded).
//! - The token file has mode 0600 so only the owner can read it.
//! - Health endpoint (`/api/health`) is exempt to allow uptime monitoring.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use rand::RngCore;
use std::path::PathBuf;
use std::sync::OnceLock;
use tracing::{info, warn};

static API_TOKEN: OnceLock<String> = OnceLock::new();

/// Path to the API token file.
fn token_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".kria").join("api_token")
}

/// Generate a 32-byte cryptographically random token, base64-encoded.
fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Load existing token from disk or generate and persist a new one.
/// Idempotent — multiple calls return the same token.
pub fn ensure_api_token() -> String {
    if let Some(token) = API_TOKEN.get() {
        return token.clone();
    }

    let path = token_path();

    // Try to load existing token
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() && trimmed.len() >= 32 {
            let _ = API_TOKEN.set(trimmed.clone());
            info!(target: "api_auth", path = %path.display(), "Loaded existing API token");
            return trimmed;
        }
    }

    // Generate new token
    let token = generate_token();

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Write with restrictive permissions
    if let Err(e) = std::fs::write(&path, &token) {
        warn!(target: "api_auth", error = %e, "Failed to persist API token");
    } else {
        // Set file permissions to 0600 (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = std::fs::metadata(&path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o600);
                let _ = std::fs::set_permissions(&path, perms);
            }
        }
        info!(target: "api_auth", path = %path.display(), "Generated new API token");
    }

    let _ = API_TOKEN.set(token.clone());
    token
}

/// Get the current API token. Generates one if not yet set.
pub fn current_token() -> String {
    if let Some(t) = API_TOKEN.get() {
        return t.clone();
    }
    ensure_api_token()
}

/// Middleware that validates the Bearer token in the Authorization header.
///
/// Exempt paths:
/// - `/api/health` — allows uptime monitoring without auth
/// - `/api/auth/token` — provides the token to localhost clients
/// - `/api/n8n/callback` — uses HMAC signature verification (its own auth layer)
pub async fn auth_middleware(request: Request<Body>, next: Next) -> Result<Response, StatusCode> {
    let path = request.uri().path();

    // Exempt paths:
    // - /api/health — uptime monitoring
    // - /api/auth/token — localhost token retrieval
    // - /api/n8n/callback — n8n callbacks use HMAC signature verification (own auth)
    if path == "/api/health" || path == "/api/auth/token" || path == "/api/n8n/callback" {
        return Ok(next.run(request).await);
    }

    // Extract Authorization header
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    let token = current_token();
    let expected = format!("Bearer {}", token);

    match auth_header {
        Some(value) if value == expected => Ok(next.run(request).await),
        Some(_) => {
            warn!(target: "api_auth", path = %path, "Rejected request with invalid token");
            Err(StatusCode::UNAUTHORIZED)
        }
        None => {
            warn!(target: "api_auth", path = %path, "Rejected request without auth token");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

/// Token bootstrap endpoint.
///
/// Disabled by default: local automation should read `~/.kria/api_token`, which
/// is protected by mode 0600. For legacy diagnostics only, set
/// `KRIA_LOCAL_API_ALLOW_TOKEN_ENDPOINT=1` before starting KRIA.
pub async fn get_token_handler() -> impl IntoResponse {
    let allowed = std::env::var("KRIA_LOCAL_API_ALLOW_TOKEN_ENDPOINT")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !allowed {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({
                "status": "disabled",
                "message": "API token endpoint is disabled. Read ~/.kria/api_token as the same OS user.",
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "token": current_token(),
            "header": format!("Authorization: Bearer {}", current_token()),
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_at_least_32_chars() {
        let t = generate_token();
        assert!(t.len() >= 32);
    }

    #[test]
    fn token_is_url_safe() {
        let t = generate_token();
        for c in t.chars() {
            assert!(
                c.is_ascii_alphanumeric() || c == '-' || c == '_',
                "Token contains non-URL-safe char: {}",
                c
            );
        }
    }

    #[test]
    fn token_is_unique() {
        let t1 = generate_token();
        let t2 = generate_token();
        assert_ne!(t1, t2, "Generated tokens should be unique");
    }
}
