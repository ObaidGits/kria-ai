use crate::infra::environment::{CommandRequest, CommandResult, EnvironmentError, ShellState};
use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::ToolContext;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 300;
const MAX_OUTPUT_BYTES: usize = 100 * 1024;
const MAX_OUTPUT_LINES: usize = 10_000;

fn default_timeout_secs() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExecuteBashInput {
    command: String,
    #[serde(default = "default_timeout_secs")]
    timeout: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExecutePythonInput {
    code: String,
    #[serde(default = "default_timeout_secs")]
    timeout: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExecutePowershellInput {
    command: String,
    #[serde(default = "default_timeout_secs")]
    timeout: u64,
}

enum PersistedBuiltin {
    Cd(String),
    Export { key: String, value: String },
    Unset(String),
}

fn parse_input<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, ToolResult> {
    serde_json::from_value(params).map_err(|error| ToolResult::err(format!("invalid parameters: {error}")))
}

fn resolve_timeout_ms(timeout_secs: u64) -> Result<u64, ToolResult> {
    if timeout_secs == 0 || timeout_secs > MAX_TIMEOUT_SECS {
        return Err(ToolResult::err(format!(
            "timeout must be between 1 and {MAX_TIMEOUT_SECS} seconds"
        )));
    }

    Ok(timeout_secs.saturating_mul(1_000))
}

fn command_success(output: CommandResult) -> ToolResult {
    ToolResult::ok(serde_json::json!({
        "exit_code": output.exit_code,
        "stdout": output.stdout,
        "stderr": output.stderr,
        "truncated": output.truncated,
    }))
}

fn env_error_to_tool_result(error: EnvironmentError) -> ToolResult {
    match error {
        EnvironmentError::CommandFailed { exit_code, stderr } => {
            let message = if stderr.trim().is_empty() {
                format!("command exited with non-zero status {exit_code}")
            } else {
                stderr.clone()
            };
            ToolResult::err_with_data(
                message,
                serde_json::json!({
                    "exit_code": exit_code,
                    "stderr": stderr,
                }),
            )
        }
        EnvironmentError::CommandTimedOut { timeout_ms } => ToolResult::err_with_data(
            format!("command timed out after {timeout_ms}ms"),
            serde_json::json!({ "timeout_ms": timeout_ms }),
        ),
        EnvironmentError::OutputLimitExceeded {
            max_bytes,
            max_lines,
            observed_bytes,
            observed_lines,
        } => ToolResult::err_with_data(
            "command output limit exceeded",
            serde_json::json!({
                "max_bytes": max_bytes,
                "max_lines": max_lines,
                "observed_bytes": observed_bytes,
                "observed_lines": observed_lines,
            }),
        ),
        EnvironmentError::ShellStateConflict {
            expected_generation,
            actual_generation,
        } => ToolResult::err_with_data(
            "shell state conflict",
            serde_json::json!({
                "expected_generation": expected_generation,
                "actual_generation": actual_generation,
            }),
        ),
        other => ToolResult::err(other.to_string()),
    }
}

fn parse_export(rest: &str) -> Option<(String, String)> {
    let (key, value) = rest.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    let first = key.chars().next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if !key
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }

    Some((key.to_string(), value.trim().to_string()))
}

fn parse_persisted_builtin(command: &str) -> Option<PersistedBuiltin> {
    let trimmed = command.trim();

    if trimmed == "cd" {
        return Some(PersistedBuiltin::Cd(String::new()));
    }

    if let Some(rest) = trimmed.strip_prefix("cd ") {
        return Some(PersistedBuiltin::Cd(rest.trim().to_string()));
    }

    if let Some(rest) = trimmed.strip_prefix("export ") {
        if let Some((key, value)) = parse_export(rest.trim()) {
            return Some(PersistedBuiltin::Export { key, value });
        }
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix("unset ") {
        let key = rest.trim();
        if !key.is_empty() {
            return Some(PersistedBuiltin::Unset(key.to_string()));
        }
        return None;
    }

    None
}

fn boundary_mutation(command: &str) -> Option<&'static str> {
    let trimmed = command.trim();
    if trimmed.starts_with("source ") || trimmed.starts_with(". ") {
        return Some("source");
    }
    if trimmed.starts_with("alias ") {
        return Some("alias");
    }
    if trimmed.starts_with("function ") {
        return Some("function");
    }
    None
}

async fn execute_request(ctx: &ToolContext, request: CommandRequest) -> ToolResult {
    if ctx.cancellation.is_cancelled() {
        return env_error_to_tool_result(EnvironmentError::CancellationRequested);
    }

    let shell_state_snapshot = ctx.snapshot_shell_state().await;
    match ctx.env.execute_command(request, shell_state_snapshot).await {
        Ok(output) => command_success(output),
        Err(error) => env_error_to_tool_result(error),
    }
}

fn resolve_cd_target(raw_path: &str, snapshot: &ShellState) -> Result<PathBuf, ToolResult> {
    let target = if raw_path.trim().is_empty() {
        std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
    } else {
        raw_path.trim().to_string()
    };

    let requested = PathBuf::from(target);
    let candidate = if requested.is_absolute() {
        requested
    } else {
        snapshot.cwd.join(requested)
    };

    let canonical = std::fs::canonicalize(&candidate)
        .map_err(|error| ToolResult::err(format!("cd failed for '{}': {}", candidate.display(), error)))?;

    let metadata = std::fs::metadata(&canonical)
        .map_err(|error| ToolResult::err(format!("cd failed for '{}': {}", canonical.display(), error)))?;

    if !metadata.is_dir() {
        return Err(ToolResult::err(format!(
            "cd target is not a directory: {}",
            canonical.display()
        )));
    }

    Ok(canonical)
}

async fn commit_cd(ctx: &ToolContext, raw_path: &str) -> ToolResult {
    let snapshot = ctx.snapshot_shell_state().await;
    let target = match resolve_cd_target(raw_path, &snapshot) {
        Ok(target) => target,
        Err(error) => return error,
    };

    match ctx
        .commit_shell_mutation(snapshot.generation, |state| {
            state.cwd = target.clone();
        })
        .await
    {
        Ok(()) => ToolResult::ok(serde_json::json!({
            "persisted": true,
            "builtin": "cd",
            "cwd": target,
            "generation": snapshot.generation.saturating_add(1),
        })),
        Err(error) => env_error_to_tool_result(error),
    }
}

async fn commit_export(ctx: &ToolContext, key: String, value: String) -> ToolResult {
    let snapshot = ctx.snapshot_shell_state().await;
    match ctx
        .commit_shell_mutation(snapshot.generation, |state| {
            state.env_vars.insert(key.clone(), value.clone());
        })
        .await
    {
        Ok(()) => ToolResult::ok(serde_json::json!({
            "persisted": true,
            "builtin": "export",
            "key": key,
            "value": value,
            "generation": snapshot.generation.saturating_add(1),
        })),
        Err(error) => env_error_to_tool_result(error),
    }
}

async fn commit_unset(ctx: &ToolContext, key: String) -> ToolResult {
    let snapshot = ctx.snapshot_shell_state().await;
    match ctx
        .commit_shell_mutation(snapshot.generation, |state| {
            state.env_vars.remove(&key);
        })
        .await
    {
        Ok(()) => ToolResult::ok(serde_json::json!({
            "persisted": true,
            "builtin": "unset",
            "key": key,
            "generation": snapshot.generation.saturating_add(1),
        })),
        Err(error) => env_error_to_tool_result(error),
    }
}

fn build_request(program: &str, args: Vec<String>, timeout_ms: u64) -> CommandRequest {
    CommandRequest {
        program: program.to_string(),
        args,
        timeout_ms,
        max_bytes: MAX_OUTPUT_BYTES,
        max_lines: MAX_OUTPUT_LINES,
    }
}

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

struct ExecuteBash;
#[async_trait]
impl ToolHandler for ExecuteBash {
    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let input: ExecuteBashInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if input.command.trim().is_empty() {
            return ToolResult::err("command cannot be empty");
        }

        if let Some(mutation) = boundary_mutation(&input.command) {
            tracing::warn!(
                event = "ShellStateBoundaryWarning",
                mutation = mutation,
                command = %input.command,
                "non-persistent shell mutation requested; execution remains ephemeral"
            );
        }

        if let Some(builtin) = parse_persisted_builtin(&input.command) {
            return match builtin {
                PersistedBuiltin::Cd(path) => commit_cd(&ctx, &path).await,
                PersistedBuiltin::Export { key, value } => commit_export(&ctx, key, value).await,
                PersistedBuiltin::Unset(key) => commit_unset(&ctx, key).await,
            };
        }

        let timeout_ms = match resolve_timeout_ms(input.timeout) {
            Ok(value) => value,
            Err(error) => return error,
        };

        if cfg!(target_os = "windows") {
            let bash_request = build_request(
                "bash",
                vec!["-c".to_string(), input.command.clone()],
                timeout_ms,
            );
            let snapshot = ctx.snapshot_shell_state().await;
            match ctx.env.execute_command(bash_request, snapshot).await {
                Ok(output) => return command_success(output),
                Err(EnvironmentError::Io { details, .. })
                    if details.contains("No such file") || details.contains("not found") =>
                {
                    let cmd_request = build_request(
                        "cmd",
                        vec!["/C".to_string(), input.command],
                        timeout_ms,
                    );
                    return execute_request(&ctx, cmd_request).await;
                }
                Err(error) => return env_error_to_tool_result(error),
            }
        }

        let request = build_request(
            "bash",
            vec!["-c".to_string(), input.command],
            timeout_ms,
        );
        execute_request(&ctx, request).await
    }
}

struct ExecutePython;
#[async_trait]
impl ToolHandler for ExecutePython {
    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let input: ExecutePythonInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if input.code.trim().is_empty() {
            return ToolResult::err("code cannot be empty");
        }

        let timeout_ms = match resolve_timeout_ms(input.timeout) {
            Ok(value) => value,
            Err(error) => return error,
        };

        let python = if cfg!(target_os = "windows") {
            "python"
        } else {
            "python3"
        };
        let request = build_request(
            python,
            vec!["-c".to_string(), input.code],
            timeout_ms,
        );

        execute_request(&ctx, request).await
    }
}

struct ExecutePowershell;
#[async_trait]
impl ToolHandler for ExecutePowershell {
    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let input: ExecutePowershellInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if input.command.trim().is_empty() {
            return ToolResult::err("command cannot be empty");
        }

        let timeout_ms = match resolve_timeout_ms(input.timeout) {
            Ok(value) => value,
            Err(error) => return error,
        };

        let ps = if cfg!(target_os = "windows") {
            "powershell"
        } else {
            "pwsh"
        };

        let request = build_request(
            ps,
            vec!["-NoProfile".to_string(), "-Command".to_string(), input.command],
            timeout_ms,
        );

        execute_request(&ctx, request).await
    }
}

pub fn register(reg: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "execute_bash".into(),
                description: "Execute a bash shell command".into(),
                category: "shell".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("command", "string", "Bash command to execute", true),
                    param(
                        "timeout",
                        "integer",
                        "Timeout in seconds (default 30)",
                        false,
                    ),
                ],
            },
            Arc::new(ExecuteBash),
        ),
        (
            ToolDef {
                name: "execute_python".into(),
                description: "Execute Python code".into(),
                category: "shell".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    param("code", "string", "Python code to execute", true),
                    param(
                        "timeout",
                        "integer",
                        "Timeout in seconds (default 30)",
                        false,
                    ),
                ],
            },
            Arc::new(ExecutePython),
        ),
        (
            ToolDef {
                name: "execute_powershell".into(),
                description: "Execute a PowerShell command".into(),
                category: "shell".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("command", "string", "PowerShell command", true),
                    param(
                        "timeout",
                        "integer",
                        "Timeout in seconds (default 30)",
                        false,
                    ),
                ],
            },
            Arc::new(ExecutePowershell),
        ),
    ];
    for (def, handler) in tools {
        reg.register(def, handler);
    }
}
