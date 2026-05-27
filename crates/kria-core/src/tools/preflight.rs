//! Preflight validation for dangerous tool calls.
//!
//! Runs BEFORE `run_isolated` to catch parameter errors early and prevent
//! obviously dangerous operations from reaching the execution layer.
//!
//! # Design Principles
//! - Fail fast: reject bad inputs before any subprocess is spawned
//! - Deterministic: same input always produces same result
//! - Bounded: no I/O, no network calls, no LLM calls
//! - Composable: each validator is independent and testable
//! - Non-blocking: all checks are synchronous (no async needed)
//!
//! # What Preflight Does NOT Do
//! - It does NOT replace the PolicyEngine (risk classification)
//! - It does NOT replace HITL (human approval)
//! - It does NOT sandbox execution
//! - It does NOT validate business logic
//!
//! Preflight catches: malformed arguments, obviously destructive patterns,
//! missing required parameters, and structurally invalid inputs.

use std::path::Path;

// ─── Result Types ────────────────────────────────────────────────────────────

/// Result of a preflight validation check.
#[derive(Debug, Clone)]
pub struct PreflightResult {
    /// Whether execution is allowed to proceed.
    pub allowed: bool,
    /// Non-fatal warnings (execution proceeds but caller is informed).
    pub warnings: Vec<String>,
    /// Reason for blocking (only set when `allowed = false`).
    pub blocked_reason: Option<String>,
}

impl PreflightResult {
    /// Execution allowed, no warnings.
    pub fn ok() -> Self {
        Self {
            allowed: true,
            warnings: Vec::new(),
            blocked_reason: None,
        }
    }

    /// Execution allowed with a warning.
    pub fn warn(msg: impl Into<String>) -> Self {
        Self {
            allowed: true,
            warnings: vec![msg.into()],
            blocked_reason: None,
        }
    }

    /// Execution blocked.
    pub fn block(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            warnings: Vec::new(),
            blocked_reason: Some(reason.into()),
        }
    }

    /// Merge another result into this one (most restrictive wins).
    pub fn merge(mut self, other: PreflightResult) -> Self {
        if !other.allowed {
            self.allowed = false;
            if let Some(reason) = other.blocked_reason {
                self.blocked_reason = Some(reason);
            }
        }
        self.warnings.extend(other.warnings);
        self
    }
}

// ─── Shell Tokenizer ─────────────────────────────────────────────────────────

/// Shell-aware tokenizer. Splits on whitespace but respects single/double quotes
/// and backslash escaping. Does NOT expand variables or globs.
///
/// This is intentionally simple — we want to analyze the literal command,
/// not what the shell would produce after expansion.
pub fn shell_tokenize(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escape_next = false;

    for ch in command.chars() {
        if escape_next {
            current.push(ch);
            escape_next = false;
            continue;
        }
        match ch {
            '\\' if !in_single_quote => {
                escape_next = true;
            }
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
            }
            ' ' | '\t' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            '|' | ';' | '&' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(ch.to_string());
            }
            _ => {
                current.push(ch);
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

// ─── Shell Validation ────────────────────────────────────────────────────────

/// Validate a shell command before execution.
///
/// Uses shell-aware tokenization to prevent bypass via quoting or escaping.
/// Shell expansion (`$(...)`, backticks, `${...}`) is detected and flagged
/// as a warning — we cannot statically analyze expanded commands.
pub fn preflight_shell(command: &str) -> PreflightResult {
    if command.trim().is_empty() {
        return PreflightResult::block("empty command");
    }

    // Detect shell expansion — static analysis is limited for these
    let has_expansion = command.contains("$(")
        || command.contains('`')
        || command.contains("${")
        || command.to_ascii_lowercase().contains("eval ");

    if has_expansion {
        // Cannot statically analyze expanded commands — warn but allow
        // (PolicyEngine handles risk classification)
        return PreflightResult::warn(
            "Command uses shell expansion — static preflight analysis is limited. \
             PolicyEngine will assess risk level.",
        );
    }

    let tokens = shell_tokenize(command);
    if tokens.is_empty() {
        return PreflightResult::block("command tokenized to empty");
    }

    let first_cmd = tokens.first().map(|s| s.as_str()).unwrap_or("");

    // ── Block: recursive deletion of root or critical system paths ──────────
    if first_cmd == "rm" || tokens.contains(&"rm".to_string()) {
        let has_recursive = tokens.iter().any(|t| t.starts_with('-') && t.contains('r'));
        let has_force = tokens.iter().any(|t| t.starts_with('-') && t.contains('f'));
        let targets_root = tokens
            .iter()
            .any(|t| *t == "/" || *t == "/*" || t.starts_with("/*"));
        let targets_critical = tokens.iter().any(|t| {
            [
                "/boot", "/usr", "/bin", "/sbin", "/lib", "/proc", "/sys", "/etc",
            ]
            .iter()
            .any(|critical| t.starts_with(critical) && t.len() <= critical.len() + 1)
        });

        if has_recursive && (targets_root || targets_critical) {
            return PreflightResult::block(format!(
                "Recursive deletion of critical system path blocked. \
                 Targets: {}",
                tokens
                    .iter()
                    .filter(|t| t.starts_with('/'))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        if has_recursive && has_force {
            return PreflightResult::warn(
                "rm -rf detected — verify target path is correct before execution",
            );
        }
    }

    // ── Block: direct disk writes ────────────────────────────────────────────
    if first_cmd == "dd" {
        let targets_device = tokens.iter().any(|t| t.starts_with("of=/dev/"));
        if targets_device {
            return PreflightResult::block(
                "Direct disk write via dd blocked by preflight. \
                 Use file-level operations instead.",
            );
        }
    }

    // ── Block: filesystem format ─────────────────────────────────────────────
    if first_cmd.starts_with("mkfs") || (first_cmd == "format" || first_cmd == "mkformat") {
        let targets_device = tokens.iter().any(|t| t.starts_with("/dev/"));
        if targets_device {
            return PreflightResult::block("Filesystem format command blocked by preflight.");
        }
    }

    // ── Warnings ─────────────────────────────────────────────────────────────
    let mut warnings = Vec::new();

    if first_cmd == "sudo" || tokens.contains(&"sudo".to_string()) {
        warnings.push("Command uses sudo — will require elevated privileges".to_string());
    }

    // Piping remote content to shell
    let has_pipe = tokens.contains(&"|".to_string()) || command.contains(" | ");
    if has_pipe {
        let has_curl_wget = tokens.iter().any(|t| t == "curl" || t == "wget");
        let has_shell_interp = tokens
            .iter()
            .any(|t| t == "sh" || t == "bash" || t == "zsh" || t == "fish");
        if has_curl_wget && has_shell_interp {
            warnings.push(
                "Piping remote content to shell interpreter — potential security risk".to_string(),
            );
        }
    }

    // Writing to system config
    let writes_to_etc = (command.contains("> /etc/") || command.contains(">> /etc/"))
        || (first_cmd == "tee" && tokens.iter().any(|t| t.starts_with("/etc/")));
    if writes_to_etc {
        warnings.push("Writing to /etc/ — system configuration change".to_string());
    }

    PreflightResult {
        allowed: true,
        warnings,
        blocked_reason: None,
    }
}

// ─── Filesystem Validation ───────────────────────────────────────────────────

/// Validate a filesystem operation before execution.
pub fn preflight_file_op(operation: &str, path: &str) -> PreflightResult {
    if path.trim().is_empty() {
        return PreflightResult::block("empty file path");
    }

    let p = Path::new(path);

    // Block write/delete on critical system paths
    let critical_prefixes = [
        "/boot",
        "/usr/bin",
        "/usr/lib",
        "/usr/sbin",
        "/sbin",
        "/bin",
        "/proc",
        "/sys",
        "/dev",
    ];

    let is_destructive = matches!(operation, "delete" | "write" | "overwrite" | "move");

    if is_destructive {
        for critical in &critical_prefixes {
            if path.starts_with(critical) {
                return PreflightResult::block(format!(
                    "Write/delete to critical system path '{}' blocked",
                    critical
                ));
            }
        }
    }

    // Warn on dotfiles in home directory
    if is_destructive {
        let path_str = p.to_string_lossy();
        if (path_str.contains("/.") || path_str.starts_with("~/.") || path_str.starts_with("."))
            && !path_str.contains("/.kria/")
        {
            return PreflightResult::warn(format!("Modifying dotfile or hidden path: {}", path));
        }
    }

    // Warn on paths outside home directory for write operations
    if is_destructive && !path.starts_with('/') {
        // Relative path — generally safe
    }

    PreflightResult::ok()
}

// ─── Network Validation ──────────────────────────────────────────────────────

/// Validate a network operation before execution.
pub fn preflight_network(url: &str) -> PreflightResult {
    if url.trim().is_empty() {
        return PreflightResult::block("empty URL");
    }

    // Block cloud metadata endpoints (SSRF prevention)
    let lower = url.to_ascii_lowercase();
    if lower.contains("169.254.169.254")
        || lower.contains("metadata.google.internal")
        || lower.contains("metadata.aws")
        || lower.contains("169.254.170.2")
    {
        return PreflightResult::block(
            "Access to cloud metadata endpoint blocked (SSRF prevention)",
        );
    }

    // Block file:// protocol in network operations
    if lower.starts_with("file://") {
        return PreflightResult::block("file:// protocol not allowed in network operations");
    }

    // Block javascript: and data: URIs
    if lower.starts_with("javascript:") || lower.starts_with("data:") {
        return PreflightResult::block(format!(
            "Protocol '{}' not allowed in network operations",
            url.split(':').next().unwrap_or("unknown")
        ));
    }

    PreflightResult::ok()
}

// ─── Argument Validation ─────────────────────────────────────────────────────

/// Validate that required parameters are present and non-empty.
///
/// Returns a block result if any required parameter is missing or empty.
pub fn preflight_required_params(
    tool_name: &str,
    params: &serde_json::Value,
    required: &[&str],
) -> PreflightResult {
    let obj = match params.as_object() {
        Some(o) => o,
        None => {
            return PreflightResult::block(format!(
                "Tool '{}': parameters must be a JSON object, got {}",
                tool_name, params
            ));
        }
    };

    for &param in required {
        match obj.get(param) {
            None => {
                return PreflightResult::block(format!(
                    "Tool '{}': required parameter '{}' is missing",
                    tool_name, param
                ));
            }
            Some(v) if v.is_null() => {
                return PreflightResult::block(format!(
                    "Tool '{}': required parameter '{}' is null",
                    tool_name, param
                ));
            }
            Some(serde_json::Value::String(s)) if s.trim().is_empty() => {
                return PreflightResult::block(format!(
                    "Tool '{}': required parameter '{}' is empty",
                    tool_name, param
                ));
            }
            _ => {}
        }
    }

    PreflightResult::ok()
}

// ─── Dispatch ────────────────────────────────────────────────────────────────

/// Run preflight validation for a tool call.
///
/// Dispatches to the appropriate validator based on tool name.
/// Returns `PreflightResult::ok()` for tools with no specific validation.
///
/// This is the main entry point called by the agent loop before `run_isolated`.
pub fn run_preflight(tool_name: &str, params: &serde_json::Value) -> PreflightResult {
    match tool_name {
        // Shell execution tools
        "execute_bash" | "run_shell_command" | "execute_command" => {
            let command = params.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let required = preflight_required_params(tool_name, params, &["command"]);
            if !required.allowed {
                return required;
            }
            preflight_shell(command)
        }

        "execute_python" => preflight_required_params(tool_name, params, &["code"]),

        "execute_powershell" => {
            let command = params.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let required = preflight_required_params(tool_name, params, &["command"]);
            if !required.allowed {
                return required;
            }
            // PowerShell-specific: warn on Invoke-Expression
            if command.to_ascii_lowercase().contains("invoke-expression")
                || command.to_ascii_lowercase().contains("iex ")
            {
                return PreflightResult::warn(
                    "PowerShell Invoke-Expression detected — potential code injection risk",
                );
            }
            PreflightResult::ok()
        }

        // File operation tools
        "write_file" | "create_file" | "overwrite_file" => {
            let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let required = preflight_required_params(tool_name, params, &["path"]);
            if !required.allowed {
                return required;
            }
            preflight_file_op("write", path)
        }

        "delete_file" | "remove_file" => {
            let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let required = preflight_required_params(tool_name, params, &["path"]);
            if !required.allowed {
                return required;
            }
            preflight_file_op("delete", path)
        }

        "move_file" | "rename_file" => {
            let src = params
                .get("source")
                .or_else(|| params.get("src"))
                .or_else(|| params.get("from"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let dst = params
                .get("destination")
                .or_else(|| params.get("dst"))
                .or_else(|| params.get("to"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            preflight_file_op("move", src).merge(preflight_file_op("write", dst))
        }

        // Network tools
        "fetch_url" | "fetch_article" | "download_file" => {
            let url = params.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let required = preflight_required_params(tool_name, params, &["url"]);
            if !required.allowed {
                return required;
            }
            preflight_network(url)
        }

        "web_search" | "searxng_search" => preflight_required_params(tool_name, params, &["query"]),

        // All other tools: no specific preflight
        _ => PreflightResult::ok(),
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Shell tokenizer ──────────────────────────────────────────────────────

    #[test]
    fn tokenize_simple_command() {
        let tokens = shell_tokenize("ls -la /tmp");
        assert_eq!(tokens, vec!["ls", "-la", "/tmp"]);
    }

    #[test]
    fn tokenize_quoted_argument() {
        let tokens = shell_tokenize(r#"echo "hello world""#);
        assert_eq!(tokens, vec!["echo", "hello world"]);
    }

    #[test]
    fn tokenize_single_quoted() {
        let tokens = shell_tokenize("echo 'hello world'");
        assert_eq!(tokens, vec!["echo", "hello world"]);
    }

    #[test]
    fn tokenize_pipe_operator() {
        let tokens = shell_tokenize("ls | grep foo");
        assert_eq!(tokens, vec!["ls", "|", "grep", "foo"]);
    }

    #[test]
    fn tokenize_escaped_space() {
        let tokens = shell_tokenize(r"echo hello\ world");
        assert_eq!(tokens, vec!["echo", "hello world"]);
    }

    // ── Shell validation ─────────────────────────────────────────────────────

    #[test]
    fn shell_rm_rf_root_is_blocked() {
        let result = preflight_shell("rm -rf /");
        assert!(!result.allowed);
        assert!(result.blocked_reason.is_some());
    }

    #[test]
    fn shell_rm_rf_root_wildcard_is_blocked() {
        let result = preflight_shell("rm -rf /*");
        assert!(!result.allowed);
    }

    #[test]
    fn shell_rm_rf_boot_is_blocked() {
        let result = preflight_shell("rm -rf /boot");
        assert!(!result.allowed);
    }

    #[test]
    fn shell_rm_rf_home_is_warned_not_blocked() {
        let result = preflight_shell("rm -rf /home/user/projects");
        // /home is not in critical_prefixes — should be warned, not blocked
        assert!(result.allowed);
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn shell_dd_to_dev_is_blocked() {
        let result = preflight_shell("dd if=/dev/zero of=/dev/sda");
        assert!(!result.allowed);
        assert!(result
            .blocked_reason
            .as_deref()
            .unwrap_or("")
            .contains("dd"));
    }

    #[test]
    fn shell_mkfs_is_blocked() {
        let result = preflight_shell("mkfs.ext4 /dev/sdb1");
        assert!(!result.allowed);
    }

    #[test]
    fn shell_sudo_is_warned() {
        let result = preflight_shell("sudo apt update");
        assert!(result.allowed);
        assert!(result.warnings.iter().any(|w| w.contains("sudo")));
    }

    #[test]
    fn shell_curl_pipe_bash_is_warned() {
        let result = preflight_shell("curl https://example.com/install.sh | bash");
        assert!(result.allowed);
        assert!(result.warnings.iter().any(|w| w.contains("remote content")));
    }

    #[test]
    fn shell_expansion_is_warned_not_blocked() {
        let result = preflight_shell("echo $(whoami)");
        assert!(result.allowed);
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn shell_safe_command_passes() {
        let result = preflight_shell("ls -la /home/user");
        assert!(result.allowed);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn shell_empty_command_is_blocked() {
        let result = preflight_shell("");
        assert!(!result.allowed);
    }

    // ── Filesystem validation ────────────────────────────────────────────────

    #[test]
    fn file_write_to_usr_bin_is_blocked() {
        let result = preflight_file_op("write", "/usr/bin/evil");
        assert!(!result.allowed);
    }

    #[test]
    fn file_delete_proc_is_blocked() {
        let result = preflight_file_op("delete", "/proc/1/mem");
        assert!(!result.allowed);
    }

    #[test]
    fn file_write_to_home_is_ok() {
        let result = preflight_file_op("write", "/home/user/document.txt");
        assert!(result.allowed);
    }

    #[test]
    fn file_read_from_etc_is_ok() {
        // Read is not destructive — should pass
        let result = preflight_file_op("read", "/etc/hosts");
        assert!(result.allowed);
    }

    #[test]
    fn file_empty_path_is_blocked() {
        let result = preflight_file_op("write", "");
        assert!(!result.allowed);
    }

    // ── Network validation ───────────────────────────────────────────────────

    #[test]
    fn network_metadata_endpoint_is_blocked() {
        let result = preflight_network("http://169.254.169.254/latest/meta-data/");
        assert!(!result.allowed);
    }

    #[test]
    fn network_file_protocol_is_blocked() {
        let result = preflight_network("file:///etc/passwd");
        assert!(!result.allowed);
    }

    #[test]
    fn network_javascript_is_blocked() {
        let result = preflight_network("javascript:alert(1)");
        assert!(!result.allowed);
    }

    #[test]
    fn network_https_is_ok() {
        let result = preflight_network("https://example.com/api");
        assert!(result.allowed);
    }

    #[test]
    fn network_empty_url_is_blocked() {
        let result = preflight_network("");
        assert!(!result.allowed);
    }

    // ── Required params validation ───────────────────────────────────────────

    #[test]
    fn missing_required_param_is_blocked() {
        let params = serde_json::json!({});
        let result = preflight_required_params("web_search", &params, &["query"]);
        assert!(!result.allowed);
        assert!(result
            .blocked_reason
            .as_deref()
            .unwrap_or("")
            .contains("query"));
    }

    #[test]
    fn null_required_param_is_blocked() {
        let params = serde_json::json!({"query": null});
        let result = preflight_required_params("web_search", &params, &["query"]);
        assert!(!result.allowed);
    }

    #[test]
    fn empty_string_required_param_is_blocked() {
        let params = serde_json::json!({"query": "  "});
        let result = preflight_required_params("web_search", &params, &["query"]);
        assert!(!result.allowed);
    }

    #[test]
    fn valid_required_param_passes() {
        let params = serde_json::json!({"query": "rust programming"});
        let result = preflight_required_params("web_search", &params, &["query"]);
        assert!(result.allowed);
    }

    #[test]
    fn non_object_params_is_blocked() {
        let params = serde_json::json!("not an object");
        let result = preflight_required_params("some_tool", &params, &["field"]);
        assert!(!result.allowed);
    }

    // ── Dispatch ─────────────────────────────────────────────────────────────

    #[test]
    fn dispatch_execute_bash_validates_command() {
        let params = serde_json::json!({"command": "rm -rf /"});
        let result = run_preflight("execute_bash", &params);
        assert!(!result.allowed);
    }

    #[test]
    fn dispatch_execute_bash_missing_command_blocked() {
        let params = serde_json::json!({});
        let result = run_preflight("execute_bash", &params);
        assert!(!result.allowed);
    }

    #[test]
    fn dispatch_fetch_url_validates_url() {
        let params = serde_json::json!({"url": "http://169.254.169.254/"});
        let result = run_preflight("fetch_url", &params);
        assert!(!result.allowed);
    }

    #[test]
    fn dispatch_web_search_requires_query() {
        let params = serde_json::json!({});
        let result = run_preflight("web_search", &params);
        assert!(!result.allowed);
    }

    #[test]
    fn dispatch_unknown_tool_passes() {
        let params = serde_json::json!({"anything": "value"});
        let result = run_preflight("get_cpu_usage", &params);
        assert!(result.allowed);
    }

    #[test]
    fn dispatch_delete_file_validates_path() {
        let params = serde_json::json!({"path": "/usr/bin/ls"});
        let result = run_preflight("delete_file", &params);
        assert!(!result.allowed);
    }

    // ── Merge ────────────────────────────────────────────────────────────────

    #[test]
    fn merge_ok_with_ok_is_ok() {
        let result = PreflightResult::ok().merge(PreflightResult::ok());
        assert!(result.allowed);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn merge_ok_with_block_is_blocked() {
        let result = PreflightResult::ok().merge(PreflightResult::block("bad"));
        assert!(!result.allowed);
    }

    #[test]
    fn merge_warns_accumulate() {
        let result = PreflightResult::warn("w1").merge(PreflightResult::warn("w2"));
        assert!(result.allowed);
        assert_eq!(result.warnings.len(), 2);
    }

    // ── Determinism ──────────────────────────────────────────────────────────

    #[test]
    fn preflight_is_deterministic() {
        let command = "rm -rf /";
        let r1 = preflight_shell(command);
        let r2 = preflight_shell(command);
        assert_eq!(r1.allowed, r2.allowed);
        assert_eq!(r1.blocked_reason, r2.blocked_reason);
    }
}
