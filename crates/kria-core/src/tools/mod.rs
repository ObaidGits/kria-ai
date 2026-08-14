use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::infra::environment::{
    EnvironmentError, EnvironmentProvider, SharedShellState, ShellState,
};

pub mod app_lifecycle;
pub mod atspi_tools;
pub mod availability;
pub mod browser_agent;
/// CPP-backed capability dispatcher (Option-A migration) — the single chat/agent
/// execution entry point that routes through `CapabilityPlatform`.
pub mod capability_dispatch;
pub mod cognition_tools;
pub mod communication;
pub mod config_patch;
pub mod desktop;
/// Audio input, per-application stream, device-profile and MPRIS handlers.
pub mod audio;

/// VPN, hotspot, proxy and saved-credential handlers.
pub mod connectivity;

/// Trash restore, archive listing and ownership handlers.
pub mod file_control;

/// Battery health, logout and scheduled-shutdown handlers.
pub mod power_session;

/// Credential-store handlers: metadata listing, store, replace, delete.
pub mod secrets;

/// Scheduled-task patching and in-tree workflow handlers.
pub mod automation_control;

/// Printing, privacy controls and firewall handlers.
pub mod print_privacy_firewall;

/// Search, health, backup, scan, firmware and sensor handlers.
pub mod system_services;

pub mod desktop_state;
pub mod developer;
pub mod disk;
pub mod documents;
pub mod dynamic_gen;
pub mod exec;
pub mod feature_control;
pub mod file_ops;
pub mod google_workspace;
pub mod google_workspace_contract;
pub mod gui_automation;
pub mod i18n;
pub mod image_generation;
pub mod interaction;
pub mod internet;
pub mod knowledge;
pub mod mount_manager;
pub mod n8n;
pub mod news;
pub mod packages;
/// Bluetooth adapter and device lifecycle handlers (Task 3.7, OSC-021).
pub mod bluetooth;

/// Shared plumbing for canonical OS tool handlers: runtime resolution, governed
/// mutation/read drivers, and the single receipt rendering shape.
pub mod os_governed;
pub mod power;
pub mod precognitive;
pub mod preflight;
pub mod proactive;
pub mod process;
pub mod quarantine;
pub mod rag;
pub mod registry;
pub mod scheduler;
pub mod shell;
pub mod subprocess_executor;
pub mod system_config;
pub mod system_info;
pub mod tasks;
pub mod vision;
pub mod vision_automation;

/// Provenance of the content that triggered a tool call.
///
/// Used by the settings-config-revamp injection wall: privileged tools (e.g.
/// `config_patch`) MUST refuse to mutate state unless the trigger is
/// [`TriggerProvenance::User`]. Defaults to `User` for backward compatibility
/// with existing tool call sites; the agent/turn boundary sets the accurate
/// value (e.g. `ExternalContent` when a tool acts on fetched web/file content).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TriggerProvenance {
    /// The tool call originates directly from user input.
    #[default]
    User,
    /// The tool call originates from external content (web page, file, doc).
    ExternalContent,
    /// The tool call originates from another tool's output.
    Tool,
}

#[derive(Clone)]
pub struct ToolContext {
    pub env: Arc<dyn EnvironmentProvider>,
    pub shell_state: SharedShellState,
    pub cancellation: CancellationToken,
    /// Provenance of the triggering content (injection wall — see
    /// [`TriggerProvenance`]). Defaults to `User`.
    pub provenance: TriggerProvenance,
    /// Optional runtime adapter for prompt-accessible feature status/control.
    pub feature_control_backend: Option<Arc<dyn feature_control::FeatureControlBackend>>,
    /// Optional handle to the live `ConfigService` (settings-config-revamp).
    /// `None` for tool calls that don't need config access; the `config_patch`
    /// tool (Task 13) requires it. Defaults to `None`.
    pub config: Option<Arc<crate::config::ConfigService>>,
    /// Optional turn-scoped configuration overlay (settings-config-revamp Task 14).
    /// When present, `effective_config()` returns the live config with these
    /// whitelisted temp overrides applied on top. Dropped at turn end (never
    /// persisted) — so it auto-reverts on success, error, or crash.
    pub request_override: Option<Arc<crate::config::RequestOverride>>,
    /// The governed OS-control runtime seam (linux-os-control-production Task 1.2).
    /// Injected into every OS-facing handler so it reaches host effects only
    /// through governed runtime methods — never raw `HostOsControl` or
    /// `LocalEnvironment`. `None` only if a registry was built without the seam;
    /// the standard builders always inject at least a detached runtime.
    pub os_runtime: Option<Arc<crate::os_control::OsControlRuntime>>,
    /// The governed-call bundle for ONE admitted canonical OS action
    /// (linux-os-control-production Task 1.2/1.7): the grant, held write leases,
    /// durable audit admission, and observation context.
    ///
    /// Attached by the agent's policy executor **only** for native-OS actions, so
    /// non-OS tools never see admission material. Without it an OS handler can
    /// still observe availability but cannot mutate — `run_mutation` requires
    /// every artifact in here.
    pub os_call: Option<Arc<crate::os_control::governed::OsGovernedCall>>,
}

impl ToolContext {
    pub fn new(
        env: Arc<dyn EnvironmentProvider>,
        shell_state: SharedShellState,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            env,
            shell_state,
            cancellation,
            provenance: TriggerProvenance::User,
            feature_control_backend: None,
            config: None,
            request_override: None,
            os_runtime: None,
            os_call: None,
        }
    }

    /// Attach the governed-call bundle for one admitted native-OS action.
    ///
    /// Only the agent's policy executor calls this, after the policy gate minted
    /// the grant, the audit store admitted the action, and the write leases were
    /// acquired. A handler that finds it present may perform a governed mutation.
    #[must_use]
    pub fn with_os_call(
        mut self,
        call: Arc<crate::os_control::governed::OsGovernedCall>,
    ) -> Self {
        self.os_call = Some(call);
        self
    }

    /// The governed-call bundle for this action, if the action was admitted as a
    /// native-OS mutation.
    #[must_use]
    pub fn os_call(&self) -> Option<&crate::os_control::governed::OsGovernedCall> {
        self.os_call.as_deref()
    }

    /// Attach the governed OS-control runtime seam (builder-style, Task 1.2).
    /// OS-facing handlers read it via [`ToolContext::os_runtime`] and reach host
    /// effects only through it.
    pub fn with_os_runtime(mut self, runtime: Arc<crate::os_control::OsControlRuntime>) -> Self {
        self.os_runtime = Some(runtime);
        self
    }

    /// The injected OS-control runtime seam, if any.
    #[must_use]
    pub fn os_runtime(&self) -> Option<Arc<crate::os_control::OsControlRuntime>> {
        self.os_runtime.clone()
    }

    /// Set the trigger provenance (builder-style). The agent/turn boundary uses
    /// this to mark tool calls that act on external/tool-derived content.
    pub fn with_provenance(mut self, provenance: TriggerProvenance) -> Self {
        self.provenance = provenance;
        self
    }

    /// Attach a runtime feature-control backend (builder-style).
    pub fn with_feature_control_backend(
        mut self,
        backend: Arc<dyn feature_control::FeatureControlBackend>,
    ) -> Self {
        self.feature_control_backend = Some(backend);
        self
    }

    /// Attach a live `ConfigService` handle (builder-style) for tools that
    /// read or mutate configuration (e.g. `config_patch`).
    pub fn with_config(mut self, config: Arc<crate::config::ConfigService>) -> Self {
        self.config = Some(config);
        self
    }

    /// Attach a turn-scoped `RequestOverride` (builder-style, settings-config-revamp
    /// Task 14). Tools that support per-turn temporary settings read it via
    /// [`ToolContext::effective_config`].
    pub fn with_request_override(
        mut self,
        request_override: Arc<crate::config::RequestOverride>,
    ) -> Self {
        self.request_override = Some(request_override);
        self
    }

    /// Resolve the effective config for THIS turn: the live `ConfigService` value
    /// with any whitelisted turn-scoped [`RequestOverride`] applied on top. Returns
    /// `None` if no `ConfigService` handle is attached. The override is never
    /// persisted — it exists only for the lifetime of this context.
    pub async fn effective_config(&self) -> Option<crate::config::KriaConfig> {
        let svc = self.config.as_ref()?;
        let base = svc.get().await;
        match &self.request_override {
            Some(ov) if !ov.is_empty() => Some(ov.overlay(&base)),
            _ => Some(base),
        }
    }

    pub async fn snapshot_shell_state(&self) -> ShellState {
        self.shell_state.lock().await.clone()
    }

    pub async fn commit_shell_mutation<F>(
        &self,
        snapshot_generation: u64,
        mutate: F,
    ) -> Result<(), EnvironmentError>
    where
        F: FnOnce(&mut ShellState),
    {
        let mut state = self.shell_state.lock().await;
        let current_generation = state.generation;
        if current_generation != snapshot_generation {
            return Err(EnvironmentError::ShellStateConflict {
                expected_generation: snapshot_generation,
                actual_generation: current_generation,
            });
        }

        mutate(&mut state);
        state.generation = state.generation.saturating_add(1);
        Ok(())
    }
}

pub use mount_manager::ToolMountManager;
pub use registry::{
    OsUnavailableHandler, ToolDef, ToolHandler, ToolRegistrationError, ToolRegistry,
    ToolResumeCapability,
};

#[cfg(test)]
mod provenance_tests {
    use super::*;

    #[test]
    fn provenance_defaults_to_user() {
        // Backward-compat: a freshly built ToolContext (via any legacy path)
        // must default to `User` so existing tool call sites are unaffected.
        assert_eq!(TriggerProvenance::default(), TriggerProvenance::User);
    }

    #[test]
    fn with_provenance_threads_value() {
        let base = TriggerProvenance::default();
        assert_eq!(base, TriggerProvenance::User);
        // The builder must carry the accurate provenance for the injection wall.
        let external = TriggerProvenance::ExternalContent;
        assert_ne!(external, TriggerProvenance::User);
        let tool = TriggerProvenance::Tool;
        assert_ne!(tool, TriggerProvenance::User);
        assert_ne!(tool, external);
    }
}
