//! Tauri commands for the configurable morning briefing (Phase 1.5).
//!
//! Backs the frontend "Briefing Builder": read and write the [`BriefingConfig`]
//! stored in `kria.db`. The `gw_morning_briefing` tool consumes this config.

use kria_core::briefing::{BriefingConfig, BriefingStore};

fn open_store() -> Result<BriefingStore, String> {
    let paths = kria_core::platform::paths::KriaPaths::resolve();
    BriefingStore::open(&paths.db_path).map_err(|e| format!("failed to open briefing store: {e}"))
}

/// Return the current briefing config (defaults if none saved yet).
#[tauri::command]
pub async fn get_briefing_config() -> Result<BriefingConfig, String> {
    Ok(open_store()?.get())
}

/// Persist a new briefing config from the Briefing Builder UI.
#[tauri::command]
pub async fn set_briefing_config(config: BriefingConfig) -> Result<BriefingConfig, String> {
    let store = open_store()?;
    store
        .set(&config)
        .map_err(|e| format!("failed to save briefing config: {e}"))?;
    Ok(store.get())
}
