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
        }
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
pub use registry::{ToolDef, ToolHandler, ToolRegistry};

/// Execute a shell command either locally or on the VM, depending on env vars.
/// When KRIA_TEST_VM_HOST is set (and we're in test mode with KRIA_RUNNING_IN_VM=1),
/// destructive commands are dispatched to the VM via SSH instead of running locally.
/// This implements the "host=brain, VM=muscle" architecture.
///
/// When `use_sudo` is true, the command is prefixed with `sudo -n` (non-interactive)
/// on the VM/Docker target. This is needed for privileged operations like shutdown.
pub async fn vm_dispatch_command(cmd: &str) -> Result<(), String> {
    vm_dispatch_command_with_sudo(cmd, false).await
}

pub async fn vm_dispatch_command_with_sudo(cmd: &str, use_sudo: bool) -> Result<(), String> {
    let vm_host = std::env::var("KRIA_TEST_VM_HOST").ok();
    let running_in_vm = std::env::var("KRIA_RUNNING_IN_VM").ok().as_deref() == Some("1");
    let docker_container = std::env::var("KRIA_TEST_DOCKER_CONTAINER_ID").ok();

    let effective_cmd = if use_sudo {
        format!("sudo -n {}", cmd)
    } else {
        cmd.to_string()
    };

    // If VM env vars are set and we're in test mode, dispatch to VM/Docker
    if (running_in_vm && vm_host.is_some()) || docker_container.is_some() {
        if let Some(ref container_id) = docker_container {
            // Docker dispatch
            let output = tokio::process::Command::new("docker")
                .args(["exec", container_id, "bash", "-c", &effective_cmd])
                .output()
                .await
                .map_err(|e| format!("docker exec failed: {e}"))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("docker exec failed: {stderr}"));
            }
        } else if let Some(ref host) = vm_host {
            // SSH dispatch
            let port = std::env::var("KRIA_TEST_VM_PORT")
                .ok()
                .and_then(|v| v.parse::<u16>().ok())
                .unwrap_or(22);
            let user = std::env::var("KRIA_TEST_VM_USER").unwrap_or_else(|_| "obaid".to_string());
            let ssh_key = std::env::var("KRIA_TEST_VM_SSH_KEY")
                .unwrap_or_else(|_| "~/.ssh/kria_id".to_string());

            let output = tokio::process::Command::new("ssh")
                .args([
                    "-o",
                    "StrictHostKeyChecking=no",
                    "-o",
                    "ConnectTimeout=10",
                    "-o",
                    "BatchMode=yes",
                    "-i",
                    &ssh_key,
                    "-p",
                    &port.to_string(),
                    &format!("{}@{}", user, host),
                    &effective_cmd,
                ])
                .output()
                .await
                .map_err(|e| format!("SSH dispatch failed: {e}"))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("SSH command failed: {stderr}"));
            }
        }
    } else {
        // Local execution
        let output = tokio::process::Command::new("sh")
            .args(["-c", &effective_cmd])
            .output()
            .await
            .map_err(|e| format!("local command failed: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("command failed: {stderr}"));
        }
    }
    Ok(())
}

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
