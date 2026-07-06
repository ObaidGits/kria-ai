//! Hot-reload activation adapter (router-contract §5 / A2.5).
//!
//! The concrete `SkillActivation` the host wires into the `BundleInstaller`.
//!
//! REAL BUG FOUND + FIXED (task 5, R11/R6/R3 validation): this previously called the
//! legacy per-skill `register_skill` on install/update, expecting it to register an
//! individual `oc_<slug>` tool into the `ToolRegistry`. But `register_skill` was
//! ALREADY fully disabled under A6 (`handler.rs`: its `tool_registry.register(...)`
//! call is commented out and it unconditionally `return`s `false`) — so `activate()`
//! ALWAYS returned `Err(...)`, which `BundleInstaller::install`/`update` treats as a
//! fatal activation failure and ROLLS BACK the entire install
//! (`installer.rs`: `if let Err(e) = act.activate(&descriptor) { self.rollback(...);
//! return Err(InstallError::RolledBack(...)) }`).
//!
//! **CONFIRMED, NOT HYPOTHETICAL**: reverted this file to its original content and ran
//! the pre-existing `real_activation_makes_tool_callable_then_removes_it` test
//! (`kria-core/tests/openclaw_bundle_tests.rs`) — it FAILED with exactly the predicted
//! error: `RolledBack("activation: no runtime backend available for skill 'oc_test'")`.
//! Net effect in production: installing a skill bundle through either desktop command
//! that wires `ToolRegistryActivation` (`install_skill_bundle`, `uninstall_skill_bundle`
//! in `kria-desktop/commands/openclaw.rs`) ALWAYS FAILED whenever OpenClaw was actually
//! enabled and the container pool was up — i.e. always failed under the exact condition
//! a user would try to install a skill.
//!
//! Under A6 semantic routing this per-skill tool registration is not just broken, it is
//! ARCHITECTURALLY UNNECESSARY: `SemanticSkillRouter::route` calls
//! `registry.get_enabled_skills()` fresh on every routing decision (no caching), so a
//! newly-installed/enabled skill is immediately discoverable through the single
//! `"openclaw"` tool (registered once at boot via `register_semantic_openclaw`) with no
//! per-skill registration needed. `activate()` therefore now succeeds unconditionally
//! (registry-driven discovery already handles visibility) and simply triggers the
//! semantic tool-index reindex callback — matching what A6 actually requires, instead
//! of resurrecting the deprecated A5-era per-skill path. `deactivate()` is likewise a
//! no-op for the same reason (disabling a skill via `set_skill_state` already removes
//! it from `get_enabled_skills()`).

use crate::openclaw::bundle::SkillActivation;
use crate::openclaw::types::SkillDescriptor;
use std::sync::Arc;

type ReindexFn = Arc<dyn Fn() + Send + Sync>;

/// Activation sink for the A6 semantic-routing model: install/enable/disable are fully
/// registry-driven (`SemanticSkillRouter` reads `get_enabled_skills()` fresh every route),
/// so this adapter's only real job is triggering the caller-supplied semantic tool-index
/// rebuild callback. It intentionally holds NO `ToolRegistry`/`RuntimeRegistry` — the A5-era
/// per-skill tool registration those enabled is deprecated/disabled (see module doc).
#[derive(Default)]
pub struct ToolRegistryActivation {
    reindex: Option<ReindexFn>,
}

impl ToolRegistryActivation {
    pub fn new() -> Self {
        Self::default()
    }

    /// Supply the semantic tool-index rebuild callback (desktop/server wires `tool_index.rebuild`).
    pub fn with_reindex(mut self, reindex: ReindexFn) -> Self {
        self.reindex = Some(reindex);
        self
    }
}

impl SkillActivation for ToolRegistryActivation {
    /// Always succeeds: under A6, a skill becomes routable the instant it is
    /// `Enabled` in `ProductionSkillRegistry` — no per-skill tool registration
    /// step exists or is needed. Triggers reindex so any semantic tool-index
    /// cache (if the caller maintains one) picks up the new skill immediately.
    fn activate(&self, skill: &SkillDescriptor) -> Result<(), String> {
        tracing::info!(
            skill_id = %skill.skill_id,
            "[OpenClaw A6] skill activated (registry-driven — no per-skill tool registration needed)"
        );
        self.reindex();
        Ok(())
    }

    /// Always succeeds: disabling a skill via `set_skill_state` already removes
    /// it from `get_enabled_skills()`, which the semantic router reads fresh.
    fn deactivate(&self, skill_id: &str) -> Result<(), String> {
        tracing::info!(skill_id = %skill_id, "[OpenClaw A6] skill deactivated (registry-driven)");
        self.reindex();
        Ok(())
    }

    fn reindex(&self) {
        if let Some(cb) = &self.reindex {
            cb();
        }
    }
}
