use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::infra::environment::{
    EnvironmentError, EnvironmentProvider, SharedShellState, ShellState,
};

pub mod app_lifecycle;
pub mod browser_agent;
pub mod communication;
pub mod desktop;
pub mod developer;
pub mod disk;
pub mod documents;
pub mod dynamic_gen;
pub mod exec;
pub mod file_ops;
pub mod gui_automation;
pub mod google_workspace;
pub mod google_workspace_contract;
pub mod i18n;
pub mod image_generation;
pub mod interaction;
pub mod internet;
pub mod knowledge;
pub mod mount_manager;
pub mod news;
pub mod packages;
pub mod power;
pub mod precognitive;
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
pub mod vision;
pub mod vision_automation;

#[derive(Clone)]
pub struct ToolContext {
    pub env: Arc<dyn EnvironmentProvider>,
    pub shell_state: SharedShellState,
    pub cancellation: CancellationToken,
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
