//! Productivity Workflow Eval Suite.
//!
//! Validates file cognition, operational sequencing, artifact correctness,
//! and user-visible completion for common productivity tasks.
//!
//! Workflows tested:
//! - Create folder, move files, organize project structure
//! - Rename files by timestamp/pattern
//! - Create markdown notes from content
//! - Archive/zip project directories
//! - Summarize spreadsheet or CSV data

use crate::workflow_eval::contracts::file_management_contract;
use crate::workflow_eval::types::{
    EvalWorkflowCategory, ObservableOutputContract, SafetyClass, SemanticCompletionContract,
    WorkflowEvalCase,
};
use std::time::Duration;

fn prod_case(
    id: &str,
    description: &str,
    prompt: &str,
    contract: SemanticCompletionContract,
    safety: SafetyClass,
    requires_daemon: bool,
    tags: &[&str],
) -> WorkflowEvalCase {
    WorkflowEvalCase {
        id: id.to_string(),
        description: description.to_string(),
        prompt: prompt.to_string(),
        category: EvalWorkflowCategory::FileManagement,
        contract,
        safety_class: safety,
        interruption: None,
        timeout: Duration::from_secs(60),
        requires_daemon,
        requires_display: requires_daemon,
        tags: {
            let mut t = tags.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            t.push("productivity".into());
            t
        },
        eval_notes: format!(
            "Validates file/productivity cognition. Case: {}. \
             FAIL if artifact not created, operation described instead of executed.",
            id
        ),
    }
}

fn file_contract_with_signal(
    signal: &str,
    artifact_glob: Option<&str>,
) -> SemanticCompletionContract {
    let mut c = file_management_contract();
    c.semantic_success_signals = vec![signal.to_string()];
    if let Some(glob) = artifact_glob {
        c.required_observable_outputs = vec![ObservableOutputContract {
            description: format!("Artifact '{}' exists on disk", glob),
            response_must_contain: vec![signal.to_string()],
            artifact_path_glob: Some(glob.to_string()),
            artifact_min_bytes: Some(1),
            artifact_content_contains: None,
            required: true,
        }];
    }
    c
}

pub fn productivity_suite() -> Vec<WorkflowEvalCase> {
    vec![
        // ── File organization ─────────────────────────────────────────────────
        prod_case(
            "wf-prod-001-create-project-folder",
            "Create project folder structure and confirm creation",
            "create a folder called my_project with subfolders src and tests inside it",
            file_contract_with_signal("created", Some("~/my_project")),
            SafetyClass::Reversible,
            false,
            &["folder", "create", "structure"],
        ),
        prod_case(
            "wf-prod-002-move-files-to-folder",
            "Move a set of files into an organized folder",
            "move all the .txt files from the current directory into a folder called documents",
            file_contract_with_signal("moved", None),
            SafetyClass::Reversible,
            false,
            &["move", "organize", "files"],
        ),
        prod_case(
            "wf-prod-003-rename-screenshots",
            "Rename screenshot files by their timestamp",
            "find all screenshot files on the desktop and rename them with a date prefix like 2024-01-15_screenshot.png",
            file_contract_with_signal("renamed", None),
            SafetyClass::Reversible,
            false,
            &["rename", "screenshots", "timestamp"],
        ),
        // ── Notes and documentation ───────────────────────────────────────────
        prod_case(
            "wf-prod-004-markdown-notes",
            "Create a markdown notes file from given content",
            "create a markdown file called meeting_notes.md with today's date and space for action items",
            {
                let mut c = file_contract_with_signal("created", Some("~/meeting_notes.md"));
                c.required_observable_outputs[0].artifact_content_contains = Some("#".to_string());
                c
            },
            SafetyClass::Reversible,
            false,
            &["markdown", "notes", "create"],
        ),
        prod_case(
            "wf-prod-005-zip-project",
            "Archive a project directory into a zip file",
            "zip the my_project folder into a file called my_project.zip",
            file_contract_with_signal("zipped", Some("~/my_project.zip")),
            SafetyClass::Reversible,
            false,
            &["zip", "archive", "project"],
        ),
        // ── Data operations ───────────────────────────────────────────────────
        prod_case(
            "wf-prod-006-csv-summary",
            "Read a CSV file and summarize its row count",
            "read the data.csv file in the current directory and tell me how many rows it has",
            {
                let mut c = file_management_contract();
                c.success_definition = "CSV file read and row count surfaced to user".into();
                c.semantic_success_signals = vec!["rows".into(), "lines".into(), "entries".into()];
                c.required_observable_outputs[0].response_must_contain =
                    vec!["rows".into(), "lines".into()];
                c
            },
            SafetyClass::Safe,
            false,
            &["csv", "data", "summarize"],
        ),
        prod_case(
            "wf-prod-007-list-large-files",
            "Find and list all files larger than 100MB",
            "find all files larger than 100 megabytes in the home directory and list them",
            {
                let mut c = file_management_contract();
                c.success_definition = "Large files found and listed in response".into();
                c.semantic_success_signals = vec!["mb".into(), "files".into(), "found".into()];
                c.required_observable_outputs[0].response_must_contain =
                    vec!["mb".into(), "files".into()];
                c
            },
            SafetyClass::Safe,
            false,
            &["find", "large-files", "disk-usage"],
        ),
        // ── GUI-assisted productivity ─────────────────────────────────────────
        prod_case(
            "wf-prod-008-open-spreadsheet",
            "Open a spreadsheet in LibreOffice Calc",
            "open the sales.csv file in libreoffice calc",
            {
                let mut c = file_management_contract();
                c.success_definition = "Spreadsheet opened in LibreOffice Calc".into();
                c.semantic_success_signals = vec!["opened".into(), "calc".into()];
                c
            },
            SafetyClass::Safe,
            true,
            &["spreadsheet", "libreoffice", "gui"],
        ),
    ]
}
