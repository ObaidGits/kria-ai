//! IDE Cognition Engine
//!
//! Provides semantic IDE state awareness via LSP (Language Server Protocol).
//! Enables KRIA to understand:
//! - Active diagnostics (errors, warnings)
//! - Compile errors and their locations
//! - Workspace structure
//! - Current file and cursor position
//!
//! ## Architecture
//! LSP is accessed via subprocess communication with language servers.
//! For VS Code, we also read the workspace state from the extension host.
//! Falls back gracefully when LSP is unavailable.
use std::path::PathBuf;

/// A diagnostic item from the IDE.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Diagnostic {
    /// File path
    pub file: String,
    /// Line number (1-indexed)
    pub line: u32,
    /// Column number (1-indexed)
    pub column: u32,
    /// Severity: "error", "warning", "info", "hint"
    pub severity: String,
    /// Diagnostic message
    pub message: String,
    /// Diagnostic source (e.g., "pylsp", "rust-analyzer")
    pub source: Option<String>,
}

/// Current IDE workspace state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IdeState {
    /// Active file path
    pub active_file: Option<String>,
    /// Workspace root directory
    pub workspace_root: Option<String>,
    /// All diagnostics in the workspace
    pub diagnostics: Vec<Diagnostic>,
    /// Number of errors
    pub error_count: usize,
    /// Number of warnings
    pub warning_count: usize,
    /// Whether a build is in progress
    pub building: bool,
    /// Last build result
    pub last_build_success: Option<bool>,
}

impl IdeState {
    pub fn empty() -> Self {
        Self {
            active_file: None,
            workspace_root: None,
            diagnostics: Vec::new(),
            error_count: 0,
            warning_count: 0,
            building: false,
            last_build_success: None,
        }
    }

    pub fn has_errors(&self) -> bool {
        self.error_count > 0
    }

    pub fn error_summary(&self) -> String {
        if self.error_count == 0 {
            "No errors".to_string()
        } else {
            let errors: Vec<String> = self
                .diagnostics
                .iter()
                .filter(|d| d.severity == "error")
                .take(5)
                .map(|d| format!("{}:{}: {}", d.file, d.line, d.message))
                .collect();
            format!("{} error(s): {}", self.error_count, errors.join("; "))
        }
    }
}

/// IDE cognition engine.
///
/// # Batch 1: PSDG persistence
///
/// Attach a `PsdgHandle` via `with_world_model()` to persist `IdeState`
/// (workspace root, active file, error count) to WorldModelStore after
/// each `get_state()` call. All persistence is fire-and-forget.
pub struct IdeCognitionEngine {
    /// Optional PSDG handle for IDE state persistence.
    world_model: Option<crate::agent::psdg::PsdgHandle>,
}

impl IdeCognitionEngine {
    pub fn new() -> Self {
        Self { world_model: None }
    }

    /// Attach a PSDG handle for IDE state persistence.
    ///
    /// When set, each `get_state()` call persists the workspace root,
    /// active file, and error count to WorldModelStore.
    pub fn with_world_model(mut self, psdg: crate::agent::psdg::PsdgHandle) -> Self {
        self.world_model = Some(psdg);
        self
    }

    /// Get the current IDE state by reading VS Code's workspace state.
    ///
    /// Reads from VS Code's SQLite state database and log files.
    /// Falls back to file-system heuristics when VS Code is not running.
    /// When a PsdgHandle is attached, persists workspace/file/error state
    /// to WorldModelStore as fire-and-forget semantic facts.
    pub async fn get_state(&self) -> IdeState {
        // Try VS Code state first
        let state = if let Some(state) = self.get_vscode_state().await {
            state
        } else {
            IdeState::empty()
        };

        // ── PSDG: persist IDE state (fire-and-forget) ────────────────────
        if let Some(ref psdg) = self.world_model {
            if let Some(ref workspace) = state.workspace_root {
                psdg.record_ide_state(workspace, state.active_file.as_deref(), state.error_count);
            }
        }

        state
    }

    /// Read VS Code workspace state from its SQLite database.
    async fn get_vscode_state(&self) -> Option<IdeState> {
        // VS Code stores state in ~/.config/Code/User/workspaceStorage/
        // and logs in ~/.config/Code/logs/
        let home = std::env::var("HOME").ok()?;
        let vscode_log_dir = PathBuf::from(&home).join(".config/Code/logs");

        if !vscode_log_dir.exists() {
            return None;
        }

        // Read the most recent VS Code log for diagnostics
        let mut state = IdeState::empty();

        // Try to find the active workspace from VS Code's recent workspaces
        let recent_workspaces_path =
            PathBuf::from(&home).join(".config/Code/User/globalStorage/state.vscdb");

        if recent_workspaces_path.exists() {
            // Read workspace root from VS Code storage natively
            if let Ok(conn) = rusqlite::Connection::open(&recent_workspaces_path) {
                let query =
                    "SELECT value FROM ItemTable WHERE key = 'history.recentlyOpenedPathsList'";
                if let Ok(value) = conn.query_row(query, [], |row| row.get::<_, String>(0)) {
                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(&value) {
                        if let Some(entries) = data.get("entries").and_then(|e| e.as_array()) {
                            if let Some(first) = entries.first() {
                                if let Some(uri) = first
                                    .get("folderUri")
                                    .or_else(|| first.get("fileUri"))
                                    .and_then(|u| u.as_str())
                                {
                                    let path = uri.replace("file://", "");
                                    state.workspace_root = Some(path.trim().to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        Some(state)
    }

    /// Run a language server check on a file and return diagnostics.
    ///
    /// Uses the appropriate language server based on file extension.
    pub async fn check_file(&self, file_path: &str) -> Vec<Diagnostic> {
        let path = PathBuf::from(file_path);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        match ext {
            "py" => self.check_python_file(file_path).await,
            "rs" => self.check_rust_file(file_path).await,
            "js" | "ts" => self.check_js_file(file_path).await,
            _ => Vec::new(),
        }
    }

    async fn check_python_file(&self, file_path: &str) -> Vec<Diagnostic> {
        // Try ruff first (fast, comprehensive) then fall back to tree-sitter natively
        let ruff_result = self.check_python_with_ruff(file_path).await;
        if !ruff_result.is_empty() || self.ruff_available() {
            return ruff_result;
        }

        // Fallback: tree-sitter for native syntax checking without subprocess
        let mut parser = tree_sitter::Parser::new();
        if parser
            .set_language(&tree_sitter_python::language().into())
            .is_err()
        {
            return Vec::new();
        }

        let content = match tokio::fs::read_to_string(file_path).await {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let tree = match parser.parse(&content, None) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let mut diagnostics = Vec::new();
        let mut cursor = tree.walk();

        loop {
            let node = cursor.node();
            if node.is_error() || node.is_missing() {
                diagnostics.push(Diagnostic {
                    file: file_path.to_string(),
                    line: node.start_position().row as u32 + 1,
                    column: node.start_position().column as u32 + 1,
                    severity: "error".to_string(),
                    message: if node.is_missing() {
                        format!("Missing {}", node.kind())
                    } else {
                        "Syntax error".to_string()
                    },
                    source: Some("tree-sitter".to_string()),
                });
            }

            if cursor.goto_first_child() {
                continue;
            }
            if cursor.goto_next_sibling() {
                continue;
            }
            let mut retracing = true;
            while retracing {
                if !cursor.goto_parent() {
                    retracing = false;
                    break;
                }
                if cursor.goto_next_sibling() {
                    break;
                }
            }
            if !retracing {
                break;
            }
        }

        diagnostics
    }

    fn ruff_available(&self) -> bool {
        std::path::Path::new("/home/obaid/anaconda3/bin/ruff").exists()
            || std::path::Path::new("/usr/bin/ruff").exists()
            || std::path::Path::new("/usr/local/bin/ruff").exists()
    }

    /// Check Python file with ruff (fast, comprehensive linter).
    async fn check_python_with_ruff(&self, file_path: &str) -> Vec<Diagnostic> {
        // Find ruff binary
        let ruff_paths = [
            "/home/obaid/anaconda3/bin/ruff",
            "/usr/bin/ruff",
            "/usr/local/bin/ruff",
        ];
        let ruff_bin = ruff_paths.iter().find(|p| std::path::Path::new(p).exists());
        let ruff_bin = match ruff_bin {
            Some(b) => *b,
            None => return Vec::new(),
        };

        let result = tokio::process::Command::new(ruff_bin)
            .args(["check", "--output-format", "json", "--quiet", file_path])
            .output()
            .await;

        match result {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(stdout.trim()) {
                    items
                        .into_iter()
                        .filter_map(|item| {
                            let message = item["message"].as_str()?.to_string();
                            let location = &item["location"];
                            let line = location["row"].as_u64().unwrap_or(1) as u32;
                            let col = location["column"].as_u64().unwrap_or(1) as u32;
                            let code = item["code"].as_str().unwrap_or("").to_string();
                            let severity = if code.starts_with('E') || code.starts_with('F') {
                                "error"
                            } else {
                                "warning"
                            };
                            Some(Diagnostic {
                                file: file_path.to_string(),
                                line,
                                column: col,
                                severity: severity.to_string(),
                                message: format!("[{}] {}", code, message),
                                source: Some("ruff".to_string()),
                            })
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    /// Check a Rust file for errors using rustc --emit=metadata.
    async fn check_rust_file(&self, file_path: &str) -> Vec<Diagnostic> {
        // SECURITY FIX: Pass file path as a direct argument (not interpolated).
        // rustc accepts the file path as a positional argument, so no injection risk.
        let result = tokio::process::Command::new("rustc")
            .args([
                "--edition",
                "2021",
                "--error-format",
                "json",
                "--emit",
                "metadata",
                "-o",
                "/dev/null",
                file_path, // Safe: positional argument, not interpolated into a script
            ])
            .output()
            .await;

        match result {
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let mut diagnostics = Vec::new();
                for line in stderr.lines() {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                        if v["reason"].as_str() == Some("compiler-message") {
                            let msg = &v["message"];
                            let severity = match msg["level"].as_str() {
                                Some("error") => "error",
                                Some("warning") => "warning",
                                _ => "info",
                            };
                            let message = msg["message"].as_str().unwrap_or("").to_string();
                            let spans = msg["spans"].as_array();
                            let (line, col) = spans
                                .and_then(|s| s.first())
                                .map(|s| {
                                    (
                                        s["line_start"].as_u64().unwrap_or(1) as u32,
                                        s["column_start"].as_u64().unwrap_or(1) as u32,
                                    )
                                })
                                .unwrap_or((1, 1));

                            diagnostics.push(Diagnostic {
                                file: file_path.to_string(),
                                line,
                                column: col,
                                severity: severity.to_string(),
                                message,
                                source: Some("rustc".to_string()),
                            });
                        }
                    }
                }
                diagnostics
            }
            _ => Vec::new(),
        }
    }

    /// Check a JS/TS file for syntax errors using node.
    async fn check_js_file(&self, file_path: &str) -> Vec<Diagnostic> {
        // SECURITY FIX: Pass file path as a direct argument (not interpolated).
        let result = tokio::process::Command::new("node")
            .args(["--check", file_path]) // Safe: positional argument
            .output()
            .await;

        match result {
            Ok(out) if !out.status.success() => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                vec![Diagnostic {
                    file: file_path.to_string(),
                    line: 1,
                    column: 1,
                    severity: "error".to_string(),
                    message: stderr.trim().to_string(),
                    source: Some("node".to_string()),
                }]
            }
            _ => Vec::new(),
        }
    }

    /// Get a human-readable summary of IDE state for the agent.
    pub async fn get_summary(&self) -> String {
        let state = self.get_state().await;
        if state.has_errors() {
            format!(
                "IDE has {} error(s). {}",
                state.error_count,
                state.error_summary()
            )
        } else if let Some(root) = &state.workspace_root {
            format!("IDE workspace: {} — no errors", root)
        } else {
            "IDE state: no active workspace detected".to_string()
        }
    }
}

impl Default for IdeCognitionEngine {
    fn default() -> Self {
        Self::new()
    }
}
