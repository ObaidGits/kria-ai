//! Authentication & secrets foundation (Phase 0).
//!
//! Two pieces gate every first-party integration:
//!   * [`vault`] — an encrypted, on-device store for credentials (AES-256-GCM
//!     + Argon2id). Replaces plaintext `.env` storage of secrets.
//!   * [`oauth`] — Authorization-Code + PKCE login for Google / GitHub /
//!     Microsoft, with tokens persisted in the vault and transparent refresh.
//!
//! Security: secret *values* are never logged. Reference secrets by key name.

pub mod oauth;
pub mod vault;

pub use oauth::{AuthSession, OAuthEngine, OAuthProvider, ProviderConfig, StoredToken};
pub use vault::{SecretEntry, SecretsVault};

/// Unified error type for the auth subsystem.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("crypto error: {0}")]
    Crypto(String),

    #[error("vault decryption failed (wrong passphrase or corrupted vault)")]
    Decrypt,

    #[error("vault format error: {0}")]
    Format(String),

    #[error("http error: {0}")]
    Http(String),

    #[error("oauth provider error: {0}")]
    Provider(String),

    #[error("provider not connected: {0}")]
    NotConnected(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("oauth state mismatch (possible CSRF) — aborting")]
    StateMismatch,
}

/// Convenience result alias for the auth subsystem.
pub type Result<T> = std::result::Result<T, AuthError>;
