//! R6 — skill management: update/enable/disable/uninstall/hot-reload
//! (tasks.md task 12).
//!
//! Real-code grounding (re-confirmed from tasks 2/5/8, not re-derived):
//! - `BundleInstaller::enable/disable` call `registry.toggle()` (→
//!   `set_skill_state(Enabled/Disabled)`) THEN `activation.activate()`/
//!   `deactivate()` (fixed in task 5 to always succeed, registry-driven).
//! - `SemanticSkillRouter::route` reads `get_enabled_skills()` FRESH every
//!   call — so a TOGGLE (enable/disable) takes effect on the VERY NEXT
//!   routing decision, no restart, no cache invalidation needed.
//!
//! FIXED (product gap 4/8, post user sign-off): `BundleInstaller::install_inner`
//! previously left every fresh install in `SkillState::Installed` — NOT
//! `Enabled` — with nothing transitioning it further, so a freshly-installed
//! skill was NOT routable until a SEPARATE `enable()` call (confirmed by
//! direct reproduction: `get_enabled_skills()` was empty and
//! `SemanticSkillRouter::route` returned "No enabled skills found in
//! registry" immediately after a real, successful, signature-verified
//! install). Real fix, additive: `install_inner` now calls
//! `registry.set_skill_state(&slug, Enabled)` right after the registry write
//! for a `VersionRelation::Fresh` install (no second step needed) — and for
//! an `Upgrade`/`Same` relation, PRESERVES whatever the skill's prior
//! enabled/disabled state was (never silently re-enabling a skill the user
//! explicitly disabled before upgrading it).
//!
//! What DOES genuinely work, confirmed by this module (both before and after
//! the fix): hot enable/disable toggling with the SAME registry/router
//! instances (no restart) — R6.4 holds for the toggle itself.
//! `BundleInstaller::uninstall` removes the registry row (task 5's `get()`
//! fix makes `Removed` state genuinely not-found) and deletes the versioned
//! store directory — real orphan-free removal.

use kria_core::openclaw::bundle::BundleInstaller;
use kria_core::openclaw::registry::ProductionSkillRegistry;
use semver::Version;
use std::sync::Arc;

/// True when `skill_id` is currently in the registry's enabled set — the SAME
/// source capability discovery (CPP federated index / prior router) reads fresh
/// on every decision, so this is the real "is it discoverable right now" check
/// without depending on the deleted `SemanticSkillRouter`.
fn is_enabled(registry: &ProductionSkillRegistry, skill_id: &str) -> Result<bool, String> {
    Ok(registry
        .get_enabled_skills()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|s| s.skill_id == skill_id))
}

/// R6.1/R6.4: real hot enable/disable. Installs a real signed bundle, asserts
/// it is immediately in the enabled set, disables it and asserts it drops out
/// with the SAME registry instance (no restart), re-enables and asserts it
/// returns — proving genuine hot toggling of discoverability.
pub async fn validate_hot_enable_disable() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let db_path = dir.path().join("r6_hotswap.db");
    let registry = Arc::new(ProductionSkillRegistry::new(&db_path).map_err(|e| e.to_string())?);
    let audit = Arc::new(
        kria_core::openclaw::audit::AuditLedger::open(&db_path, b"r6-test-key".to_vec())
            .map_err(|e| e.to_string())?,
    );
    let store = dir.path().join("store");
    std::fs::create_dir_all(&store).map_err(|e| e.to_string())?;
    let author_dir = dir.path().join("authored");
    std::fs::create_dir_all(&author_dir).map_err(|e| e.to_string())?;

    let bundle_root = crate::openclaw_eval::installer_matrix::author_signed_bundle(
        &author_dir,
        "oc_r6_hotswap",
        [9u8; 32],
    )?;

    let installer = BundleInstaller::new(registry.clone(), audit, store)
        .with_kria_version(Version::new(1, 0, 0))
        .with_trust_policy(kria_core::openclaw::bundle::verify::TrustPolicy {
            trusted_keys: Vec::new(),
            require_signature: true,
        });
    installer
        .install(&bundle_root)
        .map_err(|e| format!("install failed: {e}"))?;

    // FIX PROOF (product gap 4/8): a fresh bundle install must now land
    // directly in `Enabled` state — NO separate `enable()` call needed.
    // `get_enabled_skills()` (what `SemanticSkillRouter::route` reads) must
    // be non-empty immediately after `install()` returns.
    let enabled_after_install = registry.get_enabled_skills().map_err(|e| e.to_string())?;
    if enabled_after_install.is_empty() {
        return Err(
            "REGRESSION: a fresh bundle install must land directly in Enabled state — \
             the auto-enable-on-install fix appears to have regressed"
                .into(),
        );
    }

    // The SAME registry instance across all remaining checks — no restart, no
    // new process — proving genuine hot behavior, not "works after reboot".
    if !is_enabled(&registry, "oc_r6_hotswap")? {
        return Err(
            "REGRESSION: fresh install must be discoverable WITHOUT a separate enable() call"
                .into(),
        );
    }

    installer
        .disable("oc_r6_hotswap")
        .map_err(|e| format!("disable failed: {e}"))?;
    if is_enabled(&registry, "oc_r6_hotswap")? {
        return Err("R6.1/R6.4 VIOLATION: disabled skill still in the enabled set, NO restart between disable and check".into());
    }

    installer
        .enable("oc_r6_hotswap")
        .map_err(|e| format!("enable failed: {e}"))?;
    if !is_enabled(&registry, "oc_r6_hotswap")? {
        return Err(
            "R6.1/R6.4: re-enabled skill did not resume discoverability, with NO restart".into(),
        );
    }

    Ok(())
}

/// R6.2/R6.5: uninstall removes the registry row AND the store directory —
/// no orphaned files, no orphaned registry entry. Uses task 5's fixed
/// `get()` (treats Removed as NotFound) as the real post-condition check.
pub fn validate_uninstall_leaves_no_orphans() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let db_path = dir.path().join("r6_uninstall.db");
    let registry = Arc::new(ProductionSkillRegistry::new(&db_path).map_err(|e| e.to_string())?);
    let audit = Arc::new(
        kria_core::openclaw::audit::AuditLedger::open(&db_path, b"r6-test-key-2".to_vec())
            .map_err(|e| e.to_string())?,
    );
    let store = dir.path().join("store");
    std::fs::create_dir_all(&store).map_err(|e| e.to_string())?;
    let author_dir = dir.path().join("authored");
    std::fs::create_dir_all(&author_dir).map_err(|e| e.to_string())?;

    let bundle_root = crate::openclaw_eval::installer_matrix::author_signed_bundle(
        &author_dir,
        "oc_r6_uninstall",
        [11u8; 32],
    )?;

    let installer = BundleInstaller::new(registry.clone(), audit, store.clone())
        .with_kria_version(Version::new(1, 0, 0))
        .with_trust_policy(kria_core::openclaw::bundle::verify::TrustPolicy {
            trusted_keys: Vec::new(),
            require_signature: true,
        });
    installer
        .install(&bundle_root)
        .map_err(|e| format!("install failed: {e}"))?;

    if !store.join("oc_r6_uninstall").exists() {
        return Err("expected store dir to exist after install".into());
    }

    installer
        .uninstall("oc_r6_uninstall")
        .map_err(|e| format!("uninstall failed: {e}"))?;

    if registry.get("oc_r6_uninstall").is_ok() {
        return Err("R6.2/R6.5: registry still returns Ok(..) for an uninstalled skill (orphaned registry entry)".into());
    }
    if store.join("oc_r6_uninstall").exists() {
        return Err(
            "R6.2/R6.5: store directory still exists after uninstall (orphaned files)".into(),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proves the FIX: fresh install lands directly `Enabled` (no separate
    /// `enable()` step) AND that toggling remains genuinely hot with no
    /// restart.
    #[tokio::test]
    async fn r6_1_4_fresh_install_auto_enabled_then_hot_toggle_works() {
        validate_hot_enable_disable()
            .await
            .expect("R6.1/R6.4: fresh install must auto-enable, then toggle hot with no restart");
    }

    #[test]
    fn r6_2_5_uninstall_leaves_no_orphans() {
        validate_uninstall_leaves_no_orphans()
            .expect("R6.2/R6.5: uninstall must leave no orphaned registry row or files");
    }

    /// FIX PROOF: `BundleInstaller::install_inner` must contain the real
    /// auto-enable transition for a fresh install. Source-text tripwire
    /// (cheap, catches accidental removal) — the real behavioral proof is
    /// `r6_1_4_fresh_install_auto_enabled_then_hot_toggle_works` above.
    #[test]
    fn fixed_installer_auto_enables_fresh_installs() {
        let installer_rs = include_str!("../../../kria-core/src/openclaw/bundle/installer.rs");
        let install_inner_section = installer_rs
            .split("fn install_inner(")
            .nth(1)
            .and_then(|s| s.split("pub fn").next())
            .unwrap_or_default();
        let auto_enables =
            install_inner_section.contains("VersionRelation::Fresh => Some(SkillState::Enabled)");
        assert!(
            auto_enables,
            "REGRESSION: install_inner must auto-enable a Fresh install — if this fails, \
             the auto-enable-on-install fix has been removed or changed shape"
        );
    }
}
