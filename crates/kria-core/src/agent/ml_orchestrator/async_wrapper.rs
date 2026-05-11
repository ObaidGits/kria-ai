// crates/kria-core/src/agent/ml_orchestrator/async_wrapper.rs
//
// Orchestrator-owned async wrapper. The Cloud LLM NEVER writes subprocess
// boilerplate — it writes only the inner ML logic. This module wraps it.

use super::helpers_template::render_helpers;
use super::types::ParsedCell;

/// Generate the complete async cell code by wrapping the LLM's inner code
/// in an orchestrator-owned subprocess shell.
pub fn wrap_async_cell(
    cell: &ParsedCell,
    job_id: &str,
    hot_root: &str,
    cold_root: &str,
    dataset_path: &str,
    status_file: &str,
) -> String {
    let helpers = render_helpers(job_id, hot_root, cold_root, dataset_path, status_file);
    let _phase_dir = cell.phase_dir();

    // Build the worker script content
    let worker_script = format!(
        r#"import sys, os, time, json, traceback

{kria_helpers}

job_paths = JobPaths("{job_id}", "{hot_root}", "{cold_root}", "{dataset_path}")
job_progress = JobProgress("{status_file}")

try:
{indented_inner}
except Exception as e:
    job_progress.fail(error=str(e))
    sys.exit(1)
"#,
        kria_helpers = helpers,
        job_id = job_id,
        hot_root = hot_root,
        cold_root = cold_root,
        dataset_path = dataset_path,
        status_file = status_file,
        indented_inner = indent_code(&cell.code, 4),
    );

    // Generate the launcher cell that writes the worker script and spawns it
    let worker_path = format!("{}/kria_worker_{}.py", hot_root, cell.cell_id);
    format!(
        r#"import subprocess, sys, os

worker_code = r'''{worker_script}'''

worker_path = "{worker_path}"
os.makedirs(os.path.dirname(worker_path), exist_ok=True)
with open(worker_path, "w") as f:
    f.write(worker_code)

proc = subprocess.Popen(
    [sys.executable, worker_path],
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    start_new_session=True,
)
print(f"KRIA_PID: {{proc.pid}}")
print(f"KRIA_ASYNC: Worker started (PID={{proc.pid}})")
"#,
        worker_script = worker_script,
        worker_path = worker_path,
    )
}

/// Wrap a synchronous cell with orchestrator helpers prepended.
pub fn wrap_sync_cell(
    cell: &ParsedCell,
    job_id: &str,
    hot_root: &str,
    cold_root: &str,
    dataset_path: &str,
    status_file: &str,
) -> String {
    let helpers = render_helpers(job_id, hot_root, cold_root, dataset_path, status_file);

    format!(
        r#"# === KRIA HELPERS (auto-injected) ===
{kria_helpers}

job_paths = JobPaths("{job_id}", "{hot_root}", "{cold_root}", "{dataset_path}")
job_progress = JobProgress("{status_file}")

# === LLM CODE ===
try:
{indented_inner}
except Exception as e:
    job_progress.fail(error=str(e))
    raise
"#,
        kria_helpers = helpers,
        job_id = job_id,
        hot_root = hot_root,
        cold_root = cold_root,
        dataset_path = dataset_path,
        status_file = status_file,
        indented_inner = indent_code(&cell.code, 4),
    )
}

fn indent_code(code: &str, spaces: usize) -> String {
    let indent = " ".repeat(spaces);
    code.lines()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                format!("{}{}", indent, line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::super::types::Phase;
    use super::*;

    fn test_cell(is_async: bool) -> ParsedCell {
        ParsedCell {
            cell_id: "train".into(),
            phase: Phase::Training,
            description: "Test".into(),
            code: "print('hello')\njob_progress.report(progress=0.5)".into(),
            timeout_secs: 10,
            retry_on_failure: false,
            is_async,
            inputs: vec![],
            outputs: vec!["04_train/model.pth".into()],
        }
    }

    #[test]
    fn sync_cell_has_helpers() {
        let cell = test_cell(false);
        let code = wrap_sync_cell(
            &cell,
            "j1",
            "/hot",
            "/cold",
            "/data.csv",
            "/hot/status.json",
        );
        assert!(code.contains("KRIA HELPERS"));
        assert!(code.contains("JobPaths"));
        assert!(code.contains("JobProgress"));
        assert!(code.contains("print('hello')"));
        assert!(code.contains("try:"));
        assert!(code.contains("except"));
    }

    #[test]
    fn async_cell_has_subprocess() {
        let cell = test_cell(true);
        let code = wrap_async_cell(
            &cell,
            "j1",
            "/hot",
            "/cold",
            "/data.csv",
            "/hot/status.json",
        );
        assert!(code.contains("subprocess.Popen"));
        assert!(code.contains("KRIA_PID"));
        assert!(code.contains("KRIA_ASYNC"));
        assert!(code.contains("kria_worker_train.py"));
        // The inner code should NOT contain subprocess — that's the whole point
        assert!(!cell.code.contains("subprocess"));
    }

    #[test]
    fn async_cell_wraps_in_try_except() {
        let cell = test_cell(true);
        let code = wrap_async_cell(
            &cell,
            "j1",
            "/hot",
            "/cold",
            "/data.csv",
            "/hot/status.json",
        );
        // The worker script (inside the raw string) should have try/except
        assert!(code.contains("try:"));
        assert!(code.contains("except Exception as e:"));
        assert!(code.contains("job_progress.fail"));
    }
}
