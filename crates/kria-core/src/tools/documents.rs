use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::sidecar::SidecarBridge;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

/// Shared sidecar handle for document tools.
#[derive(Clone)]
struct DocSidecar(Option<Arc<Mutex<Arc<SidecarBridge>>>>);

impl DocSidecar {
    async fn try_extract(&self, path: &str, operations: &[&str]) -> Option<serde_json::Value> {
        let bridge = self.0.as_ref()?;
        let bridge = bridge.lock().await;
        let params = serde_json::json!({
            "file": path,
            "operations": operations,
        });
        bridge.request("document.extract", params).await.ok()
    }
}

struct ParseDocument {
    sidecar: DocSidecar,
}

#[async_trait]
impl ToolHandler for ParseDocument {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let path = params["path"].as_str().unwrap_or("");
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        // Try sidecar for rich extraction (PDF, DOCX)
        if matches!(ext.as_str(), "pdf" | "docx" | "xlsx") {
            if let Some(result) = self
                .sidecar
                .try_extract(path, &["text", "tables", "sections"])
                .await
            {
                return ToolResult::ok(serde_json::json!({
                    "path": path, "format": ext, "backend": "sidecar",
                    "result": result,
                }));
            }
        }

        // Fallback to Rust-native extraction
        match ext.as_str() {
            "txt" | "md" | "log" | "json" | "yaml" | "yml" | "toml" | "csv" | "xml" => {
                match tokio::fs::read_to_string(path).await {
                    Ok(content) => {
                        let max = 50000;
                        let truncated = content.len() > max;
                        let text = if truncated { &content[..max] } else { &content };
                        ToolResult::ok(serde_json::json!({
                            "path": path, "format": ext, "content": text,
                            "truncated": truncated, "total_chars": content.len(),
                            "backend": "native",
                        }))
                    }
                    Err(e) => ToolResult::err(format!("read failed: {e}")),
                }
            }
            // Code and notebook files — treated as plain text
            "py" | "rs" | "ts" | "js" | "go" | "java" | "c" | "cpp" | "h" | "sh" | "rb" | "php"
            | "swift" | "kt" | "r" | "sql" | "lua" => match tokio::fs::read_to_string(path).await {
                Ok(content) => {
                    let max = 50000;
                    let truncated = content.len() > max;
                    let text = if truncated { &content[..max] } else { &content };
                    ToolResult::ok(serde_json::json!({
                        "path": path, "format": ext, "content": text,
                        "truncated": truncated, "total_chars": content.len(),
                        "backend": "native",
                    }))
                }
                Err(e) => ToolResult::err(format!("read failed: {e}")),
            },
            // Jupyter notebooks — extract only source cells, skip outputs
            "ipynb" => match tokio::fs::read_to_string(path).await {
                Ok(raw) => {
                    let nb: serde_json::Value = match serde_json::from_str(&raw) {
                        Ok(v) => v,
                        Err(e) => return ToolResult::err(format!("invalid notebook JSON: {e}")),
                    };
                    let cells = nb["cells"].as_array();
                    let mut extracted = String::new();
                    if let Some(cells) = cells {
                        for (i, cell) in cells.iter().enumerate() {
                            let cell_type = cell["cell_type"].as_str().unwrap_or("unknown");
                            let source = cell["source"]
                                .as_array()
                                .map(|lines| {
                                    lines.iter().filter_map(|l| l.as_str()).collect::<String>()
                                })
                                .or_else(|| cell["source"].as_str().map(String::from))
                                .unwrap_or_default();
                            if !source.trim().is_empty() {
                                extracted.push_str(&format!(
                                    "# Cell {} [{}]\n{}\n\n",
                                    i + 1,
                                    cell_type,
                                    source
                                ));
                            }
                        }
                    }
                    let max = 50000;
                    let truncated = extracted.len() > max;
                    let text = if truncated {
                        &extracted[..max]
                    } else {
                        &extracted
                    };
                    ToolResult::ok(serde_json::json!({
                        "path": path, "format": "ipynb",
                        "content": text, "truncated": truncated,
                        "total_chars": extracted.len(), "backend": "native",
                    }))
                }
                Err(e) => ToolResult::err(format!("read failed: {e}")),
            },
            "pdf" => {
                // Fallback: poppler's pdftotext
                let output = tokio::process::Command::new("pdftotext")
                    .args([path, "-"])
                    .output()
                    .await;
                match output {
                    Ok(o) if o.status.success() => {
                        let text = String::from_utf8_lossy(&o.stdout).to_string();
                        ToolResult::ok(serde_json::json!({
                            "path": path, "format": "pdf", "content": text,
                            "chars": text.len(), "backend": "pdftotext",
                        }))
                    }
                    _ => ToolResult::err("PDF parsing failed (install pdftotext or start sidecar)"),
                }
            }
            "docx" => {
                // Fallback: pandoc
                let output = tokio::process::Command::new("pandoc")
                    .args(["-f", "docx", "-t", "plain", path])
                    .output()
                    .await;
                match output {
                    Ok(o) if o.status.success() => {
                        let text = String::from_utf8_lossy(&o.stdout).to_string();
                        ToolResult::ok(serde_json::json!({
                            "path": path, "format": "docx", "content": text,
                            "backend": "pandoc",
                        }))
                    }
                    _ => ToolResult::err("DOCX parsing failed (install pandoc or start sidecar)"),
                }
            }
            _ => ToolResult::err(format!("unsupported document format: {ext}")),
        }
    }
}

struct ParseCsv {
    sidecar: DocSidecar,
}

/// Parse CSV rows from raw text content (shared by the `path` and `csv_text`
/// code paths so both produce identical output shape).
fn parse_csv_rows(content: &str, max_rows: usize) -> serde_json::Value {
    let lines: Vec<&str> = content.lines().collect();
    let header = lines.first().copied().unwrap_or("");
    let columns: Vec<&str> = header.split(',').collect();
    let row_count = lines.len().saturating_sub(1);
    let sample_rows: Vec<Vec<&str>> = lines
        .iter()
        .skip(1)
        .take(max_rows)
        .map(|l| l.split(',').collect())
        .collect();

    serde_json::json!({
        "columns": columns,
        "column_count": columns.len(),
        "row_count": row_count,
        "sample_rows": sample_rows,
        "truncated": row_count > max_rows,
    })
}

#[async_trait]
impl ToolHandler for ParseCsv {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let max_rows = params["max_rows"].as_u64().unwrap_or(100) as usize;

        // BUG #5 FIX (category C: Tool Implementation issue). Root cause: this
        // tool only accepted a `path` parameter and ALWAYS called
        // `tokio::fs::read_to_string(path)` on whatever string it received —
        // there was no way to pass raw/inline CSV text. When the LLM (or a
        // caller) passed literal CSV content like "a,b,c\n1,2,3\n4,5,6" as
        // `path`, the OS reported "No such file or directory", surfaced
        // upstream as "unknown error". Add an explicit `csv_text` parameter
        // for inline content, alongside the existing `path` parameter for
        // file input, so BOTH are supported without breaking the file path
        // behavior any existing caller relies on.
        if let Some(csv_text) = params["csv_text"].as_str() {
            let result = parse_csv_rows(csv_text, max_rows);
            let mut result = result;
            result["format"] = serde_json::json!("csv");
            result["backend"] = serde_json::json!("inline");
            result["source"] = serde_json::json!("csv_text");
            return ToolResult::ok(result);
        }

        let path = params["path"].as_str().unwrap_or("");
        if path.is_empty() {
            return ToolResult::err(
                "missing required parameter: provide either 'path' (file) or 'csv_text' (inline CSV content)"
                    .to_string(),
            );
        }

        // Try sidecar for pandas analysis (schema detection, statistics)
        if let Some(result) = self.sidecar.try_extract(path, &["text", "tables"]).await {
            return ToolResult::ok(serde_json::json!({
                "path": path, "format": "csv", "backend": "sidecar",
                "result": result,
            }));
        }

        // Fallback: native CSV parsing with basic analysis
        match tokio::fs::read_to_string(path).await {
            Ok(content) => {
                let mut result = parse_csv_rows(&content, max_rows);
                result["path"] = serde_json::json!(path);
                result["format"] = serde_json::json!("csv");
                result["backend"] = serde_json::json!("native");
                ToolResult::ok(result)
            }
            Err(e) => ToolResult::err(format!("CSV read failed: {e}")),
        }
    }
}

struct SummarizeDocument {
    sidecar: DocSidecar,
}

#[async_trait]
impl ToolHandler for SummarizeDocument {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let path = params["path"].as_str().unwrap_or("");
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        // Try sidecar for structured summary (PDF/DOCX)
        if matches!(ext.as_str(), "pdf" | "docx") {
            if let Some(result) = self.sidecar.try_extract(path, &["text", "sections"]).await {
                // Produce a structured summary from sidecar output
                let text = result.get("text").and_then(|t| t.as_str()).unwrap_or("");
                let word_count = text.split_whitespace().count();
                let sections = result
                    .get("sections")
                    .cloned()
                    .unwrap_or(serde_json::json!([]));
                let preview: String = text.chars().take(500).collect();
                return ToolResult::ok(serde_json::json!({
                    "path": path, "format": ext, "backend": "sidecar",
                    "word_count": word_count,
                    "char_count": text.len(),
                    "sections": sections,
                    "preview": preview,
                }));
            }
        }

        // Fallback: basic text analysis
        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("read failed: {e}")),
        };
        let word_count = content.split_whitespace().count();
        let line_count = content.lines().count();
        let preview: String = content.chars().take(500).collect();
        ToolResult::ok(serde_json::json!({
            "path": path, "backend": "native",
            "word_count": word_count,
            "line_count": line_count,
            "char_count": content.len(),
            "preview": preview,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> ParseCsv {
        ParseCsv {
            sidecar: DocSidecar(None),
        }
    }

    /// BUG #5 regression (category C: Tool Implementation issue). Reproduces
    /// the exact real production failure: passing raw CSV text as if it were
    /// a value the tool could parse directly, instead of a filesystem path.
    #[tokio::test]
    async fn regr_bug5_parses_inline_csv_text_via_csv_text_param() {
        let result = tool()
            .execute(serde_json::json!({ "csv_text": "a,b,c\n1,2,3\n4,5,6" }))
            .await;
        assert!(result.success, "expected success, got: {:?}", result.error);
        assert_eq!(result.data["columns"], serde_json::json!(["a", "b", "c"]));
        assert_eq!(result.data["row_count"], 2);
        assert_eq!(result.data["backend"], "inline");
        assert_eq!(
            result.data["sample_rows"],
            serde_json::json!([["1", "2", "3"], ["4", "5", "6"]])
        );
    }

    /// Non-regression: the original file-path behavior must still work.
    #[tokio::test]
    async fn regr_bug5_still_parses_real_file_via_path_param() {
        let dir = std::env::temp_dir();
        let file_path = dir.join(format!("kria_parse_csv_test_{}.csv", uuid::Uuid::new_v4()));
        tokio::fs::write(&file_path, "x,y\n7,8\n9,10")
            .await
            .expect("write temp csv");

        let result = tool()
            .execute(serde_json::json!({ "path": file_path.to_string_lossy() }))
            .await;

        tokio::fs::remove_file(&file_path).await.ok();

        assert!(result.success, "expected success, got: {:?}", result.error);
        assert_eq!(result.data["backend"], "native");
        assert_eq!(result.data["columns"], serde_json::json!(["x", "y"]));
        assert_eq!(result.data["row_count"], 2);
    }

    /// BUG #5 regression: neither `path` nor `csv_text` supplied must produce
    /// a clear, actionable error instead of a confusing OS-level failure.
    #[tokio::test]
    async fn regr_bug5_missing_both_params_gives_clear_error() {
        let result = tool().execute(serde_json::json!({})).await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(
            err.contains("path") && err.contains("csv_text"),
            "error should mention both options: {err}"
        );
    }

    /// BUG #5 regression: passing literal CSV text via the OLD `path` field
    /// (the exact original bug's mistaken-usage pattern) must still fail
    /// clearly rather than silently succeeding with garbage data — the real
    /// fix is that a caller/LLM should now use `csv_text` instead, not that
    /// `path` should quietly start accepting non-path content.
    #[tokio::test]
    async fn regr_bug5_literal_csv_text_via_path_still_fails_clearly() {
        let result = tool()
            .execute(serde_json::json!({ "path": "a,b,c\n1,2,3\n4,5,6" }))
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("CSV read failed"));
    }
}

pub fn register(reg: &ToolRegistry) {
    register_with_sidecar(reg, None);
}

pub fn register_with_sidecar(reg: &ToolRegistry, sidecar: Option<Arc<SidecarBridge>>) {
    let doc_sc = DocSidecar(sidecar.map(|s| Arc::new(Mutex::new(s))));

    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "parse_document".into(),
                description: "Read, analyze, extract, or summarize any file given its path. \
                    Use when the user mentions a file path and asks to analyze, read, open, \
                    or understand its contents. Supports PDF, DOCX, CSV, TXT, MD, JSON, YAML, \
                    TOML, XML and more. Also use when user says 'what does this file say', \
                    'summarize this document', or pastes a file path with an analysis request."
                    .into(),
                category: "documents".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param("path", "string", "Document file path", true)],
            },
            Arc::new(ParseDocument {
                sidecar: doc_sc.clone(),
            }),
        ),
        (
            ToolDef {
                name: "parse_csv".into(),
                description: "Parse CSV with column detection and sample rows. Provide EITHER 'path' (a CSV file on disk) OR 'csv_text' (raw/inline CSV content pasted in the request) — not both required.".into(),
                category: "documents".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("path", "string", "CSV file path (use this OR csv_text)", false),
                    param(
                        "csv_text",
                        "string",
                        "Raw/inline CSV text content, e.g. \"a,b\\n1,2\" (use this OR path)",
                        false,
                    ),
                    param(
                        "max_rows",
                        "integer",
                        "Max sample rows (default 100)",
                        false,
                    ),
                ],
            },
            Arc::new(ParseCsv {
                sidecar: doc_sc.clone(),
            }),
        ),
        (
            ToolDef {
                name: "summarize_document".into(),
                description: "Get document statistics, sections, and preview".into(),
                category: "documents".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param("path", "string", "File path", true)],
            },
            Arc::new(SummarizeDocument { sidecar: doc_sc }),
        ),
    ];
    for (def, handler) in tools {
        reg.register(def, handler);
    }
}
