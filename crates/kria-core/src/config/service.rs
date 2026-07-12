//! `ConfigService` — the single serialized reader/writer for `KriaConfig`
//! (settings-config-revamp, Task 1).
//!
//! Design contract (`design.md` C1 / C1.1 / C1.2):
//! - Lives in `kria-core` and MUST NOT call `kria-desktop` apply services. Its
//!   write path is `validate → persist → bump version → publish`. Runtime
//!   effects are executed by the desktop-side effect executor (C5) that
//!   subscribes to `KriaEvent::ConfigChanged`.
//! - Serializes all writes via an internal async mutex (single writer,
//!   no lost update — Property 4). A monotonic `version` lets lossy-broadcast
//!   subscribers reconcile by re-reading current config.
//! - Preserves the `KriaConfig` serde shape (frontend `get_settings` contract).
//!
//! Task 1 scope: NO storage change yet. `patch`/`patch_batch` still persist
//! through the existing [`KriaConfig::save`] (whole-file TOML). The SQLite
//! field-level store is wired in Task 4. Nothing calls this service until
//! Task 2 routes desktop reads/writes through it behind `KRIA_CONFIG_SERVICE`,
//! so this module is behaviourally inert (flag-off parity holds trivially).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::{Mutex, RwLock};

use crate::config::KriaConfig;
use crate::infra::event_bus::{EventBus, KriaEvent};

/// Persistence seam for the user config layer. Task 1 default is the existing
/// whole-file TOML save; Task 4 replaces it with the field-level SQLite store
/// behind `KRIA_CONFIG_BACKEND`. Injectable so tests avoid touching the real
/// `~/.kria/config.toml`.
pub trait ConfigPersist: Send + Sync {
    fn persist(&self, cfg: &KriaConfig) -> Result<(), String>;
}

/// Audit sink for permanent config changes (settings-config-revamp Task 15).
/// Implemented by the hash-chained [`crate::safety::AuditLogger`] so every
/// committed change is recorded durably (prior/new value, source, change-set id)
/// in addition to the in-memory undo ring. Secret fields are never passed here.
pub trait ConfigAuditSink: Send + Sync {
    fn record_config_change(
        &self,
        section: &str,
        field: &str,
        prior: Option<&serde_json::Value>,
        new: &serde_json::Value,
        source: &str,
        change_set_id: &str,
    );
}

/// Log config changes into the hash-chained audit ledger. `action = "config_change"`,
/// risk GREEN (the change is already schema-validated + risk-gated upstream by
/// `config_patch`/`patch_config`), decided by policy/system.
impl ConfigAuditSink for crate::safety::AuditLogger {
    fn record_config_change(
        &self,
        section: &str,
        field: &str,
        prior: Option<&serde_json::Value>,
        new: &serde_json::Value,
        source: &str,
        change_set_id: &str,
    ) {
        let params = serde_json::json!({
            "section": section,
            "field": field,
            "prior": prior,
            "new": new,
            "source": source,
            "change_set_id": change_set_id,
        });
        self.log(
            "config",
            "config_change",
            &params,
            crate::safety::RiskLevel::Green,
            crate::safety::audit::Decision::AutoExecuted,
            crate::safety::audit::DecidedBy::Policy,
        );
    }
}

/// Default persistence: the existing whole-file TOML save (`~/.kria/config.toml`).
pub struct TomlFilePersist;

impl ConfigPersist for TomlFilePersist {
    fn persist(&self, cfg: &KriaConfig) -> Result<(), String> {
        cfg.save().map_err(|e| e.to_string())
    }
}

/// No-op persistence for tests (keeps changes in-memory only).
pub struct NoopPersist;

impl ConfigPersist for NoopPersist {
    fn persist(&self, _cfg: &KriaConfig) -> Result<(), String> {
        Ok(())
    }
}

/// Who initiated a config change (recorded for audit/source tracking).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeSource {
    Ui,
    Prompt,
    Env,
    Migration,
    Import,
    System,
}

impl ChangeSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChangeSource::Ui => "ui",
            ChangeSource::Prompt => "prompt",
            ChangeSource::Env => "env",
            ChangeSource::Migration => "migration",
            ChangeSource::Import => "import",
            ChangeSource::System => "system",
        }
    }
}

/// A single field-level change request.
#[derive(Clone, Debug)]
pub struct Change {
    pub section: String,
    pub field: String,
    pub value: serde_json::Value,
}

impl Change {
    pub fn new(
        section: impl Into<String>,
        field: impl Into<String>,
        value: serde_json::Value,
    ) -> Self {
        Self {
            section: section.into(),
            field: field.into(),
            value,
        }
    }
}

/// Result of a committed single patch.
#[derive(Clone, Debug)]
pub struct AppliedChange {
    pub section: String,
    pub field: String,
    pub prior_value: Option<serde_json::Value>,
    pub new_value: serde_json::Value,
    pub version: u64,
}

/// Result of a committed batch (transaction group — design C1.2).
#[derive(Clone, Debug)]
pub struct AppliedChangeSet {
    pub change_set_id: String,
    pub changes: Vec<AppliedChange>,
    pub version: u64,
}

/// Errors surfaced by the write path.
#[derive(Debug, thiserror::Error)]
pub enum ConfigServiceError {
    #[error("unknown config section '{0}'")]
    UnknownSection(String),
    #[error("config is not an object (serialization invariant violated)")]
    NotAnObject,
    #[error("stale config version: expected {expected}, current {current} — re-read and retry")]
    StaleVersion { expected: u64, current: u64 },
    #[error("config serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("config persist error: {0}")]
    Persist(String),
}

/// The single source of truth for live configuration.
pub struct ConfigService {
    /// The live, effective config. Reads clone; writes go through `write_lock`.
    inner: Arc<RwLock<KriaConfig>>,
    /// Monotonic version, bumped on every committed change.
    version: Arc<AtomicU64>,
    /// Serializes writers so concurrent patches cannot interleave (Property 4).
    write_lock: Arc<Mutex<()>>,
    /// Change-event bus (the wired `infra::EventBus`; publishes
    /// `KriaEvent::ConfigChanged`).
    event_bus: Arc<EventBus>,
    /// Whole-blob persistence backend (TOML file — used when no field-level
    /// store is present).
    persist: Arc<dyn ConfigPersist>,
    /// Field-level SQLite store (settings-config-revamp Task 4). When present,
    /// writes go to `(section,key)` rows and reads resolve via the layered
    /// `code < default.toml < DB < env` path.
    store: Option<Arc<dyn crate::config::store::ConfigStore>>,
    /// Vault-backed secret store (Task 6). Secrets are persisted here (never in
    /// the config store) and hydrated into the effective config on resolve.
    secrets: Option<Arc<crate::config::secrets::SecretStore>>,
    /// Bounded change history for same-session undo (Task 15). Newest last.
    history: Arc<Mutex<Vec<AppliedChange>>>,
    /// Optional durable audit sink (Task 15): the hash-chained AuditLogger. When
    /// set, every committed non-secret change is recorded persistently in addition
    /// to the in-memory undo ring. Wired by the desktop at startup.
    audit_sink: std::sync::RwLock<Option<Arc<dyn ConfigAuditSink>>>,
    /// Startup barrier (N1): subscribers register before external changes flow.
    ready: Arc<AtomicBool>,
}

const MAX_HISTORY: usize = 100;

impl ConfigService {
    /// Build a service over an existing shared config handle and event bus,
    /// persisting through the default whole-file TOML save (Task 1 behaviour).
    /// The shared `Arc<RwLock<KriaConfig>>` is reused so existing AppState
    /// readers observe the same live value (Task 2 wiring).
    pub fn new(inner: Arc<RwLock<KriaConfig>>, event_bus: Arc<EventBus>) -> Self {
        Self::with_persist(inner, event_bus, Arc::new(TomlFilePersist))
    }

    /// Build a service with an explicit whole-blob persistence backend
    /// (tests inject `NoopPersist`).
    pub fn with_persist(
        inner: Arc<RwLock<KriaConfig>>,
        event_bus: Arc<EventBus>,
        persist: Arc<dyn ConfigPersist>,
    ) -> Self {
        Self {
            inner,
            version: Arc::new(AtomicU64::new(0)),
            write_lock: Arc::new(Mutex::new(())),
            event_bus,
            persist,
            store: None,
            secrets: None,
            history: Arc::new(Mutex::new(Vec::new())),
            audit_sink: std::sync::RwLock::new(None),
            ready: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Build a service backed by the field-level SQLite store (Task 4). Writes
    /// persist as `(section,key)` rows; reads resolve via the layered path.
    pub fn with_store(
        inner: Arc<RwLock<KriaConfig>>,
        event_bus: Arc<EventBus>,
        store: Arc<dyn crate::config::store::ConfigStore>,
    ) -> Self {
        Self::with_store_and_secrets(inner, event_bus, store, None)
    }

    /// Build a SQLite-backed service with a vault-backed secret store (Task 6).
    pub fn with_store_and_secrets(
        inner: Arc<RwLock<KriaConfig>>,
        event_bus: Arc<EventBus>,
        store: Arc<dyn crate::config::store::ConfigStore>,
        secrets: Option<Arc<crate::config::secrets::SecretStore>>,
    ) -> Self {
        Self {
            inner,
            version: Arc::new(AtomicU64::new(0)),
            write_lock: Arc::new(Mutex::new(())),
            event_bus,
            persist: Arc::new(NoopPersist),
            store: Some(store),
            secrets,
            history: Arc::new(Mutex::new(Vec::new())),
            audit_sink: std::sync::RwLock::new(None),
            ready: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Current monotonic config version.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// Startup barrier (N1): mark the service ready once all effect subscribers
    /// have registered. External/UI/prompt changes should only be processed
    /// after this. Currently advisory; enforced at call sites in later tasks.
    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Clone the full effective config (preserves serde shape).
    pub async fn get(&self) -> KriaConfig {
        self.inner.read().await.clone()
    }

    /// Read one section as a JSON value (by top-level section name).
    pub async fn get_section_value(&self, section: &str) -> Option<serde_json::Value> {
        let cfg = self.inner.read().await;
        let root = serde_json::to_value(&*cfg).ok()?;
        root.get(section).cloned()
    }

    /// Deserialize one section into a typed value.
    pub async fn get_section<T: for<'de> Deserialize<'de>>(&self, section: &str) -> Option<T> {
        let value = self.get_section_value(section).await?;
        serde_json::from_value(value).ok()
    }

    /// Full layered reload from storage (startup / external reload).
    /// SQLite backend: `code < default.toml < DB < env`. TOML backend: the
    /// legacy loader (`default.toml` + `~/.kria/config.toml` + env).
    pub async fn resolve(&self) -> anyhow::Result<KriaConfig> {
        match &self.store {
            Some(store) => {
                let mut cfg = KriaConfig::resolve_from_store(store.as_ref());
                // Hydrate secrets from the vault (they are never in the DB rows).
                if let Some(secrets) = &self.secrets {
                    secrets.hydrate(&mut cfg);
                }
                Ok(cfg)
            }
            None => KriaConfig::load(None),
        }
    }

    /// Bulk-replace the entire live config (the `update_settings` save path,
    /// which is inherently whole-config today). Persists, bumps the version,
    /// and publishes a wildcard `ConfigChanged`. Field-level UI patching lands
    /// in Task 11; this preserves the existing bulk-save contract for Task 2.
    pub async fn replace_all(
        &self,
        new_cfg: KriaConfig,
        source: ChangeSource,
    ) -> Result<u64, ConfigServiceError> {
        self.replace_all_checked(new_cfg, source, None).await
    }

    /// Like [`replace_all`] but with optimistic concurrency: pass the version the
    /// caller last observed; if the live version moved (a concurrent writer), the
    /// write is rejected with [`ConfigServiceError::StaleVersion`] so the caller can
    /// re-read + retry (lost-update prevention — Task 11).
    pub async fn replace_all_checked(
        &self,
        new_cfg: KriaConfig,
        source: ChangeSource,
        expected_version: Option<u64>,
    ) -> Result<u64, ConfigServiceError> {
        let _guard = self.write_lock.lock().await;
        if let Some(expected) = expected_version {
            let current = self.version();
            if expected != current {
                return Err(ConfigServiceError::StaleVersion { expected, current });
            }
        }
        // Capture the prior config so the whole-blob save can be audited at
        // field granularity (settings-config-revamp Task 15).
        let prior_json = serde_json::to_value(&*self.inner.read().await).ok();
        let new_json = serde_json::to_value(&new_cfg).ok();
        {
            let mut cfg = self.inner.write().await;
            *cfg = new_cfg;
            if let Some(store) = &self.store {
                // Persist only the user-layer deviations vs baseline-with-env,
                // so env-derived values are not captured as user overrides.
                cfg.write_user_layer_diff(store.as_ref(), source.as_str())
                    .map_err(ConfigServiceError::Persist)?;
                // Secrets go to the vault, never the config store.
                if let Some(secrets) = &self.secrets {
                    secrets.persist(&cfg);
                }
            } else {
                self.persist
                    .persist(&cfg)
                    .map_err(ConfigServiceError::Persist)?;
            }
        }
        let version = self.version.fetch_add(1, Ordering::AcqRel) + 1;
        self.event_bus.publish(KriaEvent::ConfigChanged {
            section: "*".to_string(),
            version,
        });

        // Audit the whole-blob save at field granularity (Task 15): diff prior vs
        // new and record each changed non-secret field into the hash-chained ledger.
        if let (Some(sink), Some(prior), Some(new)) = (
            self.audit_sink
                .read()
                .expect("config audit_sink lock poisoned")
                .clone(),
            prior_json,
            new_json,
        ) {
            let change_set_id = uuid::Uuid::new_v4().to_string();
            if let (Some(prior_obj), Some(new_obj)) = (prior.as_object(), new.as_object()) {
                for (section, new_sect) in new_obj {
                    let (Some(new_fields), prior_sect) =
                        (new_sect.as_object(), prior_obj.get(section))
                    else {
                        continue;
                    };
                    for (field, new_val) in new_fields {
                        if crate::config::is_secret_field(section, field) {
                            continue;
                        }
                        let prior_val = prior_sect.and_then(|s| s.get(field));
                        if prior_val != Some(new_val) {
                            sink.record_config_change(
                                section,
                                field,
                                prior_val,
                                new_val,
                                source.as_str(),
                                &change_set_id,
                            );
                        }
                    }
                }
            }
        }

        Ok(version)
    }

    /// Subscribe to change events (reconcile via `version` on `Lagged`).
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<KriaEvent> {
        self.event_bus.subscribe()
    }

    /// Apply a single field-level change: `validate → persist → bump → publish`.
    ///
    /// `expected_version` enables optimistic concurrency (Req 2.6): pass the
    /// version the caller last observed to reject a stale write.
    pub async fn patch(
        &self,
        section: &str,
        field: &str,
        value: serde_json::Value,
        source: ChangeSource,
        expected_version: Option<u64>,
    ) -> Result<AppliedChange, ConfigServiceError> {
        let set = self
            .patch_batch(
                vec![Change::new(section, field, value)],
                source,
                expected_version,
            )
            .await?;
        Ok(set
            .changes
            .into_iter()
            .next()
            .expect("batch of one yields one applied change"))
    }

    /// Apply a batch of changes atomically under a single write lock, one
    /// persist, one version bump. Effect collapsing/ordering (design C1.2) is a
    /// desktop concern (Task 8); here we guarantee an all-or-nothing persist and
    /// emit one `ConfigChanged` per touched section.
    pub async fn patch_batch(
        &self,
        changes: Vec<Change>,
        source: ChangeSource,
        expected_version: Option<u64>,
    ) -> Result<AppliedChangeSet, ConfigServiceError> {
        let _guard = self.write_lock.lock().await;

        // Optimistic concurrency check under the write lock.
        if let Some(expected) = expected_version {
            let current = self.version();
            if expected != current {
                return Err(ConfigServiceError::StaleVersion { expected, current });
            }
        }

        let mut cfg = self.inner.write().await;

        // Serialize the whole config to a JSON object, apply each change at
        // field granularity, then deserialize back. This gives generic
        // field-level patching without a per-field match and preserves the
        // serde shape exactly.
        let mut root = serde_json::to_value(&*cfg)?;
        {
            let obj = root
                .as_object_mut()
                .ok_or(ConfigServiceError::NotAnObject)?;

            // Pre-validate every section exists before mutating anything
            // (all-or-nothing).
            for ch in &changes {
                if !obj.contains_key(&ch.section) {
                    return Err(ConfigServiceError::UnknownSection(ch.section.clone()));
                }
            }

            for ch in &changes {
                let section_val = obj
                    .get_mut(&ch.section)
                    .ok_or_else(|| ConfigServiceError::UnknownSection(ch.section.clone()))?;
                let section_obj = section_val
                    .as_object_mut()
                    .ok_or(ConfigServiceError::NotAnObject)?;
                section_obj.insert(ch.field.clone(), ch.value.clone());
            }
        }

        // Capture prior values for the applied-change records.
        let prior_root = serde_json::to_value(&*cfg)?;
        let prior_of = |section: &str, field: &str| -> Option<serde_json::Value> {
            prior_root.get(section).and_then(|s| s.get(field)).cloned()
        };

        // Deserialize back into a typed config (serde(default) tolerant).
        let new_cfg: KriaConfig = serde_json::from_value(root)?;
        *cfg = new_cfg;

        // Persist: field-level rows when a SQLite store is present (Task 4),
        // else the whole-blob backend (TOML file — Task 1 default).
        if let Some(store) = &self.store {
            let mut touched_secret = false;
            for ch in &changes {
                // Never persist plaintext secrets to the config store — the
                // vault-backed SecretStore handles those. The in-memory value is
                // still applied so runtime clients see it.
                if crate::config::is_secret_field(&ch.section, &ch.field) {
                    touched_secret = true;
                    continue;
                }
                let json = serde_json::to_string(&ch.value)?;
                store
                    .put(&ch.section, &ch.field, &json, source.as_str())
                    .map_err(ConfigServiceError::Persist)?;
            }
            // If a secret field changed (or a provider key), persist secrets to
            // the vault from the updated in-memory config.
            if touched_secret {
                if let Some(secrets) = &self.secrets {
                    secrets.persist(&cfg);
                }
            }
        } else {
            self.persist
                .persist(&cfg)
                .map_err(ConfigServiceError::Persist)?;
        }
        drop(cfg);

        let version = self.version.fetch_add(1, Ordering::AcqRel) + 1;

        // Emit one ConfigChanged per distinct touched section.
        let mut seen = std::collections::BTreeSet::new();
        for ch in &changes {
            if seen.insert(ch.section.clone()) {
                self.event_bus.publish(KriaEvent::ConfigChanged {
                    section: ch.section.clone(),
                    version,
                });
            }
        }

        let _ = source; // recorded by the storage layer in Task 4; kept for API stability.

        let applied: Vec<AppliedChange> = changes
            .into_iter()
            .map(|ch| AppliedChange {
                prior_value: prior_of(&ch.section, &ch.field),
                new_value: ch.value,
                section: ch.section,
                field: ch.field,
                version,
            })
            .collect();

        let change_set_id = uuid::Uuid::new_v4().to_string();

        // Record in the bounded change history for same-session undo (Task 15).
        // Secret fields are not recorded (their prior/new values are secret).
        {
            let mut hist = self.history.lock().await;
            for ch in &applied {
                if crate::config::is_secret_field(&ch.section, &ch.field) {
                    continue;
                }
                hist.push(ch.clone());
            }
            let overflow = hist.len().saturating_sub(MAX_HISTORY);
            if overflow > 0 {
                hist.drain(0..overflow);
            }
        }

        // Durably record the change-set in the hash-chained audit ledger (Task 15).
        // Secrets are excluded (never write plaintext secret values to the ledger).
        if let Some(sink) = self
            .audit_sink
            .read()
            .expect("config audit_sink lock poisoned")
            .clone()
        {
            for ch in &applied {
                if crate::config::is_secret_field(&ch.section, &ch.field) {
                    continue;
                }
                sink.record_config_change(
                    &ch.section,
                    &ch.field,
                    ch.prior_value.as_ref(),
                    &ch.new_value,
                    source.as_str(),
                    &change_set_id,
                );
            }
        }

        Ok(AppliedChangeSet {
            change_set_id,
            changes: applied,
            version,
        })
    }

    /// Install a durable audit sink (the hash-chained AuditLogger). Called once by
    /// the desktop at startup (settings-config-revamp Task 15). Idempotent-safe.
    pub fn set_audit_sink(&self, sink: Arc<dyn ConfigAuditSink>) {
        *self
            .audit_sink
            .write()
            .expect("config audit_sink lock poisoned") = Some(sink);
    }

    /// Read one field's current value (read-back, e.g. "what is my theme?").
    pub async fn read_field(&self, section: &str, field: &str) -> Option<serde_json::Value> {
        let root = serde_json::to_value(&*self.inner.read().await).ok()?;
        root.get(section).and_then(|s| s.get(field)).cloned()
    }

    /// Undo the most recent recorded change by restoring its prior value
    /// (same-session; forward patch — never a history deletion). Returns the
    /// restored `(section, field)` or `None` if there is nothing to undo.
    pub async fn undo_last(&self) -> Option<(String, String)> {
        let last = {
            let mut hist = self.history.lock().await;
            hist.pop()
        }?;
        let prior = last.prior_value.clone().unwrap_or(serde_json::Value::Null);
        // Apply as a normal forward patch (records a NEW history entry).
        match self
            .patch(
                &last.section,
                &last.field,
                prior,
                ChangeSource::System,
                None,
            )
            .await
        {
            Ok(_) => Some((last.section, last.field)),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::store::ConfigStore as _;

    fn service() -> ConfigService {
        let cfg = Arc::new(RwLock::new(KriaConfig::default()));
        let bus = Arc::new(EventBus::new(64));
        // NoopPersist keeps the test hermetic — never touches ~/.kria/config.toml.
        ConfigService::with_persist(cfg, bus, Arc::new(NoopPersist))
    }

    /// Test audit sink that records every change it is handed.
    #[derive(Default)]
    struct RecordingSink {
        changes: std::sync::Mutex<Vec<(String, String, String)>>,
    }
    impl ConfigAuditSink for RecordingSink {
        fn record_config_change(
            &self,
            section: &str,
            field: &str,
            _prior: Option<&serde_json::Value>,
            new: &serde_json::Value,
            source: &str,
            _change_set_id: &str,
        ) {
            self.changes.lock().unwrap().push((
                section.to_string(),
                field.to_string(),
                format!("{new}:{source}"),
            ));
        }
    }

    #[tokio::test]
    async fn committed_change_is_recorded_in_audit_sink() {
        let svc = service();
        let sink = Arc::new(RecordingSink::default());
        svc.set_audit_sink(sink.clone());

        svc.patch(
            "ui",
            "theme",
            serde_json::json!("dark"),
            ChangeSource::Ui,
            None,
        )
        .await
        .expect("patch ok");

        let recorded = sink.changes.lock().unwrap().clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "ui");
        assert_eq!(recorded[0].1, "theme");
        assert!(recorded[0].2.contains("dark"));
    }

    #[tokio::test]
    async fn get_settings_output_is_identical_across_both_paths() {
        // Property 1 / Req 12.1: get_settings must be byte-identical whether it
        // routes through ConfigService (flag on) or reads the handle directly
        // (flag off). Both clone the SAME config and redact the SAME way, so the
        // serialized+redacted JSON must match exactly.
        let cfg = Arc::new(RwLock::new(KriaConfig::default()));
        {
            let mut c = cfg.write().await;
            c.ui.theme = "dark".to_string();
            c.llm.cloud_api_key = "sk-secret-value".to_string();
        }
        let bus = Arc::new(EventBus::new(16));
        let svc = ConfigService::with_persist(cfg.clone(), bus, Arc::new(NoopPersist));

        // Path A (flag off): read the shared handle directly + redact.
        let mut direct = cfg.read().await.clone();
        direct.redact_secrets();
        let json_direct = serde_json::to_value(&direct).unwrap();

        // Path B (flag on): read via ConfigService + redact.
        let mut via_service = svc.get().await;
        via_service.redact_secrets();
        let json_service = serde_json::to_value(&via_service).unwrap();

        assert_eq!(
            json_direct, json_service,
            "get_settings must be byte-identical across paths"
        );
        // And the secret must be redacted (not leaked) in both.
        assert_ne!(
            json_service["llm"]["cloud_api_key"],
            serde_json::json!("sk-secret-value")
        );
        assert_eq!(json_service["ui"]["theme"], serde_json::json!("dark"));
    }

    #[tokio::test]
    async fn replace_all_audits_changed_fields() {
        let svc = service();
        let sink = Arc::new(RecordingSink::default());
        svc.set_audit_sink(sink.clone());

        // Whole-blob save that flips ui.theme — must be recorded (Task 15).
        let mut new_cfg = svc.get().await;
        let before = new_cfg.ui.theme.clone();
        new_cfg.ui.theme = if before == "dark" {
            "light".into()
        } else {
            "dark".into()
        };
        svc.replace_all(new_cfg, ChangeSource::Ui)
            .await
            .expect("replace_all ok");

        let recorded = sink.changes.lock().unwrap().clone();
        assert!(
            recorded.iter().any(|(s, f, _)| s == "ui" && f == "theme"),
            "whole-blob theme change should be audited, got {recorded:?}"
        );
    }

    #[tokio::test]
    async fn secret_change_is_not_sent_to_audit_sink() {
        let svc = service();
        let sink = Arc::new(RecordingSink::default());
        svc.set_audit_sink(sink.clone());

        // A secret field must never be written to the audit ledger in plaintext.
        let _ = svc
            .patch(
                "llm",
                "cloud_api_key",
                serde_json::json!("sk-secret"),
                ChangeSource::Ui,
                None,
            )
            .await;

        assert!(
            sink.changes.lock().unwrap().is_empty(),
            "secret field must not be recorded in the audit sink"
        );
    }

    #[tokio::test]
    async fn patch_round_trips_a_field() {
        let svc = service();
        assert_eq!(svc.version(), 0);

        let applied = svc
            .patch(
                "ui",
                "theme",
                serde_json::json!("dark"),
                ChangeSource::Ui,
                None,
            )
            .await
            .expect("patch ok");

        assert_eq!(applied.version, 1);
        assert_eq!(applied.new_value, serde_json::json!("dark"));
        // Read back through get(): the field actually changed.
        assert_eq!(svc.get().await.ui.theme, "dark");
        assert_eq!(svc.version(), 1);
    }

    #[tokio::test]
    async fn version_increments_monotonically() {
        let svc = service();
        for expected in 1..=3u64 {
            svc.patch(
                "ui",
                "window_width",
                serde_json::json!(1000 + expected),
                ChangeSource::Ui,
                None,
            )
            .await
            .unwrap();
            assert_eq!(svc.version(), expected);
        }
    }

    #[tokio::test]
    async fn stale_version_is_rejected() {
        let svc = service();
        // current version is 0; pass a wrong expected version.
        let err = svc
            .patch(
                "ui",
                "theme",
                serde_json::json!("dark"),
                ChangeSource::Ui,
                Some(99),
            )
            .await
            .unwrap_err();
        matches!(err, ConfigServiceError::StaleVersion { .. });
        // nothing changed, version unmoved.
        assert_eq!(svc.version(), 0);
        assert_ne!(svc.get().await.ui.theme, "dark");
    }

    #[tokio::test]
    async fn unknown_section_is_rejected_atomically() {
        let svc = service();
        let batch = vec![
            Change::new("ui", "theme", serde_json::json!("dark")),
            Change::new("does_not_exist", "x", serde_json::json!(1)),
        ];
        let err = svc
            .patch_batch(batch, ChangeSource::Ui, None)
            .await
            .unwrap_err();
        matches!(err, ConfigServiceError::UnknownSection(_));
        // all-or-nothing: the valid change in the batch was NOT applied.
        assert_ne!(svc.get().await.ui.theme, "dark");
        assert_eq!(svc.version(), 0);
    }

    #[tokio::test]
    async fn batch_applies_all_fields_with_one_version_bump() {
        let svc = service();
        let batch = vec![
            Change::new("ui", "theme", serde_json::json!("dark")),
            Change::new("ui", "high_contrast", serde_json::json!(true)),
        ];
        let set = svc
            .patch_batch(batch, ChangeSource::Ui, None)
            .await
            .unwrap();
        assert_eq!(set.version, 1); // single bump for the whole batch
        assert_eq!(set.changes.len(), 2);
        assert!(!set.change_set_id.is_empty());
        let cfg = svc.get().await;
        assert_eq!(cfg.ui.theme, "dark");
        assert!(cfg.ui.high_contrast);
    }

    #[tokio::test]
    async fn change_events_are_published_per_section() {
        let svc = service();
        let mut rx = svc.subscribe();
        svc.patch(
            "ui",
            "theme",
            serde_json::json!("dark"),
            ChangeSource::Ui,
            None,
        )
        .await
        .unwrap();
        let evt = rx.try_recv().expect("one event");
        match evt {
            KriaEvent::ConfigChanged { section, version } => {
                assert_eq!(section, "ui");
                assert_eq!(version, 1);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn startup_barrier_flag() {
        let svc = service();
        assert!(!svc.is_ready());
        svc.mark_ready();
        assert!(svc.is_ready());
    }

    #[tokio::test]
    async fn get_section_deserializes() {
        let svc = service();
        svc.patch(
            "ui",
            "theme",
            serde_json::json!("dark"),
            ChangeSource::Ui,
            None,
        )
        .await
        .unwrap();
        let ui: crate::config::UiConfig = svc.get_section("ui").await.expect("ui section");
        assert_eq!(ui.theme, "dark");
    }

    fn sqlite_service() -> (ConfigService, Arc<crate::config::store::SqliteConfigStore>) {
        let store = Arc::new(crate::config::store::SqliteConfigStore::open_in_memory().unwrap());
        let cfg = Arc::new(RwLock::new(KriaConfig::default()));
        let bus = Arc::new(EventBus::new(64));
        let svc = ConfigService::with_store(cfg, bus, store.clone());
        (svc, store)
    }

    #[tokio::test]
    async fn sqlite_backend_writes_field_rows() {
        let (svc, store) = sqlite_service();
        svc.patch(
            "ui",
            "theme",
            serde_json::json!("dark"),
            ChangeSource::Prompt,
            None,
        )
        .await
        .unwrap();

        let rows = store.all().unwrap();
        let row = rows
            .iter()
            .find(|r| r.section == "ui" && r.key == "theme")
            .expect("theme row persisted");
        assert_eq!(row.value_json, "\"dark\"");
        assert_eq!(row.source, "prompt");
        // in-memory reflects it too
        assert_eq!(svc.get().await.ui.theme, "dark");
    }

    #[tokio::test]
    async fn resolve_from_store_layers_db_over_baseline() {
        let (svc, store) = sqlite_service();
        svc.patch(
            "ui",
            "theme",
            serde_json::json!("dark"),
            ChangeSource::Ui,
            None,
        )
        .await
        .unwrap();
        // Fresh resolve (baseline default.toml/code < DB rows < env) reflects the row.
        let resolved = KriaConfig::resolve_from_store(store.as_ref());
        assert_eq!(resolved.ui.theme, "dark");
    }

    #[tokio::test]
    async fn read_field_returns_current_value() {
        let svc = service();
        svc.patch(
            "ui",
            "theme",
            serde_json::json!("dark"),
            ChangeSource::Ui,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            svc.read_field("ui", "theme").await,
            Some(serde_json::json!("dark"))
        );
        assert_eq!(svc.read_field("ui", "no_such").await, None);
    }

    #[tokio::test]
    async fn undo_last_restores_prior_value() {
        let svc = service();
        // default theme is "light"; change to dark, then undo.
        let before = svc.get().await.ui.theme.clone();
        svc.patch(
            "ui",
            "theme",
            serde_json::json!("dark"),
            ChangeSource::Ui,
            None,
        )
        .await
        .unwrap();
        assert_eq!(svc.get().await.ui.theme, "dark");
        let undone = svc.undo_last().await;
        assert_eq!(undone, Some(("ui".to_string(), "theme".to_string())));
        assert_eq!(svc.get().await.ui.theme, before);
    }

    #[tokio::test]
    async fn undo_with_no_history_is_none() {
        let svc = service();
        assert_eq!(svc.undo_last().await, None);
    }

    #[tokio::test]
    async fn secret_fields_are_not_persisted_to_store() {
        let (svc, store) = sqlite_service();
        // Patch a secret field: applied in-memory but NEVER written to the DB.
        svc.patch(
            "llm",
            "cloud_api_key",
            serde_json::json!("sk-secret-123"),
            ChangeSource::Ui,
            None,
        )
        .await
        .unwrap();
        assert_eq!(svc.get().await.llm.cloud_api_key, "sk-secret-123");
        let rows = store.all().unwrap();
        assert!(
            !rows
                .iter()
                .any(|r| r.section == "llm" && r.key == "cloud_api_key"),
            "secret must not be persisted as a plaintext row"
        );
    }

    #[test]
    fn user_layer_diff_never_writes_secrets() {
        let store = crate::config::store::SqliteConfigStore::open_in_memory().unwrap();
        let mut cfg = KriaConfig::default();
        cfg.llm.cloud_api_key = "sk-should-not-persist".to_string();
        cfg.ui.theme = "dark".to_string();
        cfg.write_user_layer_diff(&store, "import").unwrap();
        let rows = store.all().unwrap();
        assert!(rows.iter().any(|r| r.section == "ui" && r.key == "theme"));
        assert!(!rows
            .iter()
            .any(|r| r.key == "cloud_api_key" || r.value_json.contains("sk-should-not-persist")));
    }

    #[test]
    fn import_parity_via_user_layer_diff() {
        // Task 5: importing a user config's deviations into rows and resolving
        // back yields the same user-visible values (round-trip parity).
        let store = crate::config::store::SqliteConfigStore::open_in_memory().unwrap();
        let mut cfg = KriaConfig::default();
        cfg.ui.theme = "dark".to_string();
        cfg.ui.font_scale = 1.25;
        cfg.write_user_layer_diff(&store, "import").unwrap();

        let resolved = KriaConfig::resolve_from_store(&store);
        assert_eq!(resolved.ui.theme, "dark");
        assert_eq!(resolved.ui.font_scale, 1.25);
        // schema version stays pinned after writes.
        assert_eq!(store.config_version(), crate::config::CONFIG_SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn replace_all_writes_only_user_deviations() {
        let (svc, store) = sqlite_service();
        let mut new_cfg = KriaConfig::default();
        new_cfg.ui.theme = "dark".to_string(); // deviates from code/baseline default ("light")
        svc.replace_all(new_cfg, ChangeSource::Ui).await.unwrap();

        let rows = store.all().unwrap();
        // The deviating field is persisted...
        assert!(rows
            .iter()
            .any(|r| r.section == "ui" && r.key == "theme" && r.value_json == "\"dark\""));
        // ...and we did NOT write a row for every field of every section.
        assert!(
            rows.len() < 50,
            "expected a minimal user layer, got {} rows",
            rows.len()
        );
    }
}
