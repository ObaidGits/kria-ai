//! Eval Report Generation — Structured output for analysis.

use super::runner::EvalBatchResult;
use std::path::PathBuf;

/// Save eval report to disk.
pub fn save_report(result: &EvalBatchResult, output_dir: &PathBuf) -> Result<PathBuf, String> {
    let _ = std::fs::create_dir_all(output_dir);

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Save human-readable report
    let report_path = output_dir.join(format!("gui_eval_report_{}.txt", timestamp));
    std::fs::write(&report_path, result.to_report())
        .map_err(|e| format!("Failed to write report: {}", e))?;

    // Save JSON for programmatic analysis
    let json_path = output_dir.join(format!("gui_eval_results_{}.json", timestamp));
    let json = serde_json::to_string_pretty(result)
        .map_err(|e| format!("Failed to serialize results: {}", e))?;
    std::fs::write(&json_path, json).map_err(|e| format!("Failed to write JSON: {}", e))?;

    Ok(report_path)
}
