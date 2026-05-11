// crates/kria-core/src/agent/ml_orchestrator/types.rs
//
// Core types for the Colab ML Orchestrator.

use serde::{Deserialize, Serialize};

/// The complete execution plan parsed from the Cloud LLM's TOML response.
#[derive(Debug, Clone)]
pub struct CellPlan {
    pub plan_name: String,
    pub task_type: String,
    pub cells: Vec<ParsedCell>,
    pub estimated_duration_minutes: u32,
    pub requires_gpu: bool,
}

/// A single executable cell after parsing and validation.
#[derive(Debug, Clone)]
pub struct ParsedCell {
    pub cell_id: String,
    pub phase: Phase,
    pub description: String,
    pub code: String,
    pub timeout_secs: u64,
    pub retry_on_failure: bool,
    pub is_async: bool,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

impl ParsedCell {
    /// Return the phase directory name (e.g., "01_setup", "04_train").
    pub fn phase_dir(&self) -> String {
        let idx = self.phase.ordinal();
        let name = self.phase.dir_name();
        format!("{:02}_{}", idx, name)
    }
}

/// ML pipeline phases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    DatasetUpload,
    Setup,
    DataLoading,
    Preprocessing,
    Training,
    Evaluation,
    ArtifactExport,
}

impl Phase {
    pub fn ordinal(&self) -> u8 {
        match self {
            Self::DatasetUpload => 0,
            Self::Setup => 1,
            Self::DataLoading => 2,
            Self::Preprocessing => 3,
            Self::Training => 4,
            Self::Evaluation => 5,
            Self::ArtifactExport => 6,
        }
    }

    pub fn dir_name(&self) -> &'static str {
        match self {
            Self::DatasetUpload => "upload",
            Self::Setup => "setup",
            Self::DataLoading => "load_data",
            Self::Preprocessing => "preprocess",
            Self::Training => "train",
            Self::Evaluation => "evaluate",
            Self::ArtifactExport => "export",
        }
    }
}

/// A file artifact that flows between phases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseArtifact {
    pub name: String,
    pub hot_path: String,
    pub cold_path: String,
    pub artifact_type: ArtifactType,
    pub size_bytes: Option<u64>,
    pub hash: Option<ContentHash>,
}

/// Hash for integrity verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContentHash {
    /// SHA-256 hex digest — model weights only.
    Sha256(String),
    /// xxhash64 — datasets and large files.
    Xxhash64(u64),
}

/// Artifact type determines hash algorithm and handling.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ArtifactType {
    TabularData,
    ModelWeights,
    MetricsJson,
    PlotImage,
    LogFile,
    Custom(String),
}

impl ArtifactType {
    /// Infer artifact type from file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "parquet" | "csv" | "tsv" => Self::TabularData,
            "pth" | "pt" | "safetensors" | "onnx" | "bin" => Self::ModelWeights,
            "json" => Self::MetricsJson,
            "png" | "svg" | "jpg" | "jpeg" => Self::PlotImage,
            "log" | "txt" => Self::LogFile,
            other => Self::Custom(other.to_string()),
        }
    }

    /// Which hash algorithm to use for this artifact type.
    pub fn hash_algorithm(&self) -> Option<HashAlgorithm> {
        match self {
            Self::ModelWeights => Some(HashAlgorithm::Sha256),
            Self::TabularData => Some(HashAlgorithm::Xxhash64),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum HashAlgorithm {
    Sha256,
    Xxhash64,
}

/// Status of a phase in the JobLedger.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PhaseStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

/// Record stored in the Ledger for each phase execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseRecord {
    pub job_id: String,
    pub cell_id: String,
    pub phase: Phase,
    pub status: PhaseStatus,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub outputs: Vec<PhaseArtifact>,
    pub error: Option<String>,
    pub worker_pid: Option<u32>,
    pub attempt: u32,
    pub last_heartbeat_ts: Option<f64>,
}

/// Training metrics reported by the worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingMetrics {
    pub state: String,
    pub progress: f64,
    #[serde(default)]
    pub metrics: serde_json::Value,
    pub error: Option<String>,
    pub pid: Option<u32>,
    pub heartbeat_ts: Option<f64>,
    pub timestamp: f64,
    /// p95 of recent batch latencies — reported by the worker.
    pub batch_latencies_p95: Option<f64>,
}

/// Exit condition for the adaptive poller.
#[derive(Debug)]
pub enum MlPollerExit {
    Completed(TrainingMetrics),
    Failed(String),
    Timeout,
    Cancelled,
}

/// Result of a single poll cycle.
#[derive(Debug)]
pub enum PollResult {
    StillRunning(TrainingMetrics),
    Completed(TrainingMetrics),
    Failed(String),
    ProcessCrashed { pid: u32 },
    ProcessHung { pid: u32, threshold_secs: f64, last_heartbeat_age_secs: f64 },
    NoStatusFile,
    Cancelled,
}

/// Artifact retrieved from Drive to local filesystem.
#[derive(Debug, Clone)]
pub struct RetrievedArtifact {
    pub name: String,
    pub local_path: std::path::PathBuf,
    pub artifact_type: ArtifactType,
    pub size_bytes: u64,
}
