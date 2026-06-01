use super::types::N8nWorkflowConfig;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::PathBuf;

const DEFAULT_CALLBACK_FRESHNESS_WINDOW_SECS: u64 = 300;
const DEFAULT_FUTURE_CALLBACK_SKEW_SECS: u64 = 30;
const DEFAULT_API_KEY_ENV: &str = "KRIA_N8N_API_KEY";
const DEFAULT_API_KEY_FILE: &str = "~/.kria/secrets/n8n_api_key";
const DEFAULT_SIGNING_SECRET_ENV: &str = "KRIA_N8N_SIGNING_SECRET";
const DEFAULT_SIGNING_SECRET_FILE: &str = "~/.kria/secrets/n8n.key";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nRuntimeMode {
    External,
    ManagedDocker,
}

impl N8nRuntimeMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::External => "external",
            Self::ManagedDocker => "managed_docker",
        }
    }
}

impl Default for N8nRuntimeMode {
    fn default() -> Self {
        Self::External
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct N8nManagedDockerConfig {
    pub container_name: String,
    pub image: String,
    pub image_digest: String,
    pub bind_host: String,
    pub host_port: u16,
    pub container_port: u16,
    pub data_dir: String,
    pub network: String,
    pub restart_policy: String,
    pub pull_policy: String,
    pub host_gateway_name: String,
    pub privileged: bool,
    pub user: String,
    pub volume_mode: String,
    pub port_collision_policy: String,
    pub healthcheck_path: String,
    pub n8n_encryption_key_file: String,
    pub dashboard_auth_required: bool,
    pub basic_auth_user_env: String,
    pub basic_auth_password_file: String,
}

impl Default for N8nManagedDockerConfig {
    fn default() -> Self {
        Self {
            container_name: "kria-n8n".into(),
            image: "n8nio/n8n:2.22.5".into(),
            image_digest: "sha256:a49bc161141d6c4b9c495b5a6e3c7c1932e61d2ed2fe3fdca01262064b4b23ca"
                .into(),
            bind_host: "127.0.0.1".into(),
            host_port: 5678,
            container_port: 5678,
            data_dir: "~/.kria/n8n/docker".into(),
            network: "bridge".into(),
            restart_policy: "unless-stopped".into(),
            pull_policy: "if_missing".into(),
            host_gateway_name: "host.docker.internal".into(),
            privileged: false,
            user: String::new(),
            volume_mode: "rw".into(),
            port_collision_policy: "fail_with_guidance".into(),
            healthcheck_path: "/healthz".into(),
            n8n_encryption_key_file: "~/.kria/secrets/n8n_encryption_key".into(),
            dashboard_auth_required: true,
            basic_auth_user_env: "KRIA_N8N_BASIC_AUTH_USER".into(),
            basic_auth_password_file: "~/.kria/secrets/n8n_basic_auth_password".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct N8nConfig {
    pub config_version: u32,
    pub enabled: bool,
    #[serde(alias = "runtime_mode")]
    pub mode: N8nRuntimeMode,
    pub base_url: String,
    pub dashboard_url: String,
    pub api_key: String,
    pub api_key_env: String,
    pub api_key_file: String,
    pub api_key_keyring: String,
    pub signing_secret: String,
    pub signing_secret_env: String,
    pub signing_secret_file: String,
    pub signing_secret_keyring: String,
    pub callback_base_url: String,
    pub callback_path: String,
    pub request_timeout_secs: u64,
    pub max_payload_bytes: usize,
    pub auto_start: bool,
    pub open_dashboard_on_start: bool,
    pub open_dashboard_from_settings: bool,
    pub healthcheck_timeout_secs: u64,
    pub healthcheck_interval_secs: u64,
    pub execution_poll_interval_secs: u64,
    pub event_stream_enabled: bool,
    pub callback_freshness_window_secs: u64,
    pub future_callback_skew_secs: u64,
    pub last_connection_status: String,
    pub last_connection_message: String,
    pub last_connection_checked_at_ms: u64,
    pub managed_docker: N8nManagedDockerConfig,
    pub default_requested_by: String,
    pub workflows: Vec<N8nWorkflowConfig>,
}

impl N8nConfig {
    pub fn runtime_mode(&self) -> &N8nRuntimeMode {
        &self.mode
    }

    /// Resolve the signing secret from multiple sources (priority order):
    /// 1. Configured environment variable
    /// 2. Configured local secret file
    ///
    /// Returns the resolved secret or empty string if none found.
    pub fn resolve_signing_secret(&self) -> String {
        let env_name = if self.signing_secret_env.trim().is_empty() {
            DEFAULT_SIGNING_SECRET_ENV
        } else {
            self.signing_secret_env.trim()
        };
        if let Ok(secret) = std::env::var(env_name) {
            if !secret.trim().is_empty() {
                return secret;
            }
        }

        let path = self.signing_secret_file_path();
        if let Ok(secret) = std::fs::read_to_string(&path) {
            let trimmed = secret.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }

        String::new()
    }

    /// Resolve the n8n API key from env/file/manual config without exposing the
    /// value to callers that only need source metadata.
    pub fn resolve_api_key(&self) -> String {
        let env_name = if self.api_key_env.trim().is_empty() {
            DEFAULT_API_KEY_ENV
        } else {
            self.api_key_env.trim()
        };
        if let Ok(secret) = std::env::var(env_name) {
            if !secret.trim().is_empty() {
                return secret;
            }
        }

        if !self.api_key_file.trim().is_empty() {
            let path = Self::expand_config_path(&self.api_key_file);
            if let Ok(secret) = std::fs::read_to_string(&path) {
                let trimmed = secret.trim().to_string();
                if !trimmed.is_empty() {
                    return trimmed;
                }
            }
        }

        self.api_key.trim().to_string()
    }

    /// Move a deprecated literal config signing secret into the local secret
    /// file and redact the in-memory config field.
    pub fn migrate_literal_signing_secret_to_file(&mut self) -> io::Result<Option<PathBuf>> {
        let secret = self.signing_secret.trim().to_string();
        if secret.is_empty() {
            return Ok(None);
        }

        let path = self.signing_secret_file_path();
        let should_write = std::fs::read_to_string(&path)
            .map(|existing| existing.trim().is_empty())
            .unwrap_or(true);
        if should_write {
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            match write_secret_file(&path, &secret) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }

        self.signing_secret.clear();
        Ok(Some(path))
    }

    /// Move a deprecated literal config API key into the local secret file and
    /// redact the in-memory config field.
    pub fn migrate_literal_api_key_to_file(&mut self) -> io::Result<Option<PathBuf>> {
        let secret = self.api_key.trim().to_string();
        if secret.is_empty() {
            return Ok(None);
        }

        let path = self.api_key_file_path();
        if !path.exists() {
            match write_secret_file(&path, &secret) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }

        self.api_key.clear();
        Ok(Some(path))
    }

    /// Get effective config with resolved secret.
    /// Use this instead of raw config when building the catalog/client.
    pub fn with_resolved_secret(mut self) -> Self {
        if !self.api_key.trim().is_empty() && self.migrate_literal_api_key_to_file().is_err() {
            self.api_key = self.resolve_api_key();
        }
        if !self.signing_secret.trim().is_empty()
            && self.migrate_literal_signing_secret_to_file().is_err()
        {
            self.signing_secret.clear();
        }
        self.api_key = self.resolve_api_key();
        self.signing_secret = self.resolve_signing_secret();
        self
    }

    pub fn api_key_file_path(&self) -> PathBuf {
        if self.api_key_file.trim().is_empty() {
            Self::expand_config_path(DEFAULT_API_KEY_FILE)
        } else {
            Self::expand_config_path(&self.api_key_file)
        }
    }

    pub fn signing_secret_file_path(&self) -> PathBuf {
        if self.signing_secret_file.trim().is_empty() {
            Self::expand_config_path(DEFAULT_SIGNING_SECRET_FILE)
        } else {
            Self::expand_config_path(&self.signing_secret_file)
        }
    }

    pub fn expand_config_path(raw: &str) -> PathBuf {
        let value = raw.trim();
        if value == "~" {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            return PathBuf::from(home);
        }
        if let Some(rest) = value.strip_prefix("~/") {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            return PathBuf::from(home).join(rest);
        }
        PathBuf::from(value)
    }
}

impl Default for N8nConfig {
    fn default() -> Self {
        Self {
            config_version: 2,
            enabled: false,
            mode: N8nRuntimeMode::External,
            base_url: String::new(),
            dashboard_url: String::new(),
            api_key: String::new(),
            api_key_env: DEFAULT_API_KEY_ENV.into(),
            api_key_file: DEFAULT_API_KEY_FILE.into(),
            api_key_keyring: "kria/n8n/api_key".into(),
            signing_secret: String::new(),
            signing_secret_env: DEFAULT_SIGNING_SECRET_ENV.into(),
            signing_secret_file: DEFAULT_SIGNING_SECRET_FILE.into(),
            signing_secret_keyring: "kria/n8n/signing_secret".into(),
            callback_base_url: String::new(),
            callback_path: "/api/n8n/callback".into(),
            request_timeout_secs: 30,
            max_payload_bytes: 64 * 1024,
            auto_start: false,
            open_dashboard_on_start: false,
            open_dashboard_from_settings: true,
            healthcheck_timeout_secs: 5,
            healthcheck_interval_secs: 30,
            execution_poll_interval_secs: 5,
            event_stream_enabled: true,
            callback_freshness_window_secs: DEFAULT_CALLBACK_FRESHNESS_WINDOW_SECS,
            future_callback_skew_secs: DEFAULT_FUTURE_CALLBACK_SKEW_SECS,
            last_connection_status: "untested".into(),
            last_connection_message: String::new(),
            last_connection_checked_at_ms: 0,
            managed_docker: N8nManagedDockerConfig::default(),
            default_requested_by: "local-user".into(),
            workflows: Vec::new(),
        }
    }
}

fn write_secret_file(path: &PathBuf, secret: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(secret.as_bytes())?;
        file.write_all(b"\n")?;
        return Ok(());
    }

    #[cfg(not(unix))]
    {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?;
        file.write_all(secret.as_bytes())?;
        file.write_all(b"\n")?;
        Ok(())
    }
}
