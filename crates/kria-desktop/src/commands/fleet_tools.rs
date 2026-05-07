use super::*;

#[derive(Debug, Clone, serde::Deserialize)]
struct ExecuteFleetCommandToolInput {
    command: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    lease_ttl_seconds: Option<u64>,
    #[serde(default)]
    max_attempts: Option<usize>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct GetFleetOverviewToolInput {
    #[serde(default)]
    target: Option<String>,
}

#[derive(Clone)]
struct ExecuteFleetCommandTool {
    fleet_control_runtime: Arc<DesktopFleetControlRuntime>,
}

#[derive(Clone)]
struct GetFleetOverviewTool {
    fleet_control_runtime: Arc<DesktopFleetControlRuntime>,
}

fn fleet_tool_param(name: &str, param_type: &str, description: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        param_type: param_type.to_string(),
        description: description.to_string(),
        required,
        default: None,
    }
}

pub(crate) fn register_fleet_runtime_tools(
    tool_registry: &ToolRegistry,
    fleet_control_runtime: Arc<DesktopFleetControlRuntime>,
) {
    let definition = ToolDef {
        name: "execute_fleet_command".to_string(),
        description: "Execute a shell command on an enrolled fleet target (connected VM/computer) through KRIA fleet connection control. Use this for remote host actions like package install/check on connected targets.".to_string(),
        category: "fleet".to_string(),
        default_tier: RiskLevel::Red,
        min_tier: "lite",
        parameters: vec![
            fleet_tool_param("command", "string", "Shell command to run on the remote target", true),
            fleet_tool_param("target", "string", "Optional target hint (target_id, display name, host, or user@host). Omit to auto-select best ready target", false),
            fleet_tool_param("lease_ttl_seconds", "integer", "Optional lease TTL in seconds (default 300, min 30, max 900)", false),
            fleet_tool_param("max_attempts", "integer", "Optional dispatch retry attempts (default 2, min 1, max 6)", false),
        ],
    };

    let handler: Arc<dyn ToolHandler> = Arc::new(ExecuteFleetCommandTool {
        fleet_control_runtime: fleet_control_runtime.clone(),
    });
    tool_registry.register(definition, handler);

    let overview_definition = ToolDef {
        name: "get_fleet_overview".to_string(),
        description: "Get enrolled/connected VM and remote target inventory, including total counts and target states. Use this for prompts like 'How many VMs do I have?' or 'List my connected machines'.".to_string(),
        category: "fleet".to_string(),
        default_tier: RiskLevel::Green,
        min_tier: "lite",
        parameters: vec![fleet_tool_param(
            "target",
            "string",
            "Optional target filter hint (target_id or display name substring)",
            false,
        )],
    };

    let overview_handler: Arc<dyn ToolHandler> = Arc::new(GetFleetOverviewTool {
        fleet_control_runtime,
    });
    tool_registry.register(overview_definition, overview_handler);
}

fn fleet_target_matches_hint(
    target: &crate::fleet_control::FleetTargetProjection,
    hint: &str,
) -> bool {
    let needle = hint.to_ascii_lowercase();
    if needle.is_empty() {
        return true;
    }

    target.target_id.to_ascii_lowercase().starts_with(&needle)
        || target.display_name.to_ascii_lowercase().contains(&needle)
}

fn remote_command_sudo_hint(stderr: &str) -> Option<&'static str> {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("sudo:")
        && (lower.contains("password") || lower.contains("tty") || lower.contains("askpass"))
    {
        return Some(
            "Remote sudo likely requires interactive password. Configure passwordless sudo for this automation user or run a non-sudo command.",
        );
    }
    None
}

fn remote_command_error_excerpt(stderr: &str) -> Option<String> {
    let compact = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(2)
        .collect::<Vec<_>>()
        .join(" | ");

    if compact.is_empty() {
        None
    } else {
        Some(truncate_for_error(&compact, 220))
    }
}

#[async_trait]
impl ToolHandler for ExecuteFleetCommandTool {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: ExecuteFleetCommandToolInput = match serde_json::from_value(params) {
            Ok(value) => value,
            Err(error) => return ToolResult::err(format!("invalid parameters: {error}")),
        };

        let command = input.command.trim();
        if command.is_empty() {
            return ToolResult::err("command parameter is required");
        }

        let lease_ttl_seconds = input.lease_ttl_seconds.unwrap_or(300).clamp(30, 900);
        let max_attempts = input.max_attempts.unwrap_or(2).clamp(1, 6);
        let target_hint = input
            .target
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let result = self
            .fleet_control_runtime
            .run_shell_command(
                command,
                target_hint,
                Duration::from_secs(lease_ttl_seconds),
                Duration::from_secs(45),
                max_attempts,
            )
            .await;

        let outcome = match result {
            Ok(value) => value,
            Err(error) => {
                return ToolResult::err(format!("fleet command dispatch failed: {error:#}"))
            }
        };

        let mut data = match serde_json::to_value(&outcome) {
            Ok(value) => value,
            Err(error) => {
                return ToolResult::err(format!(
                    "fleet command result serialization failed: {error}"
                ))
            }
        };

        if let Some(hint) = remote_command_sudo_hint(&outcome.stderr) {
            data["hint"] = serde_json::json!(hint);
        }

        if outcome.exit_code == 0 {
            ToolResult::ok(data)
        } else {
            let mut message = format!(
                "remote command exited with non-zero status {}",
                outcome.exit_code
            );
            if let Some(excerpt) = remote_command_error_excerpt(&outcome.stderr) {
                message.push_str("; stderr: ");
                message.push_str(&excerpt);
            }
            if let Some(hint) = remote_command_sudo_hint(&outcome.stderr) {
                message.push_str("; ");
                message.push_str(hint);
            }

            ToolResult::err_with_data(message, data)
        }
    }
}

#[async_trait]
impl ToolHandler for GetFleetOverviewTool {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: GetFleetOverviewToolInput = match serde_json::from_value(params) {
            Ok(value) => value,
            Err(error) => return ToolResult::err(format!("invalid parameters: {error}")),
        };

        let targets = self.fleet_control_runtime.snapshot_targets().await;
        let total_targets = targets.len();
        let ready_targets = targets.iter().filter(|row| row.state == "ready").count();
        let leased_targets = targets.iter().filter(|row| row.state == "leased").count();
        let tainted_targets = targets
            .iter()
            .filter(|row| row.state == "tainted" || row.tainted)
            .count();
        let quarantined_targets = targets
            .iter()
            .filter(|row| row.state == "quarantine")
            .count();
        let disabled_targets = targets.iter().filter(|row| row.state == "disabled").count();

        let filter_hint = input
            .target
            .as_deref()
            .map(str::trim)
            .filter(|hint| !hint.is_empty())
            .map(str::to_string);

        let visible_targets = if let Some(filter) = filter_hint.as_deref() {
            targets
                .iter()
                .filter(|row| fleet_target_matches_hint(row, filter))
                .cloned()
                .collect::<Vec<_>>()
        } else {
            targets.clone()
        };

        let mut payload = serde_json::json!({
            "total_targets": total_targets,
            "ready_targets": ready_targets,
            "leased_targets": leased_targets,
            "tainted_targets": tainted_targets,
            "quarantined_targets": quarantined_targets,
            "disabled_targets": disabled_targets,
            "selected_target_count": visible_targets.len(),
            "filter_applied": filter_hint,
            "targets": visible_targets,
        });

        if payload["selected_target_count"] == serde_json::json!(0)
            && payload["filter_applied"].is_string()
            && total_targets > 0
        {
            payload["hint"] = serde_json::json!(
                "No targets matched the filter. Retry with target_id or a display-name fragment."
            );
        }

        ToolResult::ok(payload)
    }
}
