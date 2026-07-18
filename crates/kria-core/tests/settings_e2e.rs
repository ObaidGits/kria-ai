//! End-to-end verification of the settings-config-revamp using the SAME code
//! paths the UI's `config_prompt` / `get_settings` commands call internally:
//! SQLite backend + secret vault + prompt disambiguation + patch engine.
//!
//! Runs headless with a temp `$HOME` so `~/.kria` is isolated (no touching the
//! real user config). Single sequential test to avoid process-env races.

use std::sync::Arc;

use kria_core::config::nl::{SettingsHandler, SettingsOutcome, SettingsRequest};
use kria_core::config::store::ConfigStore;
use kria_core::config::{ChangeSource, ConfigService, KriaConfig, SecretStore, SqliteConfigStore};
use kria_core::infra::event_bus::EventBus;
use kria_core::tools::TriggerProvenance;
use tokio::sync::RwLock;

#[tokio::test(flavor = "current_thread")]
async fn settings_revamp_end_to_end() {
    // ── Isolated HOME so ~/.kria points at a temp dir ──────────────────────
    let tmp = std::env::temp_dir().join(format!("kria-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::env::set_var("HOME", &tmp);
    // Make sure no ambient env-locks interfere with this run.
    for v in ["KRIA_GPU_AUTOSCALE", "KRIA_LLM_MODE", "KRIA_TIER"] {
        std::env::remove_var(v);
    }

    let paths = kria_core::platform::paths::KriaPaths::resolve();
    let toml_path = paths.user_config();
    let bak_path = toml_path.with_extension("toml.bak");

    // ── Seed a legacy ~/.kria/config.toml (full, like save() writes) ───────
    // Start from the project baseline so the file is NOT sparse, then set a
    // user deviation (theme=dark) + a plaintext secret (simulating the old
    // insecure file).
    let mut legacy = KriaConfig::load_baseline_no_env();
    legacy.ui.theme = "dark".to_string();
    legacy.llm.cloud_api_key = "sk-legacy-secret".to_string();
    std::fs::write(&toml_path, toml::to_string_pretty(&legacy).unwrap()).unwrap();
    assert!(toml_path.exists());

    // ── Open the SQLite store + vault exactly like the desktop startup ─────
    let store: Arc<dyn ConfigStore> =
        Arc::new(SqliteConfigStore::open(&paths.db_path).expect("open sqlite config store"));
    let secrets = Arc::new(SecretStore::open_default().expect("open vault"));

    // ── One-time import (what maybe_import_toml_into_store does) ───────────
    assert!(store.all().unwrap().is_empty(), "store starts empty");
    let imported: KriaConfig =
        toml::from_str(&std::fs::read_to_string(&toml_path).unwrap()).unwrap();
    secrets.persist(&imported).unwrap();
    imported
        .write_user_layer_diff(store.as_ref(), "import")
        .unwrap();
    std::fs::rename(&toml_path, &bak_path).unwrap();

    // VERIFY migration:
    assert!(bak_path.exists(), "backup .bak created");
    assert!(!toml_path.exists(), "original config.toml moved");
    let rows = store.all().unwrap();
    assert!(
        rows.iter()
            .any(|r| r.section == "ui" && r.key == "theme" && r.value_json == "\"dark\""),
        "theme deviation persisted as a DB row"
    );
    assert!(
        !rows.iter().any(|r| r.key == "cloud_api_key"),
        "secret must NOT be a DB row"
    );
    assert!(
        !rows
            .iter()
            .any(|r| r.value_json.contains("sk-legacy-secret")),
        "secret value must never appear in the config store"
    );
    {
        // Confirm the secret is retrievable from the vault via hydrate.
        let mut probe = KriaConfig::default();
        secrets.hydrate(&mut probe);
        assert_eq!(
            probe.llm.cloud_api_key, "sk-legacy-secret",
            "secret migrated into the vault"
        );
    }
    println!("[e2e] migration OK: .bak created, theme row in DB, secret in vault not DB");

    // ── Build ConfigService exactly like the desktop (sqlite + secrets) ────
    let mut effective = KriaConfig::resolve_from_store(store.as_ref());
    secrets.hydrate(&mut effective);
    assert_eq!(effective.ui.theme, "dark", "resolve reflects DB row");
    assert_eq!(
        effective.llm.cloud_api_key, "sk-legacy-secret",
        "resolve hydrates secret from vault"
    );

    let inner = Arc::new(RwLock::new(effective));
    let bus = Arc::new(EventBus::new(64));
    let service =
        ConfigService::with_store_and_secrets(inner, bus, store.clone(), Some(secrets.clone()));

    // ── Shared handler flow (same path chat + config_prompt command use) ─────
    let service = Arc::new(service);
    let handler = SettingsHandler::new(service.clone());
    // GREEN theme change auto-applies.
    let outcome = handler
        .handle(SettingsRequest::change(
            "ui",
            "theme",
            serde_json::json!("light"),
        ))
        .await;
    assert!(
        matches!(outcome, SettingsOutcome::Applied { .. }),
        "GREEN theme change auto-applies, got {outcome:?}"
    );
    assert_eq!(service.get().await.ui.theme, "light");
    // ...and it persisted to the DB (field-level row updated).
    let rows = store.all().unwrap();
    assert!(rows
        .iter()
        .any(|r| r.section == "ui" && r.key == "theme" && r.value_json == "\"light\""));
    println!("[e2e] prompt 'change theme to light' → Applied + persisted to DB");

    // ── Injection wall: a change from external content is REFUSED ──────────
    let refused = handler
        .handle(
            SettingsRequest::change("ui", "theme", serde_json::json!("dark"))
                .with_provenance(TriggerProvenance::ExternalContent),
        )
        .await;
    assert!(
        matches!(refused, SettingsOutcome::Refused { .. }),
        "external-content trigger must be refused (injection wall), got {refused:?}"
    );
    assert_eq!(
        service.get().await.ui.theme,
        "light",
        "no change from injection"
    );
    println!("[e2e] injection wall OK: external 'change theme to dark' refused");

    // ── Risk gating: a YELLOW field needs approval (does not auto-apply) ───
    let approval = handler
        .handle(SettingsRequest::change(
            "agent",
            "autonomy_profile",
            serde_json::json!("aggressive"),
        ))
        .await;
    assert!(matches!(approval, SettingsOutcome::NeedsApproval { .. }));
    assert_ne!(service.get().await.agent.autonomy_profile, "aggressive");
    println!("[e2e] YELLOW field → NeedsApproval (not auto-applied)");

    // ── Undo the theme change → back to dark ───────────────────────────────
    let undone = service.undo_last().await;
    assert_eq!(undone, Some(("ui".to_string(), "theme".to_string())));
    assert_eq!(
        service.get().await.ui.theme,
        "dark",
        "undo restored prior value"
    );
    println!("[e2e] undo OK: theme restored to 'dark'");

    // ── Env-lock: a field pinned by a KRIA_* var is refused ────────────────
    std::env::set_var("KRIA_GPU_AUTOSCALE", "1");
    let locked = handler
        .handle(SettingsRequest::change(
            "orchestrator",
            "gpu_autoscale",
            serde_json::json!(true),
        ))
        .await;
    std::env::remove_var("KRIA_GPU_AUTOSCALE");
    match &locked {
        SettingsOutcome::Refused { reason } => {
            assert!(
                reason.contains("locked by environment"),
                "expected env-lock refusal, got: {reason}"
            );
        }
        other => panic!("expected env-lock Refused, got {other:?}"),
    }
    println!("[e2e] env-lock OK: gpu_autoscale refused while KRIA_GPU_AUTOSCALE set");

    // ── get_settings-shape redaction: secret never leaves in the JSON ──────
    let mut redacted = service.get().await;
    redacted.redact_secrets();
    let json = serde_json::to_string(&redacted).unwrap();
    assert!(
        !json.contains("sk-legacy-secret"),
        "redacted settings JSON must not contain the secret"
    );
    println!("[e2e] redaction OK: secret absent from settings JSON");

    // cleanup
    drop(service);
    let _ = std::fs::remove_dir_all(&tmp);
    println!("[e2e] ALL CHECKS PASSED");

    let _ = ChangeSource::Ui; // silence unused import if refactored
}
