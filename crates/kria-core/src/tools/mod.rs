use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::infra::environment::{
	EnvironmentError, EnvironmentProvider, ShellState, SharedShellState,
};

pub mod app_lifecycle;
pub mod communication;
pub mod desktop;
pub mod developer;
pub mod disk;
pub mod documents;
pub mod exec;
pub mod file_ops;
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
pub mod rag;
pub mod registry;
pub mod scheduler;
pub mod shell;
pub mod system_config;
pub mod system_info;
pub mod vision;

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
