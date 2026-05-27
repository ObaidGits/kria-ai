//! Universal execution-result synthesis layer.
//!
//! Transforms raw tool outputs into intelligent, human-readable responses.
//! Separates conversational answers from debug/execution metadata.
//!
//! # Design Principles
//! - Universal: Works for all tools without per-tool hardcoding
//! - Deterministic: Consistent synthesis rules
//! - Transparent: Preserves raw output in debug section
//! - Human-readable: Tables, lists, paragraphs — whatever fits the data
//! - Maintainable: No giant if/else chains or regex formatters
//!
//! # Architecture
//! ```
//! ToolResult → ResultSynthesizer → SynthesizedResult
//!                                   ├─ human_readable   (markdown for display)
//!                                   ├─ conversational_summary (one-liner)
//!                                   ├─ execution_metadata
//!                                   ├─ raw_payload (debug/expandable)
//!                                   └─ verification_outcome (optional)
//! ```

use crate::infra::isolation::ToolResult;
use serde::{Deserialize, Serialize};

// ─── Synthesized Result ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesizedResult {
    /// Full human-readable markdown response for display.
    /// May contain tables, lists, paragraphs, code blocks.
    pub human_readable: String,

    /// One-line conversational summary (for collapsed UI badge).
    pub conversational_summary: String,

    /// Structured execution metadata (counts, status, timing, etc.)
    pub execution_metadata: ExecutionMetadata,

    /// Raw tool output for debugging (expandable in UI)
    pub raw_payload: serde_json::Value,

    /// Verification outcome (if verifier ran)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_outcome: Option<VerificationOutcome>,

    /// Original success flag from ToolResult
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionMetadata {
    pub tool: String,
    pub outcome: OutcomeClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeClass {
    Success,
    SuccessEmpty,
    PartialSuccess,
    Failure,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationOutcome {
    pub verified: bool,
    pub confidence: f64,
    pub evidence: String,
}

// ─── Shell Output Formatter ──────────────────────────────────────────────────

/// Detects whether stdout looks like a columnar table (header + data rows).
/// Heuristic: first non-empty line has ≥2 whitespace-separated tokens,
/// and at least one subsequent line has the same token count.
fn looks_like_table(lines: &[&str]) -> bool {
    let non_empty: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if non_empty.len() < 2 {
        return false;
    }
    let header_cols = non_empty[0].split_whitespace().count();
    if header_cols < 2 {
        return false;
    }
    // At least half the data rows should have ≥2 tokens
    let matching = non_empty[1..]
        .iter()
        .filter(|l| l.split_whitespace().count() >= 2)
        .count();
    matching >= non_empty[1..].len().saturating_sub(1).max(1)
}

/// Parse a whitespace-delimited table into (headers, rows).
/// Handles variable-width columns by using the header positions as column boundaries.
fn parse_columnar_table<'a>(lines: &[&'a str]) -> (Vec<String>, Vec<Vec<String>>) {
    let non_empty: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if non_empty.is_empty() {
        return (vec![], vec![]);
    }

    let header_line = non_empty[0];

    // Find column start positions from the header
    let mut col_starts: Vec<usize> = vec![0];
    let mut in_word = false;
    for (i, ch) in header_line.char_indices() {
        if ch == ' ' || ch == '\t' {
            in_word = false;
        } else if !in_word {
            if i > 0 {
                col_starts.push(i);
            }
            in_word = true;
        }
    }

    // Extract header names
    let headers: Vec<String> = col_starts
        .iter()
        .enumerate()
        .map(|(ci, &start)| {
            let end = col_starts.get(ci + 1).copied().unwrap_or(header_line.len());
            header_line
                .get(start..end.min(header_line.len()))
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .filter(|h| !h.is_empty())
        .collect();

    // Extract data rows using same column boundaries
    let rows: Vec<Vec<String>> = non_empty[1..]
        .iter()
        .map(|line| {
            col_starts
                .iter()
                .enumerate()
                .map(|(ci, &start)| {
                    let end = col_starts.get(ci + 1).copied().unwrap_or(line.len() + 1);
                    line.get(start..end.min(line.len()))
                        .unwrap_or("")
                        .trim()
                        .to_string()
                })
                .collect()
        })
        .filter(|row: &Vec<String>| row.iter().any(|c| !c.is_empty()))
        .collect();

    (headers, rows)
}

/// Render (headers, rows) as a markdown table.
fn render_markdown_table(headers: &[String], rows: &[Vec<String>]) -> String {
    if headers.is_empty() {
        return String::new();
    }

    // Compute column widths
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len().max(3)).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    let mut out = String::new();

    // Header row
    out.push('|');
    for (i, h) in headers.iter().enumerate() {
        out.push_str(&format!(" {:<width$} |", h, width = widths[i]));
    }
    out.push('\n');

    // Separator
    out.push('|');
    for w in &widths {
        out.push_str(&format!(" {:-<width$} |", "", width = w));
    }
    out.push('\n');

    // Data rows
    for row in rows {
        out.push('|');
        for (i, w) in widths.iter().enumerate() {
            let cell = row.get(i).map(|s| s.as_str()).unwrap_or("");
            out.push_str(&format!(" {:<width$} |", cell, width = w));
        }
        out.push('\n');
    }

    out
}

/// Detect if lines look like a key: value list (e.g. `Name: foo`).
fn looks_like_kv_list(lines: &[&str]) -> bool {
    let non_empty: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if non_empty.len() < 2 {
        return false;
    }
    let kv_count = non_empty
        .iter()
        .filter(|l| l.contains(':') || l.contains('='))
        .count();
    kv_count * 2 >= non_empty.len()
}

/// Detect if lines look like a simple list (one item per line, no columns).
fn looks_like_simple_list(lines: &[&str]) -> bool {
    let non_empty: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if non_empty.len() < 2 {
        return false;
    }
    // Each line has roughly the same token count (1-3) and no obvious table structure
    let avg_tokens: usize = non_empty
        .iter()
        .map(|l| l.split_whitespace().count())
        .sum::<usize>()
        / non_empty.len().max(1);
    avg_tokens <= 4 && !looks_like_table(lines)
}

/// Format stdout into a human-readable markdown string.
/// Detects tables, key-value lists, simple lists, and plain paragraphs.
/// Handles truncation gracefully.
pub fn format_stdout_for_human(stdout: &str, truncated: bool, max_rows: usize) -> String {
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return "_No output._".to_string();
    }

    let all_lines: Vec<&str> = stdout.lines().collect();
    let total_lines = all_lines.len();
    let display_lines: Vec<&str> = all_lines.iter().copied().take(max_rows).collect();

    let mut out = String::new();

    if looks_like_table(&display_lines) {
        let (headers, rows) = parse_columnar_table(&display_lines);
        if !headers.is_empty() {
            out.push_str(&render_markdown_table(&headers, &rows));
        } else {
            // Fallback: code block
            out.push_str("```\n");
            out.push_str(&display_lines.join("\n"));
            out.push_str("\n```\n");
        }
    } else if looks_like_kv_list(&display_lines) {
        for line in &display_lines {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                out.push('\n');
                continue;
            }
            // Render as bold key + value
            if let Some(pos) = trimmed.find(':') {
                let key = trimmed[..pos].trim();
                let val = trimmed[pos + 1..].trim();
                out.push_str(&format!("- **{}**: {}\n", key, val));
            } else if let Some(pos) = trimmed.find('=') {
                let key = trimmed[..pos].trim();
                let val = trimmed[pos + 1..].trim();
                out.push_str(&format!("- **{}**: {}\n", key, val));
            } else {
                out.push_str(&format!("- {}\n", trimmed));
            }
        }
    } else if looks_like_simple_list(&display_lines) {
        for line in &display_lines {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                out.push_str(&format!("- {}\n", trimmed));
            }
        }
    } else if total_lines <= 6 {
        // Short output: inline code block
        out.push_str("```\n");
        out.push_str(&display_lines.join("\n"));
        out.push_str("\n```\n");
    } else {
        // Long prose or mixed: code block
        out.push_str("```\n");
        out.push_str(&display_lines.join("\n"));
        out.push_str("\n```\n");
    }

    // Truncation notice
    if truncated || total_lines > max_rows {
        let hidden = total_lines.saturating_sub(max_rows);
        if hidden > 0 {
            out.push_str(&format!(
                "\n> ⚠️ Output truncated — {} more line{} not shown.\n",
                hidden,
                if hidden == 1 { "" } else { "s" }
            ));
        } else {
            out.push_str("\n> ⚠️ Output was truncated by the system.\n");
        }
    }

    out
}

// ─── Result Synthesizer ─────────────────────────────────────────────────────

pub struct ResultSynthesizer {
    /// Max lines to render in human-readable output before truncating.
    max_display_rows: usize,
}

impl Default for ResultSynthesizer {
    fn default() -> Self {
        Self {
            max_display_rows: 200,
        }
    }
}

impl ResultSynthesizer {
    pub fn new(max_display_rows: usize) -> Self {
        Self { max_display_rows }
    }

    /// Synthesize a ToolResult into a SynthesizedResult.
    pub fn synthesize(
        &self,
        tool_name: &str,
        tool_result: &ToolResult,
        verification: Option<VerificationOutcome>,
    ) -> SynthesizedResult {
        if !tool_result.success {
            return self.synthesize_failure(tool_name, tool_result, verification);
        }

        let outcome = self.classify_outcome(tool_result);
        let item_count = self.extract_item_count(&tool_result.data);
        let duration_ms = self.extract_duration(&tool_result.data);
        let exit_code = self.extract_exit_code(&tool_result.data);
        let truncated = self.check_truncation(&tool_result.data);

        let (human_readable, conversational_summary) = self.generate_human_response(
            tool_name,
            &tool_result.data,
            outcome,
            item_count,
            exit_code,
            truncated,
        );

        SynthesizedResult {
            human_readable,
            conversational_summary,
            execution_metadata: ExecutionMetadata {
                tool: tool_name.to_string(),
                outcome,
                item_count,
                duration_ms,
                exit_code,
                truncated,
                extra: None,
            },
            raw_payload: tool_result.data.clone(),
            verification_outcome: verification,
            success: true,
        }
    }

    fn synthesize_failure(
        &self,
        tool_name: &str,
        tool_result: &ToolResult,
        verification: Option<VerificationOutcome>,
    ) -> SynthesizedResult {
        let error_msg = tool_result.error.as_deref().unwrap_or("unknown error");
        let first_line = error_msg.lines().next().unwrap_or(error_msg);
        let summary = if first_line.len() > 200 {
            format!("{}…", &first_line[..200])
        } else {
            first_line.to_string()
        };

        let human_readable = format!("**`{}` failed**\n\n> {}\n", tool_name, summary);
        let conversational_summary = format!("❌ {} failed: {}", tool_name, summary);

        SynthesizedResult {
            human_readable,
            conversational_summary,
            execution_metadata: ExecutionMetadata {
                tool: tool_name.to_string(),
                outcome: OutcomeClass::Failure,
                item_count: None,
                duration_ms: None,
                exit_code: None,
                truncated: None,
                extra: None,
            },
            raw_payload: tool_result.data.clone(),
            verification_outcome: verification,
            success: false,
        }
    }

    /// Generate both the full human-readable response and the one-line summary.
    fn generate_human_response(
        &self,
        tool_name: &str,
        data: &serde_json::Value,
        outcome: OutcomeClass,
        item_count: Option<u64>,
        exit_code: Option<i32>,
        truncated: Option<bool>,
    ) -> (String, String) {
        match outcome {
            OutcomeClass::SuccessEmpty => {
                let hr = format!("**`{}`** completed — no results found.\n", tool_name);
                let summary = format!("✅ {} — no results", tool_name);
                (hr, summary)
            }

            OutcomeClass::PartialSuccess => {
                let stdout = data.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
                let is_trunc = truncated.unwrap_or(false);
                let hr = if !stdout.trim().is_empty() {
                    format!(
                        "**`{}`** completed (partial output):\n\n{}",
                        tool_name,
                        format_stdout_for_human(stdout, is_trunc, self.max_display_rows)
                    )
                } else {
                    format!(
                        "**`{}`** completed with partial results ({} items).\n",
                        tool_name,
                        item_count.unwrap_or(0)
                    )
                };
                let summary = format!(
                    "⚠️ {} — partial results{}",
                    tool_name,
                    if is_trunc { ", output truncated" } else { "" }
                );
                (hr, summary)
            }

            OutcomeClass::Success => {
                self.generate_success_response(tool_name, data, item_count, exit_code)
            }

            _ => {
                let hr = format!("**`{}`** completed.\n", tool_name);
                let summary = format!("✅ {}", tool_name);
                (hr, summary)
            }
        }
    }

    fn generate_success_response(
        &self,
        tool_name: &str,
        data: &serde_json::Value,
        item_count: Option<u64>,
        exit_code: Option<i32>,
    ) -> (String, String) {
        // ── Shell command output ──────────────────────────────────────────
        if let Some(stdout) = data.get("stdout").and_then(|v| v.as_str()) {
            let truncated = data
                .get("stdout_truncated")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let stdout_trimmed = stdout.trim();

            if stdout_trimmed.is_empty() {
                let hr = format!(
                    "**`{}`** completed successfully (exit code {}, no output).\n",
                    tool_name,
                    exit_code.unwrap_or(0)
                );
                let summary = format!("✅ {} — completed, no output", tool_name);
                return (hr, summary);
            }

            let line_count = stdout_trimmed.lines().count();
            let formatted =
                format_stdout_for_human(stdout_trimmed, truncated, self.max_display_rows);

            let hr = format!("**`{}`** output:\n\n{}", tool_name, formatted);
            let summary = format!(
                "✅ {} — {} line{}",
                tool_name,
                line_count,
                if line_count == 1 { "" } else { "s" }
            );
            return (hr, summary);
        }

        // ── Structured results (search, list, etc.) ───────────────────────
        if let Some(count) = item_count {
            let hr = self.format_structured_results(tool_name, data, count);
            let summary = format!(
                "✅ {} — {} item{}",
                tool_name,
                count,
                if count == 1 { "" } else { "s" }
            );
            return (hr, summary);
        }

        // ── File operation ────────────────────────────────────────────────
        if let Some(path) = data.get("path").and_then(|v| v.as_str()) {
            let hr = format!("**`{}`** completed: `{}`\n", tool_name, path);
            let summary = format!("✅ {} — {}", tool_name, path);
            return (hr, summary);
        }

        // ── Generic success ───────────────────────────────────────────────
        let hr = format!("**`{}`** completed successfully.\n", tool_name);
        let summary = format!("✅ {}", tool_name);
        (hr, summary)
    }

    fn format_structured_results(
        &self,
        tool_name: &str,
        data: &serde_json::Value,
        count: u64,
    ) -> String {
        // Find the array of results
        let arr = if let Some(a) = data.as_array() {
            Some(a.as_slice())
        } else {
            data.get("results")
                .or_else(|| data.get("items"))
                .or_else(|| data.get("files"))
                .or_else(|| data.get("rows"))
                .and_then(|v| v.as_array())
                .map(|v| v.as_slice())
        };

        let Some(arr) = arr else {
            return format!(
                "**`{}`** returned {} item{}.\n",
                tool_name,
                count,
                if count == 1 { "" } else { "s" }
            );
        };

        if arr.is_empty() {
            return format!("**`{}`** returned no results.\n", tool_name);
        }

        // Detect if items are objects with consistent keys → render as table
        let first = &arr[0];
        if let Some(obj) = first.as_object() {
            let keys: Vec<&str> = obj.keys().map(|k| k.as_str()).take(6).collect();
            if keys.len() >= 2 {
                let headers: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
                let rows: Vec<Vec<String>> = arr
                    .iter()
                    .take(self.max_display_rows)
                    .map(|item| {
                        keys.iter()
                            .map(|k| {
                                item.get(*k)
                                    .map(|v| match v {
                                        serde_json::Value::String(s) => s.clone(),
                                        serde_json::Value::Null => String::new(),
                                        other => other.to_string(),
                                    })
                                    .unwrap_or_default()
                            })
                            .collect()
                    })
                    .collect();

                let mut out = format!(
                    "**`{}`** returned {} item{}:\n\n",
                    tool_name,
                    count,
                    if count == 1 { "" } else { "s" }
                );
                out.push_str(&render_markdown_table(&headers, &rows));
                if arr.len() > self.max_display_rows {
                    out.push_str(&format!(
                        "\n> ⚠️ Showing {} of {} items.\n",
                        self.max_display_rows, count
                    ));
                }
                return out;
            }
        }

        // Items are strings or simple values → bullet list
        let mut out = format!(
            "**`{}`** returned {} item{}:\n\n",
            tool_name,
            count,
            if count == 1 { "" } else { "s" }
        );
        for item in arr.iter().take(self.max_display_rows) {
            let label = match item {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Object(o) => {
                    // Pick first string field
                    o.values()
                        .find_map(|v| v.as_str().map(|s| s.to_string()))
                        .unwrap_or_else(|| item.to_string())
                }
                other => other.to_string(),
            };
            out.push_str(&format!("- {}\n", label));
        }
        if arr.len() > self.max_display_rows {
            out.push_str(&format!(
                "\n> ⚠️ Showing {} of {} items.\n",
                self.max_display_rows, count
            ));
        }
        out
    }

    // ─── Helpers ─────────────────────────────────────────────────────────────

    fn classify_outcome(&self, tool_result: &ToolResult) -> OutcomeClass {
        let data = &tool_result.data;
        if self.is_empty_result(data) {
            return OutcomeClass::SuccessEmpty;
        }
        if self.has_partial_success_markers(data) {
            return OutcomeClass::PartialSuccess;
        }
        OutcomeClass::Success
    }

    fn is_empty_result(&self, data: &serde_json::Value) -> bool {
        if let Some(arr) = data.as_array() {
            return arr.is_empty();
        }
        if let Some(obj) = data.as_object() {
            if obj.is_empty() {
                return true;
            }
            for key in [
                "results", "items", "messages", "events", "files", "rows", "data",
            ] {
                if let Some(arr) = obj.get(key).and_then(|v| v.as_array()) {
                    if arr.is_empty() {
                        return true;
                    }
                }
            }
            // stdout present but empty
            if let Some(s) = obj.get("stdout").and_then(|v| v.as_str()) {
                if s.trim().is_empty() {
                    // Only empty if no stderr either
                    let stderr = obj
                        .get("stderr")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    if stderr.is_empty() {
                        return true;
                    }
                }
            }
        }
        if let Some(s) = data.as_str() {
            return s.trim().is_empty();
        }
        false
    }

    fn has_partial_success_markers(&self, data: &serde_json::Value) -> bool {
        if let Some(obj) = data.as_object() {
            if obj
                .get("stdout_truncated")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || obj
                    .get("stderr_truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                || obj
                    .get("truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            {
                return true;
            }
            if let Some(failed) = obj.get("failed_count").and_then(|v| v.as_u64()) {
                if failed > 0 {
                    return true;
                }
            }
        }
        false
    }

    fn extract_item_count(&self, data: &serde_json::Value) -> Option<u64> {
        if let Some(count) = data.get("count").and_then(|v| v.as_u64()) {
            return Some(count);
        }
        if let Some(arr) = data.as_array() {
            return Some(arr.len() as u64);
        }
        for key in [
            "results", "items", "messages", "events", "files", "rows", "data",
        ] {
            if let Some(arr) = data.get(key).and_then(|v| v.as_array()) {
                return Some(arr.len() as u64);
            }
        }
        None
    }

    fn extract_duration(&self, data: &serde_json::Value) -> Option<u128> {
        data.get("duration_ms")
            .and_then(|v| v.as_u64())
            .map(|v| v as u128)
    }

    fn extract_exit_code(&self, data: &serde_json::Value) -> Option<i32> {
        data.get("exit_code")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32)
    }

    fn check_truncation(&self, data: &serde_json::Value) -> Option<bool> {
        if let Some(obj) = data.as_object() {
            if obj
                .get("stdout_truncated")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || obj
                    .get("stderr_truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                || obj
                    .get("truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            {
                return Some(true);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_detection() {
        let lines = vec![
            "CONTAINER ID   IMAGE         STATUS    PORTS",
            "abc123         nginx         Up 2h     80/tcp",
            "def456         postgres      Up 1d     5432/tcp",
        ];
        assert!(looks_like_table(&lines));
    }

    #[test]
    fn test_kv_detection() {
        let lines = vec!["Name: nginx", "Status: running", "Port: 80"];
        assert!(looks_like_kv_list(&lines));
    }

    #[test]
    fn test_simple_list_detection() {
        let lines = vec!["file1.txt", "file2.txt", "file3.txt"];
        assert!(looks_like_simple_list(&lines));
    }

    #[test]
    fn test_format_stdout_table() {
        let stdout = "CONTAINER ID   IMAGE    STATUS\nabc123         nginx    Up 2h\ndef456         redis    Up 1d";
        let result = format_stdout_for_human(stdout, false, 200);
        assert!(result.contains('|'), "should render as markdown table");
        assert!(result.contains("nginx"));
        assert!(result.contains("redis"));
    }

    #[test]
    fn test_format_stdout_list() {
        let stdout = "file1.txt\nfile2.txt\nfile3.txt";
        let result = format_stdout_for_human(stdout, false, 200);
        assert!(result.contains("- file1.txt"));
    }

    #[test]
    fn test_format_stdout_truncation_notice() {
        let stdout = "line1\nline2\nline3";
        let result = format_stdout_for_human(stdout, true, 200);
        assert!(result.contains("truncated"));
    }

    #[test]
    fn test_synthesize_shell_table() {
        let synthesizer = ResultSynthesizer::default();
        let tool_result = ToolResult::ok(serde_json::json!({
            "exit_code": 0,
            "stdout": "CONTAINER ID   IMAGE    STATUS\nabc123         nginx    Up 2h\ndef456         redis    Up 1d",
            "stderr": "",
        }));
        let result = synthesizer.synthesize("execute_bash", &tool_result, None);
        assert!(result.success);
        assert!(
            result.human_readable.contains('|'),
            "should contain markdown table"
        );
        assert!(result.human_readable.contains("nginx"));
        assert!(result.conversational_summary.contains("✅"));
    }

    #[test]
    fn test_synthesize_empty_result() {
        let synthesizer = ResultSynthesizer::default();
        let tool_result = ToolResult::ok(serde_json::json!({ "results": [] }));
        let result = synthesizer.synthesize("search_files", &tool_result, None);
        assert!(result.success);
        assert_eq!(
            result.execution_metadata.outcome,
            OutcomeClass::SuccessEmpty
        );
        assert!(result.conversational_summary.contains("no results"));
    }

    #[test]
    fn test_synthesize_failure() {
        let synthesizer = ResultSynthesizer::default();
        let tool_result = ToolResult::err("permission denied");
        let result = synthesizer.synthesize("read_file", &tool_result, None);
        assert!(!result.success);
        assert_eq!(result.execution_metadata.outcome, OutcomeClass::Failure);
        assert!(result.conversational_summary.contains("❌"));
        assert!(result.conversational_summary.contains("permission denied"));
    }

    #[test]
    fn test_synthesize_truncated() {
        let synthesizer = ResultSynthesizer::default();
        let tool_result = ToolResult::ok(serde_json::json!({
            "stdout": "line1\nline2\nline3",
            "stdout_truncated": true,
            "exit_code": 0,
        }));
        let result = synthesizer.synthesize("execute_bash", &tool_result, None);
        assert!(result.success);
        assert_eq!(
            result.execution_metadata.outcome,
            OutcomeClass::PartialSuccess
        );
        assert!(result.human_readable.contains("truncated"));
    }

    #[test]
    fn test_render_markdown_table() {
        let headers = vec!["Name".to_string(), "Status".to_string()];
        let rows = vec![
            vec!["nginx".to_string(), "running".to_string()],
            vec!["redis".to_string(), "stopped".to_string()],
        ];
        let table = render_markdown_table(&headers, &rows);
        assert!(table.contains("| Name"));
        assert!(table.contains("| nginx"));
        assert!(table.contains("---"));
    }
}
