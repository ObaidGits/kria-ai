// crates/kria-core/src/agent/ml_orchestrator/mod.rs
//
// Colab ML Orchestrator — Distributed Remote Job Orchestrator
//
// Transforms KRIA into an autonomous Data Science & ML coding agent.
// The Cloud LLM generates TOML-frontmattered Python code, the Rust
// orchestrator wraps it with safety guarantees, and Colab executes it.

pub mod types;
pub mod plan_parser;
pub mod code_gate;
pub mod helpers_template;
pub mod async_wrapper;
pub mod integrity;
pub mod ledger;
pub mod poller;
pub mod sync_cell;

// Re-exports for convenience
pub use types::{
    CellPlan, ParsedCell, Phase, PhaseArtifact, ArtifactType, ContentHash,
    HashAlgorithm, TrainingMetrics, MlPollerExit, PollResult,
    RetrievedArtifact, PhaseStatus, PhaseRecord,
};
pub use plan_parser::{parse_cell_plan, PlanParseError};
pub use code_gate::{capability_check, CapabilityError};
pub use ledger::{LedgerHandle, LedgerActor, LedgerMsg};
pub use poller::{AdaptiveMlPoller, ColabExecutor, PollEvent};
pub use integrity::{sha256_file, xxhash64_file, compute_hash, verify_hash};
pub use sync_cell::{generate_sync_cell, generate_checkpoint_sync};
pub use helpers_template::render_helpers;
pub use async_wrapper::{wrap_async_cell, wrap_sync_cell};

use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Configuration for the ML Orchestrator.
#[derive(Debug, Clone)]
pub struct MlOrchestratorConfig {
    pub enabled: bool,
    pub poll_interval_secs: u64,
    pub max_training_duration_secs: u64,
    pub heartbeat_floor_secs: u64,
    pub heartbeat_ceiling_secs: u64,
    pub checkpoint_interval_secs: u64,
    pub artifact_output_dir: String,
    pub max_regen_attempts: u32,
    pub capability_check_enabled: bool,
    pub fuse_retry_max: u32,
    pub fuse_retry_initial_delay_secs: u64,
}

impl Default for MlOrchestratorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            poll_interval_secs: 15,
            max_training_duration_secs: 3600,
            heartbeat_floor_secs: 60,
            heartbeat_ceiling_secs: 600,
            checkpoint_interval_secs: 300,
            artifact_output_dir: "/home/obaid/kria_artifacts".into(),
            max_regen_attempts: 2,
            capability_check_enabled: true,
            fuse_retry_max: 3,
            fuse_retry_initial_delay_secs: 1,
        }
    }
}

/// Top-level orchestrator for ML pipelines.
pub struct MlOrchestrator {
    config: MlOrchestratorConfig,
    cancel: CancellationToken,
}

impl MlOrchestrator {
    pub fn new(config: MlOrchestratorConfig) -> Self {
        Self {
            config,
            cancel: CancellationToken::new(),
        }
    }

    /// Create an adaptive poller from the config.
    pub fn create_poller(&self) -> AdaptiveMlPoller {
        AdaptiveMlPoller {
            poll_interval: Duration::from_secs(self.config.poll_interval_secs),
            max_training_duration: Duration::from_secs(self.config.max_training_duration_secs),
            heartbeat_floor: Duration::from_secs(self.config.heartbeat_floor_secs),
            heartbeat_ceiling: Duration::from_secs(self.config.heartbeat_ceiling_secs),
            cancel: self.cancel.clone(),
        }
    }

    /// Cancel the current pipeline.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Check if the pipeline has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Generate the complete wrapped code for a cell.
    /// This is the main entry point for cell preparation.
    pub fn prepare_cell(
        &self,
        cell: &ParsedCell,
        job_id: &str,
        hot_root: &str,
        cold_root: &str,
        dataset_path: &str,
    ) -> String {
        let status_file = format!(
            "{}/{}/training_status.json",
            hot_root,
            cell.phase_dir()
        );

        if cell.is_async {
            wrap_async_cell(cell, job_id, hot_root, cold_root, dataset_path, &status_file)
        } else {
            wrap_sync_cell(cell, job_id, hot_root, cold_root, dataset_path, &status_file)
        }
    }

    /// Validate a cell's code against the capability allowlist.
    pub fn validate_cell(&self, cell: &ParsedCell) -> Result<(), CapabilityError> {
        if self.config.capability_check_enabled {
            capability_check(&cell.code)
        } else {
            Ok(())
        }
    }

    /// Parse and validate a complete cell plan from the LLM's TOML response.
    pub fn parse_plan(&self, raw: &str, job_id: &str) -> Result<CellPlan, PlanParseError> {
        parse_cell_plan(raw, job_id)
    }

    /// Generate the sync cell for a completed phase.
    pub fn generate_phase_sync(
        &self,
        job_id: &str,
        cell: &ParsedCell,
        hot_root: &str,
        cold_root: &str,
    ) -> String {
        let artifact_names: Vec<&str> = cell.outputs.iter()
            .map(|o| o.split('/').last().unwrap_or(o))
            .collect();
        generate_sync_cell(job_id, &cell.phase_dir(), hot_root, cold_root, &artifact_names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TOML: &str = r#"
[plan]
name = "test_plan"
task = "classification"
gpu_required = true
estimated_duration_minutes = 10

[[cells]]
id = "setup"
phase = "setup"
description = "Install deps"
inputs = []
outputs = ["01_setup/packages.json"]
timeout = 120
retry = true
async_cell = false
code = '''
print("hello")
'''

[[cells]]
id = "train"
phase = "training"
description = "Train model"
inputs = ["01_setup/packages.json"]
outputs = ["04_train/model.pth"]
timeout = 10
retry = false
async_cell = true
code = '''
import torch
job_progress.report(progress=0.5, metrics={"loss": 0.3})
job_paths.safe_save_model(model, "04_train/model.pth")
job_progress.complete()
'''
"#;

    #[test]
    fn full_pipeline_parse_validate_prepare() {
        let orch = MlOrchestrator::new(MlOrchestratorConfig::default());

        // Step 1: Parse
        let plan = orch.parse_plan(VALID_TOML, "job_123").unwrap();
        assert_eq!(plan.cells.len(), 2);

        // Step 2: Validate each cell
        for cell in &plan.cells {
            orch.validate_cell(cell).unwrap();
        }

        // Step 3: Prepare sync cell
        let sync = orch.generate_phase_sync("job_123", &plan.cells[0], "/hot", "/cold");
        assert!(sync.contains("packages.json"));
        assert!(sync.contains("manifest.json"));
    }

    #[test]
    fn prepare_sync_cell() {
        let orch = MlOrchestrator::new(MlOrchestratorConfig::default());
        let plan = orch.parse_plan(VALID_TOML, "j1").unwrap();
        let code = orch.prepare_cell(&plan.cells[0], "j1", "/hot", "/cold", "/data.csv");
        // Sync cell should have helpers but NO subprocess
        assert!(code.contains("KRIA HELPERS"));
        assert!(!code.contains("subprocess"));
    }

    #[test]
    fn prepare_async_cell_has_subprocess() {
        let orch = MlOrchestrator::new(MlOrchestratorConfig::default());
        let plan = orch.parse_plan(VALID_TOML, "j1").unwrap();
        let code = orch.prepare_cell(&plan.cells[1], "j1", "/hot", "/cold", "/data.csv");
        // Async cell should have subprocess wrapper
        assert!(code.contains("subprocess.Popen"));
        assert!(code.contains("KRIA_PID"));
    }

    #[test]
    fn rejected_banned_code() {
        let toml = r#"
[plan]
name = "bad"
task = "test"

[[cells]]
id = "evil"
phase = "setup"
description = "Bad code"
inputs = []
outputs = []
code = '''
import os
os.system("rm -rf /")
'''
"#;
        let orch = MlOrchestrator::new(MlOrchestratorConfig::default());
        let plan = orch.parse_plan(toml, "j1").unwrap();
        assert!(orch.validate_cell(&plan.cells[0]).is_err());
    }

    #[test]
    fn create_poller_from_config() {
        let orch = MlOrchestrator::new(MlOrchestratorConfig::default());
        let poller = orch.create_poller();
        assert_eq!(poller.poll_interval, Duration::from_secs(15));
        assert_eq!(poller.heartbeat_floor, Duration::from_secs(60));
        assert_eq!(poller.heartbeat_ceiling, Duration::from_secs(600));
    }
}
