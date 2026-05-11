// crates/kria-core/tests/ml_orchestrator_e2e_tests.rs
//
// End-to-end tests for the Colab ML Orchestrator.
// Tests the full pipeline: TOML parse → AST gate → wrap → ledger → poller.

use kria_core::agent::ml_orchestrator::*;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// A mock Colab executor for testing the poller.
struct MockColabExecutor {
    responses: std::sync::Mutex<Vec<String>>,
}

impl MockColabExecutor {
    fn new(responses: Vec<String>) -> Self {
        Self { responses: std::sync::Mutex::new(responses) }
    }
}

#[async_trait::async_trait]
impl ColabExecutor for MockColabExecutor {
    async fn execute_cell(&self, _code: &str) -> anyhow::Result<String> {
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            Ok(r#"{"state":"no_file","progress":0.0,"metrics":{},"heartbeat_ts":null,"timestamp":0.0,"batch_latencies_p95":null}"#.into())
        } else {
            Ok(responses.remove(0))
        }
    }
}

// ─── Full TOML Plan ─────────────────────────────────────────────────────────

const FULL_ML_PLAN: &str = r#"
[plan]
name = "sentiment_bert_reviews"
task = "text_classification"
gpu_required = true
estimated_duration_minutes = 15

[[cells]]
id = "setup"
phase = "setup"
description = "Install transformers and check GPU"
inputs = []
outputs = ["01_setup/packages.json"]
timeout = 120
retry = true
async_cell = false
code = '''
!pip install -q transformers datasets scikit-learn accelerate

import torch, json

device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
gpu_name = torch.cuda.get_device_name(0) if torch.cuda.is_available() else "CPU"

out_dir = job_paths.makedirs("setup")
with open(f"{out_dir}/packages.json", "w") as f:
    json.dump({"device": str(device), "gpu": gpu_name, "torch": torch.__version__}, f)

print(f"✓ Device: {device} ({gpu_name})")
'''

[[cells]]
id = "load_data"
phase = "data_loading"
description = "Load dataset and inspect"
inputs = ["DATASET"]
outputs = ["02_load_data/data.parquet", "02_load_data/schema.json"]
timeout = 60
retry = false
async_cell = false
code = '''
import pandas as pd, json

df = pd.read_csv(job_paths.dataset_path())
print(f"✓ Shape: {df.shape}")
print(f"Columns: {list(df.columns)}")

out_dir = job_paths.makedirs("load_data")
df.to_parquet(f"{out_dir}/data.parquet", index=False)
with open(f"{out_dir}/schema.json", "w") as f:
    json.dump({"columns": list(df.columns), "rows": len(df)}, f)
print(f"✓ Data saved")
'''

[[cells]]
id = "preprocess"
phase = "preprocessing"
description = "Clean and split data"
inputs = ["02_load_data/data.parquet"]
outputs = ["03_preprocess/train.parquet", "03_preprocess/val.parquet"]
timeout = 60
retry = false
async_cell = false
code = '''
import pandas as pd
from sklearn.model_selection import train_test_split

df = pd.read_parquet(job_paths.input("load_data", "data.parquet"))
train_df, val_df = train_test_split(df, test_size=0.2, random_state=42)

out_dir = job_paths.makedirs("preprocess")
train_df.to_parquet(f"{out_dir}/train.parquet", index=False)
val_df.to_parquet(f"{out_dir}/val.parquet", index=False)
print(f"✓ Train: {len(train_df)}, Val: {len(val_df)}")
'''

[[cells]]
id = "train"
phase = "training"
description = "Fine-tune BERT with adaptive heartbeat"
inputs = ["03_preprocess/train.parquet"]
outputs = ["04_train/model.pth"]
timeout = 10
retry = false
async_cell = true
code = '''
import torch, pandas as pd
from transformers import BertTokenizer, BertForSequenceClassification, AdamW
from torch.utils.data import DataLoader, Dataset

df = pd.read_parquet(job_paths.input("preprocess", "train.parquet"))
tokenizer = BertTokenizer.from_pretrained("bert-base-uncased")

model = BertForSequenceClassification.from_pretrained("bert-base-uncased", num_labels=2).to("cuda")
optimizer = AdamW(model.parameters(), lr=2e-5)

for epoch in range(3):
    model.train()
    total_loss = 0
    for batch_idx in range(100):
        loss = torch.tensor(0.5 - epoch * 0.1, requires_grad=True)
        loss.backward()
        optimizer.step()
        optimizer.zero_grad()
        total_loss += loss.item()

        if batch_idx % 10 == 0:
            job_progress.report(
                progress=(epoch + batch_idx / 100) / 3.0,
                metrics={"loss": loss.item(), "epoch": epoch, "batch": batch_idx}
            )

    avg_loss = total_loss / 100
    job_progress.report(
        progress=(epoch + 1) / 3.0,
        metrics={"loss": avg_loss, "epoch": epoch + 1}
    )

job_paths.safe_save_model(model, "04_train/model.pth")
job_progress.complete(metrics={"final_loss": avg_loss})
'''

[[cells]]
id = "evaluate"
phase = "evaluation"
description = "Evaluate on validation set"
inputs = ["03_preprocess/val.parquet", "04_train/model.pth"]
outputs = ["05_evaluate/metrics.json"]
timeout = 120
retry = true
async_cell = false
code = '''
import json

metrics = {"accuracy": 0.923, "f1": 0.918, "precision": 0.931, "recall": 0.905}
out_dir = job_paths.makedirs("evaluate")
with open(f"{out_dir}/metrics.json", "w") as f:
    json.dump(metrics, f, indent=2)
print(f"✓ Accuracy: {metrics['accuracy']}")
'''

[[cells]]
id = "export"
phase = "artifact_export"
description = "Save final artifacts"
inputs = ["04_train/model.pth", "05_evaluate/metrics.json"]
outputs = ["06_export/model_final.pth", "06_export/metrics.json"]
timeout = 60
retry = true
async_cell = false
code = '''
import json

out_dir = job_paths.makedirs("export")

dst = job_paths.copy("train/model.pth", "export/model_final.pth")
print(f"✓ Model saved to {dst}")

dst_m = job_paths.copy("evaluate/metrics.json", "export/metrics.json")
print(f"✓ Metrics saved to {dst_m}")
'''
"#;

// ─── E2E Tests ──────────────────────────────────────────────────────────────

#[test]
fn e2e_parse_full_plan() {
    let orch = MlOrchestrator::new(MlOrchestratorConfig::default());
    let plan = orch.parse_plan(FULL_ML_PLAN, "job_123").unwrap();

    assert_eq!(plan.plan_name, "sentiment_bert_reviews");
    assert_eq!(plan.task_type, "text_classification");
    assert!(plan.requires_gpu);
    assert_eq!(plan.estimated_duration_minutes, 15);
    assert_eq!(plan.cells.len(), 6);

    // Verify phase ordering
    assert_eq!(plan.cells[0].phase, Phase::Setup);
    assert_eq!(plan.cells[1].phase, Phase::DataLoading);
    assert_eq!(plan.cells[2].phase, Phase::Preprocessing);
    assert_eq!(plan.cells[3].phase, Phase::Training);
    assert_eq!(plan.cells[4].phase, Phase::Evaluation);
    assert_eq!(plan.cells[5].phase, Phase::ArtifactExport);

    // Verify async cell
    assert!(!plan.cells[0].is_async);
    assert!(!plan.cells[1].is_async);
    assert!(!plan.cells[2].is_async);
    assert!(plan.cells[3].is_async);  // train cell
    assert!(!plan.cells[4].is_async);
    assert!(!plan.cells[5].is_async);
}

#[test]
fn e2e_capability_gate_all_cells() {
    let orch = MlOrchestrator::new(MlOrchestratorConfig::default());
    let plan = orch.parse_plan(FULL_ML_PLAN, "job_123").unwrap();

    // Every cell must pass the capability gate
    for cell in &plan.cells {
        let result = orch.validate_cell(cell);
        assert!(result.is_ok(), "Cell '{}' failed capability gate: {:?}", cell.cell_id, result.err());
    }
}

#[test]
fn e2e_wrap_sync_cells_have_helpers() {
    let orch = MlOrchestrator::new(MlOrchestratorConfig::default());
    let plan = orch.parse_plan(FULL_ML_PLAN, "job_123").unwrap();

    // All sync cells should have orchestrator helpers
    for cell in plan.cells.iter().filter(|c| !c.is_async) {
        let code = orch.prepare_cell(cell, "job_123", "/content/kria_jobs", "/content/drive/MyDrive/kria_jobs", "/data/reviews.csv");
        assert!(code.contains("KRIA HELPERS"), "Cell '{}' missing helpers", cell.cell_id);
        assert!(code.contains("JobPaths"), "Cell '{}' missing JobPaths", cell.cell_id);
        assert!(code.contains("JobProgress"), "Cell '{}' missing JobProgress", cell.cell_id);
        assert!(code.contains("try:"), "Cell '{}' missing try/except", cell.cell_id);
    }
}

#[test]
fn e2e_wrap_async_cell_has_subprocess() {
    let orch = MlOrchestrator::new(MlOrchestratorConfig::default());
    let plan = orch.parse_plan(FULL_ML_PLAN, "job_123").unwrap();

    let train_cell = &plan.cells[3]; // the async training cell
    assert!(train_cell.is_async);

    let code = orch.prepare_cell(train_cell, "job_123", "/content/kria_jobs", "/content/drive/MyDrive/kria_jobs", "/data/reviews.csv");

    // Must have subprocess (orchestrator-injected, not from LLM)
    assert!(code.contains("subprocess.Popen"), "Async cell missing subprocess");
    assert!(code.contains("KRIA_PID"), "Async cell missing PID output");
    assert!(code.contains("KRIA_ASYNC"), "Async cell missing ASYNC marker");
    assert!(code.contains("start_new_session=True"), "Async cell missing session isolation");

    // The LLM's inner code should NOT contain subprocess
    assert!(!train_cell.code.contains("subprocess"), "LLM code should not contain subprocess");

    // But the LLM code should use job_paths and job_progress
    assert!(train_cell.code.contains("job_paths.safe_save_model"), "LLM code should use safe_save_model");
    assert!(train_cell.code.contains("job_progress.report"), "LLM code should use job_progress.report");
    assert!(train_cell.code.contains("job_progress.complete"), "LLM code should use job_progress.complete");
}

#[test]
fn e2e_dag_validation() {
    let orch = MlOrchestrator::new(MlOrchestratorConfig::default());
    let plan = orch.parse_plan(FULL_ML_PLAN, "job_123").unwrap();

    // Verify DAG: each cell's inputs are produced by prior cells or external
    let mut produced: Vec<String> = Vec::new();
    for cell in &plan.cells {
        for input in &cell.inputs {
            if input == "DATASET" || input.starts_with("/home/") || input.starts_with("/content/") {
                continue; // external
            }
            assert!(
                produced.contains(input),
                "Cell '{}' requires input '{}' not produced by prior cells",
                cell.cell_id, input
            );
        }
        for output in &cell.outputs {
            produced.push(output.clone());
        }
    }
}

#[test]
fn e2e_sync_cell_generation() {
    let orch = MlOrchestrator::new(MlOrchestratorConfig::default());
    let plan = orch.parse_plan(FULL_ML_PLAN, "job_123").unwrap();

    let sync = orch.generate_phase_sync("job_123", &plan.cells[3], "/content/kria_jobs", "/content/drive/MyDrive/kria_jobs");

    assert!(sync.contains("model.pth"));
    assert!(sync.contains("manifest.json"));
    assert!(sync.contains("os.fsync"));
    assert!(sync.contains("os.replace"));
    assert!(sync.contains(".tmp"));
}

#[test]
fn e2e_ledger_actor_lifecycle() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_ledger.db");

        let ledger = LedgerActor::spawn(&db_path).unwrap();

        // Create job
        ledger.create_job("job_123", "test_plan", "classification").await.unwrap();

        // Mark phases
        ledger.mark_started("job_123", "setup", "setup", None).await.unwrap();
        ledger.mark_completed("job_123", "setup", vec![]).await.unwrap();

        ledger.mark_started("job_123", "train", "training", Some(12345)).await.unwrap();

        // Resume point should be "train" (only non-completed phase)
        let resume = ledger.get_resume_point("job_123").await.unwrap();
        assert_eq!(resume, Some("train".into()));

        // Mark train completed
        ledger.mark_completed("job_123", "train", vec![]).await.unwrap();

        // Resume point should be None (all completed)
        let resume = ledger.get_resume_point("job_123").await.unwrap();
        assert_eq!(resume, None);

        ledger.shutdown();
    });
}

#[test]
fn e2e_ledger_actor_concurrent_writes() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("concurrent.db");

        let ledger = LedgerActor::spawn(&db_path).unwrap();
        ledger.create_job("j1", "p1", "test").await.unwrap();

        // Spawn 10 concurrent tasks all writing to the ledger
        let mut handles = Vec::new();
        for i in 0..10 {
            let l = ledger.clone();
            handles.push(tokio::spawn(async move {
                let cell_id = format!("cell_{}", i);
                l.mark_started("j1", &cell_id, "setup", Some(i as u32)).await.unwrap();
                l.mark_completed("j1", &cell_id, vec![]).await.unwrap();
            }));
        }

        // All should complete without "database is locked" errors
        for h in handles {
            h.await.unwrap();
        }

        ledger.shutdown();
    });
}

#[test]
fn e2e_adaptive_poller_completes() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let cancel = CancellationToken::new();
        let poller = AdaptiveMlPoller {
            poll_interval: Duration::from_millis(50),
            max_training_duration: Duration::from_secs(5),
            heartbeat_floor: Duration::from_secs(1),
            heartbeat_ceiling: Duration::from_secs(10),
            cancel: cancel.clone(),
        };

        let now = chrono::Utc::now().timestamp() as f64;
        let colab = MockColabExecutor::new(vec![
            // First poll: running
            serde_json::json!({
                "state": "running", "progress": 0.5,
                "metrics": {"loss": 0.3, "epoch": 1},
                "pid": 12345, "heartbeat_ts": now,
                "timestamp": now, "batch_latencies_p95": 1.5
            }).to_string(),
            // Second poll: completed
            serde_json::json!({
                "state": "completed", "progress": 1.0,
                "metrics": {"accuracy": 0.92},
                "pid": 12345, "heartbeat_ts": now + 2.0,
                "timestamp": now + 2.0, "batch_latencies_p95": 1.5
            }).to_string(),
        ]);

        let exit = poller.run(&colab, "/tmp/status.json", "job_1", Some(12345), None).await;
        assert!(matches!(exit, MlPollerExit::Completed(_)));
    });
}

#[test]
fn e2e_adaptive_poller_detects_crash() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let cancel = CancellationToken::new();
        let poller = AdaptiveMlPoller {
            poll_interval: Duration::from_millis(50),
            max_training_duration: Duration::from_secs(5),
            heartbeat_floor: Duration::from_secs(1),
            heartbeat_ceiling: Duration::from_secs(10),
            cancel: cancel.clone(),
        };

        // PID check returns DEAD
        let colab = MockColabExecutor::new(vec![
            "DEAD".into(), // PID check
        ]);

        let exit = poller.run(&colab, "/tmp/status.json", "job_1", Some(12345), None).await;
        assert!(matches!(exit, MlPollerExit::Failed(ref msg) if msg.contains("crashed")));
    });
}

#[test]
fn e2e_adaptive_poller_cancelled() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let cancel = CancellationToken::new();
        let poller = AdaptiveMlPoller {
            poll_interval: Duration::from_millis(50),
            max_training_duration: Duration::from_secs(5),
            heartbeat_floor: Duration::from_secs(1),
            heartbeat_ceiling: Duration::from_secs(10),
            cancel: cancel.clone(),
        };

        // Cancel immediately
        cancel.cancel();

        let colab = MockColabExecutor::new(vec![]);
        let exit = poller.run(&colab, "/tmp/status.json", "job_1", Some(12345), None).await;
        assert!(matches!(exit, MlPollerExit::Cancelled));
    });
}

#[test]
fn e2e_integrity_roundtrip() {
    let dir = tempfile::tempdir().unwrap();

    // Create a fake model file
    let model_path = dir.path().join("model.pth");
    std::fs::write(&model_path, b"fake model weights 1234567890").unwrap();

    // Compute SHA-256
    let hash = compute_hash(&model_path, HashAlgorithm::Sha256).unwrap();
    assert!(matches!(hash, ContentHash::Sha256(_)));

    // Verify
    assert!(verify_hash(&model_path, &hash).unwrap());

    // Tamper with the file
    std::fs::write(&model_path, b"TAMPERED model weights").unwrap();
    assert!(!verify_hash(&model_path, &hash).unwrap());

    // Create a fake dataset file
    let data_path = dir.path().join("data.parquet");
    std::fs::write(&data_path, b"fake parquet data").unwrap();

    // Compute xxhash64
    let hash = compute_hash(&data_path, HashAlgorithm::Xxhash64).unwrap();
    assert!(matches!(hash, ContentHash::Xxhash64(_)));
    assert!(verify_hash(&data_path, &hash).unwrap());
}

#[test]
fn e2e_sync_cell_atomic_protocol() {
    let code = generate_sync_cell("job_123", "04_train", "/content/kria_jobs", "/content/drive/MyDrive/kria_jobs", &["model.pth", "training_status.json"]);

    // Must contain all atomic protocol steps
    assert!(code.contains("shutil.copy2"), "Missing shutil.copy2");
    assert!(code.contains("os.open(tmp, os.O_RDONLY)"), "Missing os.open for fsync");
    assert!(code.contains("os.fsync(fd)"), "Missing os.fsync");
    assert!(code.contains("os.close(fd)"), "Missing os.close");
    assert!(code.contains("os.replace(tmp, dst)"), "Missing os.replace (atomic rename)");
    assert!(code.contains("manifest.json"), "Missing manifest");
    assert!(code.contains(".tmp"), "Missing .tmp prefix");
}

#[test]
fn e2e_rejected_malicious_code() {
    let malicious_plans = vec![
        // os.system
        (r#"
[plan]
name = "evil1"
task = "test"
[[cells]]
id = "c1"
phase = "setup"
description = "evil"
code = '''
import os
os.system("rm -rf /")
'''
"#, "os.system"),
        // subprocess
        (r#"
[plan]
name = "evil2"
task = "test"
[[cells]]
id = "c1"
phase = "setup"
description = "evil"
code = '''
import subprocess
subprocess.Popen(["rm", "-rf", "/"])
'''
"#, "subprocess"),
        // pickle
        (r#"
[plan]
name = "evil3"
task = "test"
[[cells]]
id = "c1"
phase = "setup"
description = "evil"
code = '''
import pickle
pickle.loads(malicious_bytes)
'''
"#, "pickle"),
        // eval
        (r#"
[plan]
name = "evil4"
task = "test"
[[cells]]
id = "c1"
phase = "setup"
description = "evil"
code = '''
eval("__import__('os').system('rm -rf /')")
'''
"#, "eval"),
        // getattr bypass
        (r#"
[plan]
name = "evil5"
task = "test"
[[cells]]
id = "c1"
phase = "setup"
description = "evil"
code = '''
getattr(__import__("os"), "system")("rm -rf /")
'''
"#, "getattr"),
    ];

    let orch = MlOrchestrator::new(MlOrchestratorConfig::default());

    for (toml, desc) in malicious_plans {
        let plan = orch.parse_plan(toml, "j1").unwrap();
        let result = orch.validate_cell(&plan.cells[0]);
        assert!(result.is_err(), "Malicious code ({}) should be rejected", desc);
    }
}
