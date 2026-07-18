//! Vault-backed secret storage for configuration (settings-config-revamp Task 6).
//!
//! The config store (`SqliteConfigStore`) and the UI JSON NEVER hold plaintext
//! secrets ([`KriaConfig::redact_secrets`] / [`is_secret_field`]). Instead the
//! real secret values live in the encrypted [`SecretsVault`] (`~/.kria/vault.enc`),
//! keyed by their config field path (`config:<section>.<field>`, and
//! `config:providers.<id>.api_key` for the nested provider keys).
//!
//! At startup the effective config is [`hydrate`](SecretStore::hydrate)d from the
//! vault (so LLM/provider clients read real keys in memory as before); on save
//! the secrets are [`persist`](SecretStore::persist)ed to the vault.
//!
//! Security note (verified): without `KRIA_VAULT_PASSPHRASE` the vault key is a
//! random 0600 keyfile beside the vault — this protects against other-user reads,
//! not a local attacker. Prefer setting `KRIA_VAULT_PASSPHRASE`.

use std::sync::Arc;

use crate::auth::SecretsVault;
use crate::config::KriaConfig;

/// Vault-backed store for the config layer's secret fields.
pub struct SecretStore {
    vault: Arc<SecretsVault>,
}

fn secret_update(key: &str, value: &str) -> (String, Option<String>) {
    (
        key.to_string(),
        (!value.is_empty()).then(|| value.to_string()),
    )
}

impl SecretStore {
    /// Open the default vault (`~/.kria/vault.enc`).
    pub fn open_default() -> Result<Self, String> {
        Ok(Self {
            vault: Arc::new(SecretsVault::open_default().map_err(|e| e.to_string())?),
        })
    }

    /// Wrap an existing vault handle (tests / shared vault).
    pub fn new(vault: Arc<SecretsVault>) -> Self {
        Self { vault }
    }

    fn get(&self, key: &str) -> Option<String> {
        self.vault.get(key).filter(|value| !value.is_empty())
    }

    /// Persist every secret field in one atomic encrypted-vault update.
    pub fn persist(&self, cfg: &KriaConfig) -> Result<(), String> {
        let mut updates = vec![
            secret_update("config:llm.cloud_api_key", &cfg.llm.cloud_api_key),
            secret_update("config:planner.cloud_api_key", &cfg.planner.cloud_api_key),
            secret_update("config:server.jwt_secret", &cfg.server.jwt_secret),
            secret_update("config:telegram.bot_token", &cfg.telegram.bot_token),
            secret_update(
                "config:image_generation.hf_inference_token",
                &cfg.image_generation.hf_inference_token,
            ),
        ];
        updates.extend(cfg.providers.providers.iter().map(|provider| {
            secret_update(
                &format!("config:providers.{}.api_key", provider.id),
                &provider.endpoint.api_key,
            )
        }));
        self.vault
            .apply_updates(&updates)
            .map_err(|error| error.to_string())
    }

    /// Fill `cfg`'s secret fields from the vault (in-memory runtime use). Only
    /// non-empty vault entries overwrite; missing entries leave the field as-is
    /// (which may already have been set by an env override — env still wins
    /// because `apply_env_and_sync` runs during resolve before hydration is
    /// applied on top only when the vault actually has a value).
    pub fn hydrate(&self, cfg: &mut KriaConfig) {
        if let Some(v) = self.get("config:llm.cloud_api_key") {
            cfg.llm.cloud_api_key = v;
        }
        if let Some(v) = self.get("config:planner.cloud_api_key") {
            cfg.planner.cloud_api_key = v;
        }
        if let Some(v) = self.get("config:server.jwt_secret") {
            cfg.server.jwt_secret = v;
        }
        if let Some(v) = self.get("config:telegram.bot_token") {
            cfg.telegram.bot_token = v;
        }
        if let Some(v) = self.get("config:image_generation.hf_inference_token") {
            cfg.image_generation.hf_inference_token = v;
        }
        for p in &mut cfg.providers.providers {
            if let Some(v) = self.get(&format!("config:providers.{}.api_key", p.id)) {
                p.endpoint.api_key = v;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault() -> (SecretStore, tempdir_guard::TempDir) {
        let dir = tempdir_guard::TempDir::new();
        let vault =
            SecretsVault::open(dir.path().join("vault.enc"), dir.path()).expect("open temp vault");
        (SecretStore::new(Arc::new(vault)), dir)
    }

    #[test]
    fn persist_then_hydrate_roundtrips_secrets() {
        let (store, _guard) = temp_vault();
        let mut cfg = KriaConfig::default();
        cfg.llm.cloud_api_key = "sk-abc".to_string();
        cfg.telegram.bot_token = "bot-123".to_string();
        if let Some(p) = cfg.providers.providers.first_mut() {
            p.endpoint.api_key = "prov-key".to_string();
        }
        store.persist(&cfg).unwrap();

        // A fresh config (secrets empty) hydrates to the persisted values.
        let mut fresh = KriaConfig::default();
        store.hydrate(&mut fresh);
        assert_eq!(fresh.llm.cloud_api_key, "sk-abc");
        assert_eq!(fresh.telegram.bot_token, "bot-123");
        assert_eq!(
            fresh.providers.providers.first().unwrap().endpoint.api_key,
            "prov-key"
        );
    }

    #[test]
    fn empty_value_clears_vault_entry() {
        let (store, _guard) = temp_vault();
        let mut cfg = KriaConfig::default();
        cfg.llm.cloud_api_key = "sk-abc".to_string();
        store.persist(&cfg).unwrap();
        // Now clear it.
        cfg.llm.cloud_api_key.clear();
        store.persist(&cfg).unwrap();
        let mut fresh = KriaConfig::default();
        store.hydrate(&mut fresh);
        assert!(fresh.llm.cloud_api_key.is_empty());
    }
}

#[cfg(test)]
mod tempdir_guard {
    use std::path::{Path, PathBuf};

    /// Minimal self-cleaning temp directory (avoids adding a dev-dependency).
    pub struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        pub fn new() -> Self {
            let mut p = std::env::temp_dir();
            let unique = format!(
                "kria-secret-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            p.push(unique);
            std::fs::create_dir_all(&p).expect("create temp dir");
            Self { path: p }
        }

        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
