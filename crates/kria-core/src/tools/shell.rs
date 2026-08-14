use crate::infra::environment::{CommandRequest, CommandResult, EnvironmentError, ShellState};
use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::{ToolContext, TriggerProvenance};
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

// ─── Raw shell containment (OSC-002 §7, OSC-030) ─────────────────────────────
//
// Raw generic shell (`execute_bash`/`execute_python`/`execute_powershell`) is
// the separately-governed Expert Mode surface: RED, always-confirmed,
// non-rollbackable, and — enforced here at the tool boundary — unavailable to
// unattended automation, forbidden from interpolating secret references, and
// unable to reach prohibited BLACK-scope administration. Structured OS
// capabilities do NOT pass through here and are never restricted by this gate.

/// The outcome of admitting a raw-shell invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawShellAdmission {
    /// The invocation may proceed to the (still RED, still approved) executor.
    Allowed,
    /// The invocation is refused at the tool boundary. `code` is a stable
    /// machine tag; `message` is the user-facing explanation.
    Refused { code: &'static str, message: String },
}

/// Whether Expert Mode raw shell is enabled for this process.
///
/// Raw shell is enabled by default and opted out with
/// `KRIA_EXPERT_MODE=0|false|off`. Even when enabled it remains RED,
/// always-confirmed, and non-rollbackable; disabling it turns the tools into an
/// honest refusal (satisfying the "disabled by default OR Expert Mode" branch
/// of OSC-002 §7 for deployments that choose the stricter posture).
fn expert_mode_enabled() -> bool {
    !matches!(
        std::env::var("KRIA_EXPERT_MODE").ok().as_deref(),
        Some("0") | Some("false") | Some("off") | Some("FALSE") | Some("OFF")
    )
}

/// Detect secret-reference interpolation in a raw command/code string.
///
/// Raw shell must never interpolate opaque secret references (OSC-002 §5/§7,
/// OSC-025). We match the reference syntaxes KRIA uses plus common credential
/// environment-variable interpolation.
fn contains_secret_interpolation(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    const REFERENCE_MARKERS: &[&str] = &[
        "${secret",
        "{{secret",
        "{{ secret",
        "<secret:",
        "secret://",
        "$kria_secret",
        "${kria_secret",
        "kria-secret://",
        "${credential",
        "{{credential",
        "vault:",
    ];
    if REFERENCE_MARKERS.iter().any(|m| lower.contains(m)) {
        return true;
    }
    // Credential-bearing environment interpolation, e.g. `$PASSWORD`,
    // `${API_KEY}`, `$AWS_SECRET_ACCESS_KEY`.
    const CREDENTIAL_VARS: &[&str] = &[
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "access_key",
        "private_key",
        "credential",
    ];
    for var in CREDENTIAL_VARS {
        let braced = format!("${{{var}");
        let bare = format!("${var}");
        if lower.contains(&braced) || lower.contains(&bare) {
            return true;
        }
    }
    false
}

/// Evaluate whether a raw-shell invocation is admissible at the tool boundary.
///
/// Order: prohibited-scope containment → secret interpolation → unattended
/// automation → Expert Mode availability. Prohibited scope and secret
/// interpolation are refused regardless of Expert Mode.
pub fn evaluate_raw_shell_admission(
    source: &str,
    provenance: TriggerProvenance,
    expert_mode: bool,
) -> RawShellAdmission {
    if let Some(prohibited) = crate::safety::black_scope::classify_command(source) {
        return RawShellAdmission::Refused {
            code: "prohibited_scope",
            message: format!(
                "prohibited scope [{}]: {}",
                prohibited.id(),
                prohibited.boundary_explanation()
            ),
        };
    }
    if contains_secret_interpolation(source) {
        return RawShellAdmission::Refused {
            code: "secret_interpolation",
            message:
                "raw shell must not interpolate secret or credential references; use a structured \
                 capability that resolves credentials inside its provider instead"
                    .to_string(),
        };
    }
    if provenance != TriggerProvenance::User {
        return RawShellAdmission::Refused {
            code: "unattended_automation",
            message:
                "raw shell is Expert Mode and unavailable to unattended automation or content-\
                 triggered execution; it requires a direct, attended user request"
                    .to_string(),
        };
    }
    if !expert_mode {
        return RawShellAdmission::Refused {
            code: "expert_mode_disabled",
            message:
                "raw shell execution is disabled; enable Expert Mode (KRIA_EXPERT_MODE) to use it"
                    .to_string(),
        };
    }
    RawShellAdmission::Allowed
}

/// Apply the raw-shell admission gate, returning an error `ToolResult` when the
/// invocation is refused.
fn admit_raw_shell(source: &str, ctx: &ToolContext) -> Result<(), ToolResult> {
    match evaluate_raw_shell_admission(source, ctx.provenance, expert_mode_enabled()) {
        RawShellAdmission::Allowed => Ok(()),
        RawShellAdmission::Refused { code, message } => {
            tracing::warn!(
                event = "RawShellRefused",
                refusal = code,
                "raw shell invocation refused at tool boundary"
            );
            Err(ToolResult::err(message))
        }
    }
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
    serde_json::from_value(params)
        .map_err(|error| ToolResult::err(format!("invalid parameters: {error}")))
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

    let canonical = std::fs::canonicalize(&candidate).map_err(|error| {
        ToolResult::err(format!(
            "cd failed for '{}': {}",
            candidate.display(),
            error
        ))
    })?;

    let metadata = std::fs::metadata(&canonical).map_err(|error| {
        ToolResult::err(format!(
            "cd failed for '{}': {}",
            canonical.display(),
            error
        ))
    })?;

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
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: ExecuteBashInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if input.command.trim().is_empty() {
            return ToolResult::err("command cannot be empty");
        }

        if let Err(refusal) = admit_raw_shell(&input.command, &ctx) {
            return refusal;
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
                    let cmd_request =
                        build_request("cmd", vec!["/C".to_string(), input.command], timeout_ms);
                    return execute_request(&ctx, cmd_request).await;
                }
                Err(error) => return env_error_to_tool_result(error),
            }
        }

        let request = build_request("bash", vec!["-c".to_string(), input.command], timeout_ms);
        execute_request(&ctx, request).await
    }
}

struct ExecutePython;
#[async_trait]
impl ToolHandler for ExecutePython {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: ExecutePythonInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if input.code.trim().is_empty() {
            return ToolResult::err("code cannot be empty");
        }

        if let Err(refusal) = admit_raw_shell(&input.code, &ctx) {
            return refusal;
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
        let request = build_request(python, vec!["-c".to_string(), input.code], timeout_ms);

        execute_request(&ctx, request).await
    }
}

struct ExecutePowershell;
#[async_trait]
impl ToolHandler for ExecutePowershell {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: ExecutePowershellInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if input.command.trim().is_empty() {
            return ToolResult::err("command cannot be empty");
        }

        if let Err(refusal) = admit_raw_shell(&input.command, &ctx) {
            return refusal;
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
            vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                input.command,
            ],
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
                description: "Execute a bash shell command (Expert Mode: RED, always-confirmed, \
                    non-rollbackable, direct-user-only; never used for prohibited administration)"
                    .into(),
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
                description: "Execute Python code (Expert Mode: RED, always-confirmed, \
                    non-rollbackable, direct-user-only; never used for prohibited administration)"
                    .into(),
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
                description: "Execute a PowerShell command (Expert Mode: RED, always-confirmed, \
                    non-rollbackable, direct-user-only; never used for prohibited administration)"
                    .into(),
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

#[cfg(test)]
mod raw_shell_admission_tests {
    use super::*;
    use crate::tools::TriggerProvenance;

    #[test]
    fn attended_benign_command_is_allowed_in_expert_mode() {
        let a = evaluate_raw_shell_admission("ls -la /var/log", TriggerProvenance::User, true);
        assert_eq!(a, RawShellAdmission::Allowed);
    }

    #[test]
    fn prohibited_scope_is_refused_even_in_expert_mode() {
        let a = evaluate_raw_shell_admission("mkfs.ext4 /dev/sdb1", TriggerProvenance::User, true);
        match a {
            RawShellAdmission::Refused { code, .. } => assert_eq!(code, "prohibited_scope"),
            other => panic!("expected refusal, got {other:?}"),
        }
    }

    #[test]
    fn secret_interpolation_is_refused() {
        for cmd in [
            "curl -H \"Authorization: ${SECRET_TOKEN}\" https://x",
            "echo $PASSWORD | login",
            "deploy --key {{secret.api_key}}",
        ] {
            let a = evaluate_raw_shell_admission(cmd, TriggerProvenance::User, true);
            match a {
                RawShellAdmission::Refused { code, .. } => {
                    assert_eq!(code, "secret_interpolation", "for `{cmd}`")
                }
                other => panic!("expected refusal for `{cmd}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn unattended_automation_is_refused() {
        let external =
            evaluate_raw_shell_admission("ls -la", TriggerProvenance::ExternalContent, true);
        assert!(matches!(
            external,
            RawShellAdmission::Refused {
                code: "unattended_automation",
                ..
            }
        ));
        let tool = evaluate_raw_shell_admission("ls -la", TriggerProvenance::Tool, true);
        assert!(matches!(
            tool,
            RawShellAdmission::Refused {
                code: "unattended_automation",
                ..
            }
        ));
    }

    #[test]
    fn disabled_expert_mode_refuses_attended_command() {
        let a = evaluate_raw_shell_admission("ls -la", TriggerProvenance::User, false);
        assert!(matches!(
            a,
            RawShellAdmission::Refused {
                code: "expert_mode_disabled",
                ..
            }
        ));
    }

    #[test]
    fn prohibited_scope_precedes_expert_mode_check() {
        // Even with Expert Mode disabled, prohibited scope reports the scope
        // refusal (the boundary explanation), not a generic disabled message.
        let a = evaluate_raw_shell_admission("useradd bob", TriggerProvenance::User, false);
        assert!(matches!(
            a,
            RawShellAdmission::Refused {
                code: "prohibited_scope",
                ..
            }
        ));
    }
}
