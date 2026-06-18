//! OAuth 2.0 Authorization-Code + PKCE engine for Google, GitHub, Microsoft.
//!
//! Implemented directly on `reqwest` (a workspace dependency) with manual PKCE
//! rather than the `oauth2` crate — this keeps full control over each
//! provider's quirks and avoids a fragile dependency on an exact crate API.
//!
//! Tokens are persisted in the [`SecretsVault`] under `oauth/{provider}` and
//! refreshed transparently by [`OAuthEngine::get_access_token`].

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::vault::{SecretEntry, SecretsVault};
use super::{AuthError, Result};

/// Supported OAuth providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthProvider {
    Google,
    GitHub,
    Microsoft,
}

impl OAuthProvider {
    /// Stable lowercase identifier, also used as the vault key suffix.
    pub fn id(&self) -> &'static str {
        match self {
            OAuthProvider::Google => "google",
            OAuthProvider::GitHub => "github",
            OAuthProvider::Microsoft => "microsoft",
        }
    }

    /// Parse a provider from its stable identifier.
    pub fn from_id(s: &str) -> Option<Self> {
        match s {
            "google" => Some(OAuthProvider::Google),
            "github" => Some(OAuthProvider::GitHub),
            "microsoft" => Some(OAuthProvider::Microsoft),
            _ => None,
        }
    }
}

/// Static endpoint + credential configuration for one provider.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub auth_endpoint: String,
    pub token_endpoint: String,
    /// Extra query params appended to the authorization URL
    /// (e.g. Google's `access_type=offline`).
    pub extra_auth_params: Vec<(String, String)>,
}

/// A persisted OAuth token set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToken {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds when the access token expires (if known).
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default = "default_token_type")]
    pub token_type: String,
}

fn default_token_type() -> String {
    "Bearer".to_string()
}

impl StoredToken {
    /// True if the access token is expired (with a 60s safety margin).
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(exp) => chrono::Utc::now().timestamp() >= exp - 60,
            None => false,
        }
    }
}

/// An in-progress authorization handshake. Hold `state` and `pkce_verifier`
/// until the redirect code is captured.
#[derive(Debug, Clone)]
pub struct AuthSession {
    pub provider: OAuthProvider,
    pub url: String,
    pub state: String,
    pub pkce_verifier: String,
}

/// Raw token endpoint response.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
}

/// OAuth engine: drives the flows and persists tokens in the vault.
pub struct OAuthEngine {
    vault: Arc<SecretsVault>,
    http: reqwest::Client,
    providers: HashMap<&'static str, ProviderConfig>,
}

impl OAuthEngine {
    /// Build an engine with no providers registered. Use
    /// [`OAuthEngine::with_env_providers`] to auto-register from environment.
    pub fn new(vault: Arc<SecretsVault>) -> Self {
        Self {
            vault,
            http: reqwest::Client::new(),
            providers: HashMap::new(),
        }
    }

    /// Register providers whose credentials are present in the environment:
    /// `KRIA_{GOOGLE,GITHUB,MICROSOFT}_CLIENT_ID` (+ `_SECRET`),
    /// `KRIA_OAUTH_REDIRECT` (default `http://127.0.0.1:8765/callback`).
    pub fn with_env_providers(mut self) -> Self {
        let redirect = std::env::var("KRIA_OAUTH_REDIRECT")
            .unwrap_or_else(|_| "http://127.0.0.1:8765/callback".to_string());

        if let Ok(id) = std::env::var("KRIA_GOOGLE_CLIENT_ID") {
            if !id.is_empty() {
                self.register(
                    OAuthProvider::Google,
                    ProviderConfig {
                        client_id: id,
                        client_secret: std::env::var("KRIA_GOOGLE_CLIENT_SECRET").ok(),
                        redirect_uri: redirect.clone(),
                        scopes: vec![
                            "openid".into(),
                            "email".into(),
                            "https://www.googleapis.com/auth/gmail.readonly".into(),
                            "https://www.googleapis.com/auth/calendar.readonly".into(),
                        ],
                        auth_endpoint: "https://accounts.google.com/o/oauth2/v2/auth".into(),
                        token_endpoint: "https://oauth2.googleapis.com/token".into(),
                        extra_auth_params: vec![
                            ("access_type".into(), "offline".into()),
                            ("prompt".into(), "consent".into()),
                        ],
                    },
                );
            }
        }

        if let Ok(id) = std::env::var("KRIA_GITHUB_CLIENT_ID") {
            if !id.is_empty() {
                self.register(
                    OAuthProvider::GitHub,
                    ProviderConfig {
                        client_id: id,
                        client_secret: std::env::var("KRIA_GITHUB_CLIENT_SECRET").ok(),
                        redirect_uri: redirect.clone(),
                        scopes: vec!["repo".into(), "read:user".into()],
                        auth_endpoint: "https://github.com/login/oauth/authorize".into(),
                        token_endpoint: "https://github.com/login/oauth/access_token".into(),
                        extra_auth_params: vec![],
                    },
                );
            }
        }

        if let Ok(id) = std::env::var("KRIA_MICROSOFT_CLIENT_ID") {
            if !id.is_empty() {
                self.register(
                    OAuthProvider::Microsoft,
                    ProviderConfig {
                        client_id: id,
                        client_secret: std::env::var("KRIA_MICROSOFT_CLIENT_SECRET").ok(),
                        redirect_uri: redirect.clone(),
                        scopes: vec![
                            "offline_access".into(),
                            "User.Read".into(),
                            "Mail.Read".into(),
                        ],
                        auth_endpoint:
                            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".into(),
                        token_endpoint:
                            "https://login.microsoftonline.com/common/oauth2/v2.0/token".into(),
                        extra_auth_params: vec![],
                    },
                );
            }
        }

        self
    }

    /// Register or override a provider configuration.
    pub fn register(&mut self, provider: OAuthProvider, config: ProviderConfig) {
        self.providers.insert(provider.id(), config);
    }

    fn config(&self, provider: OAuthProvider) -> Result<&ProviderConfig> {
        self.providers
            .get(provider.id())
            .ok_or_else(|| AuthError::Config(format!("provider not configured: {}", provider.id())))
    }

    /// Step 1: build the authorization URL (with PKCE + CSRF state).
    pub fn begin_authorization(&self, provider: OAuthProvider) -> Result<AuthSession> {
        let cfg = self.config(provider)?;
        let state = random_b64url(24);
        let verifier = random_b64url(48);
        let challenge = pkce_challenge_s256(&verifier);

        let mut params: Vec<(&str, String)> = vec![
            ("response_type", "code".into()),
            ("client_id", cfg.client_id.clone()),
            ("redirect_uri", cfg.redirect_uri.clone()),
            ("scope", cfg.scopes.join(" ")),
            ("state", state.clone()),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256".into()),
        ];
        for (k, v) in &cfg.extra_auth_params {
            params.push((k.as_str(), v.clone()));
        }

        let mut ser = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in &params {
            ser.append_pair(k, v);
        }
        let query = ser.finish();
        let url = format!("{}?{}", cfg.auth_endpoint, query);

        Ok(AuthSession {
            provider,
            url,
            state,
            pkce_verifier: verifier,
        })
    }

    /// Step 3: exchange the authorization code for tokens and persist them.
    pub async fn complete_authorization(
        &self,
        provider: OAuthProvider,
        code: &str,
        pkce_verifier: &str,
    ) -> Result<StoredToken> {
        let cfg = self.config(provider)?;
        let mut form: Vec<(String, String)> = vec![
            ("grant_type".into(), "authorization_code".into()),
            ("code".into(), code.to_string()),
            ("redirect_uri".into(), cfg.redirect_uri.clone()),
            ("client_id".into(), cfg.client_id.clone()),
            ("code_verifier".into(), pkce_verifier.to_string()),
        ];
        if let Some(secret) = &cfg.client_secret {
            form.push(("client_secret".into(), secret.clone()));
        }

        let token = self.post_token(cfg, &form).await?;
        self.store_token(provider, &token)?;
        Ok(token)
    }

    /// Return a valid access token, refreshing transparently if expired.
    pub async fn get_access_token(&self, provider: OAuthProvider) -> Result<String> {
        let token = self
            .load_token(provider)?
            .ok_or_else(|| AuthError::NotConnected(provider.id().to_string()))?;

        if token.is_expired() && token.refresh_token.is_some() {
            let refreshed = self.refresh(provider).await?;
            return Ok(refreshed.access_token);
        }
        Ok(token.access_token)
    }

    /// Force a refresh using the stored refresh token.
    pub async fn refresh(&self, provider: OAuthProvider) -> Result<StoredToken> {
        let cfg = self.config(provider)?;
        let existing = self
            .load_token(provider)?
            .ok_or_else(|| AuthError::NotConnected(provider.id().to_string()))?;
        let refresh_token = existing
            .refresh_token
            .clone()
            .ok_or_else(|| AuthError::Provider("no refresh token available".into()))?;

        let mut form: Vec<(String, String)> = vec![
            ("grant_type".into(), "refresh_token".into()),
            ("refresh_token".into(), refresh_token.clone()),
            ("client_id".into(), cfg.client_id.clone()),
        ];
        if let Some(secret) = &cfg.client_secret {
            form.push(("client_secret".into(), secret.clone()));
        }

        let mut token = self.post_token(cfg, &form).await?;
        // Providers often omit the refresh token on refresh — keep the old one.
        if token.refresh_token.is_none() {
            token.refresh_token = Some(refresh_token);
        }
        self.store_token(provider, &token)?;
        Ok(token)
    }

    /// True if a token is stored for this provider.
    pub fn is_connected(&self, provider: OAuthProvider) -> bool {
        self.load_token(provider).ok().flatten().is_some()
    }

    /// Remove the stored token for a provider.
    pub fn revoke(&self, provider: OAuthProvider) -> Result<()> {
        self.vault.delete(&vault_key(provider))?;
        Ok(())
    }

    async fn post_token(
        &self,
        cfg: &ProviderConfig,
        form: &[(String, String)],
    ) -> Result<StoredToken> {
        let resp = self
            .http
            .post(&cfg.token_endpoint)
            .header(reqwest::header::ACCEPT, "application/json")
            .form(form)
            .send()
            .await
            .map_err(|e| AuthError::Http(e.to_string()))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| AuthError::Http(e.to_string()))?;

        if !status.is_success() {
            return Err(AuthError::Provider(format!(
                "token endpoint returned {status}: {}",
                truncate(&body, 300)
            )));
        }

        let parsed: TokenResponse = serde_json::from_str(&body)
            .map_err(|e| AuthError::Provider(format!("parse token response: {e}")))?;

        let scopes = parsed
            .scope
            .map(|s| s.split([' ', ',']).map(|x| x.to_string()).collect())
            .unwrap_or_default();
        let expires_at = parsed
            .expires_in
            .map(|secs| chrono::Utc::now().timestamp() + secs);

        Ok(StoredToken {
            access_token: parsed.access_token,
            refresh_token: parsed.refresh_token,
            expires_at,
            scopes,
            token_type: parsed.token_type.unwrap_or_else(default_token_type),
        })
    }

    fn store_token(&self, provider: OAuthProvider, token: &StoredToken) -> Result<()> {
        let value = serde_json::to_string(token)?;
        let entry = SecretEntry {
            value,
            updated_at: chrono::Utc::now().timestamp(),
            metadata: serde_json::json!({ "provider": provider.id() }),
        };
        self.vault.set_entry(&vault_key(provider), entry)
    }

    fn load_token(&self, provider: OAuthProvider) -> Result<Option<StoredToken>> {
        match self.vault.get(&vault_key(provider)) {
            Some(raw) => Ok(Some(serde_json::from_str(&raw)?)),
            None => Ok(None),
        }
    }
}

fn vault_key(provider: OAuthProvider) -> String {
    format!("oauth/{}", provider.id())
}

/// Generate a URL-safe base64 random string from `n` random bytes.
fn random_b64url(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// PKCE S256 challenge = base64url(sha256(verifier)).
fn pkce_challenge_s256(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

/// Capture the redirect authorization code via a one-shot loopback HTTP
/// listener (for desktop flows). Binds `127.0.0.1:port`, accepts a single
/// `GET /callback?code=...&state=...`, validates `state`, and returns the code.
pub async fn capture_authorization_code(port: u16, expected_state: &str) -> Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(AuthError::Io)?;

    let (mut stream, _) = listener.accept().await.map_err(AuthError::Io)?;
    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await.map_err(AuthError::Io)?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // First line: "GET /callback?code=...&state=... HTTP/1.1"
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| AuthError::Provider("malformed redirect request".into()))?;

    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            _ => {}
        }
    }

    let body = "<html><body><h2>KRIA: authorization received.</h2>\
                <p>You can close this tab and return to KRIA.</p></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;

    if state.as_deref() != Some(expected_state) {
        return Err(AuthError::StateMismatch);
    }
    code.ok_or_else(|| AuthError::Provider("no authorization code in redirect".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        // RFC 7636 Appendix B test vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(pkce_challenge_s256(verifier), expected);
    }

    #[test]
    fn stored_token_expiry_logic() {
        let mut t = StoredToken {
            access_token: "a".into(),
            refresh_token: Some("r".into()),
            expires_at: Some(chrono::Utc::now().timestamp() - 10),
            scopes: vec![],
            token_type: "Bearer".into(),
        };
        assert!(t.is_expired());
        t.expires_at = Some(chrono::Utc::now().timestamp() + 3600);
        assert!(!t.is_expired());
        t.expires_at = None;
        assert!(!t.is_expired());
    }

    #[test]
    #[serial_test::serial]
    fn begin_authorization_builds_url_with_pkce() {
        std::env::set_var("KRIA_VAULT_PASSPHRASE", "oauth-test-pass-000");
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(SecretsVault::open(dir.path().join("v.enc"), dir.path()).unwrap());
        let mut engine = OAuthEngine::new(vault);
        engine.register(
            OAuthProvider::Google,
            ProviderConfig {
                client_id: "cid".into(),
                client_secret: Some("sec".into()),
                redirect_uri: "http://127.0.0.1:8765/callback".into(),
                scopes: vec!["openid".into()],
                auth_endpoint: "https://accounts.google.com/o/oauth2/v2/auth".into(),
                token_endpoint: "https://oauth2.googleapis.com/token".into(),
                extra_auth_params: vec![("access_type".into(), "offline".into())],
            },
        );
        let session = engine.begin_authorization(OAuthProvider::Google).unwrap();
        assert!(session.url.contains("code_challenge_method=S256"));
        assert!(session.url.contains("client_id=cid"));
        assert!(session.url.contains("access_type=offline"));
        assert!(!session.state.is_empty());
        assert!(session.pkce_verifier.len() >= 43);
    }
}
