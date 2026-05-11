// crates/kria-core/src/agent/ml_orchestrator/plan_parser.rs
//
// TOML-only parser for Cloud LLM responses. Zero regex. Zero string slicing.

use serde::Deserialize;
use std::collections::HashSet;

use super::types::{CellPlan, ParsedCell, Phase};

/// Raw TOML payload from the Cloud LLM.
#[derive(Debug, Deserialize)]
struct LlmPayload {
    plan: PlanMeta,
    #[serde(default)]
    cells: Vec<RawCellDef>,
}

#[derive(Debug, Deserialize)]
struct PlanMeta {
    name: String,
    task: String,
    #[serde(default)]
    gpu_required: bool,
    #[serde(default = "default_duration")]
    estimated_duration_minutes: u32,
}

fn default_duration() -> u32 {
    15
}

/// A single cell definition — code is a TOML multiline literal string.
#[derive(Debug, Deserialize)]
struct RawCellDef {
    id: String,
    phase: String,
    description: String,
    #[serde(default)]
    inputs: Vec<String>,
    #[serde(default)]
    outputs: Vec<String>,
    #[serde(default = "default_timeout")]
    timeout: u64,
    #[serde(default)]
    retry: bool,
    #[serde(default)]
    async_cell: bool,
    code: String,
}

fn default_timeout() -> u64 {
    60
}

/// Parse the LLM's TOML response into a CellPlan.
/// This is the ONLY parsing function. No regex. No string slicing.
pub fn parse_cell_plan(raw: &str, job_id: &str) -> Result<CellPlan, PlanParseError> {
    let payload: LlmPayload =
        toml::from_str(raw).map_err(|e| PlanParseError::TomlParse(e.to_string()))?;

    if payload.cells.is_empty() {
        return Err(PlanParseError::NoCells);
    }

    let cells: Vec<ParsedCell> = payload
        .cells
        .into_iter()
        .map(|c| {
            let code = c.code.replace("{job_id}", job_id);
            Ok(ParsedCell {
                cell_id: c.id,
                phase: parse_phase(&c.phase)?,
                description: c.description,
                code,
                timeout_secs: c.timeout,
                retry_on_failure: c.retry,
                is_async: c.async_cell,
                inputs: c.inputs,
                outputs: c.outputs,
            })
        })
        .collect::<Result<Vec<_>, PlanParseError>>()?;

    validate_dag(&cells)?;

    Ok(CellPlan {
        plan_name: payload.plan.name,
        task_type: payload.plan.task,
        cells,
        estimated_duration_minutes: payload.plan.estimated_duration_minutes,
        requires_gpu: payload.plan.gpu_required,
    })
}

fn parse_phase(s: &str) -> Result<Phase, PlanParseError> {
    match s.to_lowercase().replace('_', "").as_str() {
        "datasetupload" | "upload" => Ok(Phase::DatasetUpload),
        "setup" => Ok(Phase::Setup),
        "dataloading" | "load" => Ok(Phase::DataLoading),
        "preprocessing" | "preprocess" => Ok(Phase::Preprocessing),
        "training" | "train" => Ok(Phase::Training),
        "evaluation" | "eval" => Ok(Phase::Evaluation),
        "artifactexport" | "export" => Ok(Phase::ArtifactExport),
        other => Err(PlanParseError::UnknownPhase(other.to_string())),
    }
}

/// Validate that cells form a valid DAG:
/// - No duplicate cell_ids
/// - Each cell's inputs are either external or produced by a prior cell
fn validate_dag(cells: &[ParsedCell]) -> Result<(), PlanParseError> {
    let mut produced: HashSet<String> = HashSet::new();
    let mut seen_ids = HashSet::new();

    for cell in cells {
        if !seen_ids.insert(&cell.cell_id) {
            return Err(PlanParseError::DuplicateCellId(cell.cell_id.clone()));
        }
        for input in &cell.inputs {
            if input == "DATASET" || input.starts_with("/home/") {
                continue; // external
            }
            if !produced.contains(input) {
                return Err(PlanParseError::MissingInput {
                    cell_id: cell.cell_id.clone(),
                    input: input.clone(),
                });
            }
        }
        for output in &cell.outputs {
            produced.insert(output.clone());
        }
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum PlanParseError {
    #[error("TOML parse error: {0}")]
    TomlParse(String),
    #[error("No cells found in plan")]
    NoCells,
    #[error("Unknown phase: {0}")]
    UnknownPhase(String),
    #[error("Duplicate cell ID: {0}")]
    DuplicateCellId(String),
    #[error("Cell '{cell_id}' requires input '{input}' not produced by any prior cell")]
    MissingInput { cell_id: String, input: String },
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
    fn parse_valid_toml() {
        let plan = parse_cell_plan(VALID_TOML, "job_123").unwrap();
        assert_eq!(plan.plan_name, "test_plan");
        assert_eq!(plan.task_type, "classification");
        assert!(plan.requires_gpu);
        assert_eq!(plan.estimated_duration_minutes, 10);
        assert_eq!(plan.cells.len(), 2);
        assert_eq!(plan.cells[0].cell_id, "setup");
        assert_eq!(plan.cells[0].phase, Phase::Setup);
        assert!(!plan.cells[0].is_async);
        assert_eq!(plan.cells[1].cell_id, "train");
        assert_eq!(plan.cells[1].phase, Phase::Training);
        assert!(plan.cells[1].is_async);
    }

    #[test]
    fn job_id_placeholder_replaced() {
        let plan = parse_cell_plan(VALID_TOML, "job_abc").unwrap();
        assert!(
            plan.cells[1].code.contains("job_abc") || plan.cells[1].code.contains("job_progress")
        );
    }

    #[test]
    fn rejects_empty_cells() {
        let toml = r#"
[plan]
name = "empty"
task = "test"
"#;
        assert!(matches!(
            parse_cell_plan(toml, "x"),
            Err(PlanParseError::NoCells)
        ));
    }

    #[test]
    fn rejects_unknown_phase() {
        let toml = r#"
[plan]
name = "bad"
task = "test"

[[cells]]
id = "c1"
phase = "bogus"
description = "test"
code = "pass"
"#;
        assert!(matches!(
            parse_cell_plan(toml, "x"),
            Err(PlanParseError::UnknownPhase(_))
        ));
    }

    #[test]
    fn rejects_duplicate_cell_id() {
        let toml = r#"
[plan]
name = "dup"
task = "test"

[[cells]]
id = "same"
phase = "setup"
description = "first"
code = "pass"

[[cells]]
id = "same"
phase = "training"
description = "second"
code = "pass"
"#;
        assert!(matches!(
            parse_cell_plan(toml, "x"),
            Err(PlanParseError::DuplicateCellId(_))
        ));
    }

    #[test]
    fn rejects_missing_dag_input() {
        let toml = r#"
[plan]
name = "dag"
task = "test"

[[cells]]
id = "train"
phase = "training"
description = "train"
inputs = ["01_setup/missing.json"]
outputs = ["model.pth"]
code = "pass"
"#;
        assert!(matches!(
            parse_cell_plan(toml, "x"),
            Err(PlanParseError::MissingInput { .. })
        ));
    }

    #[test]
    fn allows_external_dataset_input() {
        let toml = r#"
[plan]
name = "ext"
task = "test"

[[cells]]
id = "load"
phase = "data_loading"
description = "load"
inputs = ["DATASET"]
outputs = ["02_load/data.parquet"]
code = "pass"
"#;
        let plan = parse_cell_plan(toml, "x").unwrap();
        assert_eq!(plan.cells.len(), 1);
    }

    #[test]
    fn phase_dir_formatting() {
        let toml = VALID_TOML;
        let plan = parse_cell_plan(toml, "x").unwrap();
        assert_eq!(plan.cells[0].phase_dir(), "01_setup");
        assert_eq!(plan.cells[1].phase_dir(), "04_train");
    }
}
