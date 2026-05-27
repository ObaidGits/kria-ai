use super::{FactSource, WorldModelStore};
use anyhow::Result;

/// Provides a semantic graph representation of the desktop environment.
/// Maps concepts like applications, windows, and workflows to the underlying
/// Bayesian fact store.
pub struct DesktopGraph<'a> {
    store: &'a WorldModelStore,
}

impl<'a> DesktopGraph<'a> {
    pub fn new(store: &'a WorldModelStore) -> Self {
        Self { store }
    }

    /// Register a new application entering the desktop state.
    pub fn register_app(&self, app_id: &str, app_name: &str) -> Result<()> {
        self.store.upsert(
            app_id,
            "is_a",
            "application",
            0.99,
            FactSource::Detected,
            "AT-SPI detection",
        )?;
        self.store.upsert(
            app_id,
            "has_name",
            app_name,
            0.99,
            FactSource::Detected,
            "AT-SPI detection",
        )?;
        Ok(())
    }

    /// Mark an application as the currently focused window.
    pub fn set_focused_app(&self, app_id: &str) -> Result<()> {
        self.store.upsert(
            "desktop_environment",
            "focused_app",
            app_id,
            0.95,
            FactSource::Detected,
            "AT-SPI focus event",
        )?;
        Ok(())
    }

    /// Link an active workflow to a specific application window.
    pub fn link_workflow_to_app(&self, workflow_id: &str, app_id: &str) -> Result<()> {
        self.store.upsert(
            workflow_id,
            "targets_app",
            app_id,
            0.90,
            FactSource::Inferred,
            "Workflow substrate planner",
        )?;
        Ok(())
    }

    /// Register a browser SPA transition or URL change.
    pub fn register_browser_navigation(&self, app_id: &str, url: &str, title: &str) -> Result<()> {
        self.store.upsert(
            app_id,
            "current_url",
            url,
            0.95,
            FactSource::Detected,
            "CDP navigation event",
        )?;
        self.store.upsert(
            app_id,
            "current_title",
            title,
            0.95,
            FactSource::Detected,
            "CDP navigation event",
        )?;
        Ok(())
    }

    /// Clear stale apps that are no longer present in the AT-SPI tree.
    ///
    /// Currently delegates to the WorldModel's decay_and_archive mechanism.
    /// TODO (Batch 2): Actively decay facts for apps not in `current_app_ids`.
    pub fn prune_stale_apps(&self, _current_app_ids: &[&str]) -> Result<()> {
        // Deeper pruning logic: look at all facts with predicate "is_a" object "application",
        // then verify if they are in current_app_ids. If not, decay them heavily.
        // For now, we rely on the WorldModel's general decay_and_archive mechanism.
        Ok(())
    }
}
