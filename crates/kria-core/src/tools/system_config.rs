use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::exec::{CommandOutput, ExecWrapper, ToolExecutionError};
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::{self, DeserializeOwned};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

const QUERY_TIMEOUT_SECS: u64 = 15;
const APPLY_TIMEOUT_SECS: u64 = 30;
const MAX_OUTPUT_BYTES: usize = 100 * 1024;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

fn parse_input<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, ToolResult> {
    serde_json::from_value(params)
        .map_err(|error| ToolResult::err(format!("invalid parameters: {error}")))
}

#[derive(Debug, Clone, Copy, JsonSchema)]
#[schemars(description = "Percentage from 0 to 100. Accepts numbers and strings like '60%'.")]
struct PercentLevel(u8);

impl PercentLevel {
    fn as_u8(self) -> u8 {
        self.0
    }
}

impl<'de> Deserialize<'de> for PercentLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RawLevel {
            Int(u64),
            Float(f64),
            Text(String),
        }

        let raw = RawLevel::deserialize(deserializer)?;
        let value = match raw {
            RawLevel::Int(value) => value,
            RawLevel::Float(value) => {
                if !value.is_finite() {
                    return Err(de::Error::custom("level must be a finite number"));
                }
                if value < 0.0 {
                    0
                } else {
                    value.round() as u64
                }
            }
            RawLevel::Text(value) => parse_level_text(&value).map_err(de::Error::custom)?,
        };

        Ok(Self(clamp_percent(value)))
    }
}

fn clamp_percent(value: u64) -> u8 {
    value.min(100) as u8
}

fn parse_level_text(text: &str) -> Result<u64, String> {
    let cleaned = text.trim().trim_end_matches('%').trim();
    if cleaned.is_empty() {
        return Err("level cannot be empty".into());
    }
    cleaned
        .parse::<u64>()
        .map_err(|_| format!("invalid level '{text}'"))
}

fn default_percent_level() -> PercentLevel {
    PercentLevel(50)
}

fn default_wifi_enabled() -> bool {
    true
}

fn default_power_plan() -> String {
    "balanced".to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetVolumeInput {
    #[serde(default = "default_percent_level")]
    level: PercentLevel,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetBrightnessInput {
    #[serde(default = "default_percent_level")]
    level: PercentLevel,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ToggleWifiInput {
    #[serde(default = "default_wifi_enabled")]
    enable: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetPowerPlanInput {
    #[serde(default = "default_power_plan")]
    plan: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ConnectWifiInput {
    ssid: String,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetEnvironmentVariableInput {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EnvironmentVariableInput {
    name: String,
}

fn exec_wrapper(timeout_secs: u64) -> ExecWrapper {
    ExecWrapper::new()
        .with_timeout(Duration::from_secs(timeout_secs))
        .with_max_output_bytes(MAX_OUTPUT_BYTES)
}

fn preferred_output(output: &CommandOutput) -> String {
    if output.stdout.trim().is_empty() {
        output.stderr.trim().to_string()
    } else {
        output.stdout.trim().to_string()
    }
}

fn non_zero_error(stderr: String, stdout: String) -> String {
    let stderr = stderr.trim().to_string();
    if stderr.is_empty() {
        stdout.trim().to_string()
    } else {
        stderr
    }
}

fn format_exec_error(error: ToolExecutionError) -> String {
    match error {
        ToolExecutionError::NonZeroExit { stderr, stdout, .. } => {
            let details = non_zero_error(stderr, stdout);
            if details.is_empty() {
                "command exited with non-zero status".to_string()
            } else {
                details
            }
        }
        ToolExecutionError::TimedOut {
            timeout_secs,
            stderr,
            stdout,
            ..
        } => {
            let details = non_zero_error(stderr, stdout);
            if details.is_empty() {
                format!("command timed out after {timeout_secs}s")
            } else {
                format!("command timed out after {timeout_secs}s: {details}")
            }
        }
        other => other.to_string(),
    }
}

async fn run_cmd(program: &str, args: &[&str], timeout_secs: u64) -> Result<CommandOutput, String> {
    exec_wrapper(timeout_secs)
        .execute(program, args)
        .await
        .map_err(format_exec_error)
}

async fn run_query(program: &str, args: &[&str]) -> Result<String, String> {
    let output = run_cmd(program, args, QUERY_TIMEOUT_SECS).await?;
    Ok(preferred_output(&output))
}

async fn run_apply(program: &str, args: &[&str]) -> Result<String, String> {
    let output = run_cmd(program, args, APPLY_TIMEOUT_SECS).await?;
    Ok(preferred_output(&output))
}

fn summarize_failures(prefix: &str, failures: &[(String, String)]) -> String {
    if failures.is_empty() {
        return prefix.to_string();
    }

    let details = failures
        .iter()
        .map(|(backend, error)| format!("{backend}: {error}"))
        .collect::<Vec<_>>()
        .join("; ");

    format!("{prefix}: {details}")
}

fn parse_percent_token(token: &str) -> Option<u8> {
    let cleaned = token
        .trim()
        .trim_matches(|c: char| matches!(c, '[' | ']' | '(' | ')' | ','));
    let without_percent = cleaned.strip_suffix('%')?;
    let value = without_percent.trim().parse::<f64>().ok()?;
    if !value.is_finite() {
        return None;
    }
    Some(value.round().clamp(0.0, 100.0) as u8)
}

fn parse_wpctl_percent(output: &str) -> Option<u8> {
    for token in output.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| matches!(c, ',' | ';'));

        if let Ok(value) = cleaned.parse::<f64>() {
            if value.is_finite() && (0.0..=1.5).contains(&value) {
                return Some((value * 100.0).round().clamp(0.0, 100.0) as u8);
            }
        }

        if let Some(percent) = parse_percent_token(cleaned) {
            return Some(percent);
        }
    }
    None
}

fn parse_any_percent(output: &str) -> Option<u8> {
    output.split_whitespace().find_map(parse_percent_token)
}

fn parse_u64_output(output: &str) -> Option<u64> {
    output
        .lines()
        .find_map(|line| line.trim().parse::<u64>().ok())
}

fn parse_gdbus_brightness_percent(output: &str) -> Option<u8> {
    let normalized: String = output
        .chars()
        .map(|c| if c.is_ascii_digit() { c } else { ' ' })
        .collect();

    normalized
        .split_whitespace()
        .find_map(|part| part.parse::<u64>().ok())
        .map(clamp_percent)
}

fn parse_xrandr_brightness_percent(output: &str) -> Option<u8> {
    for line in output.lines() {
        if let Some((_, value)) = line.split_once("Brightness:") {
            if let Ok(fraction) = value.trim().parse::<f64>() {
                if fraction.is_finite() {
                    return Some((fraction * 100.0).round().clamp(0.0, 100.0) as u8);
                }
            }
        }
    }
    None
}

fn first_connected_display(xrandr_output: &str) -> Option<String> {
    xrandr_output
        .lines()
        .find(|line| line.contains(" connected"))
        .and_then(|line| line.split_whitespace().next())
        .map(str::to_string)
}

fn validate_env_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("environment variable name is required".into());
    }

    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return Err("environment variable name is required".into());
    };

    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err("invalid environment variable name".into());
    }

    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("invalid environment variable name".into());
    }

    Ok(())
}

async fn query_current_volume() -> Result<(u8, &'static str), String> {
    let mut failures: Vec<(String, String)> = Vec::new();

    match run_query("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).await {
        Ok(output) => {
            if let Some(volume) = parse_wpctl_percent(&output) {
                return Ok((volume, "wpctl"));
            }
            failures.push((
                "wpctl".into(),
                format!("unparseable volume output: {}", output.trim()),
            ));
        }
        Err(error) => failures.push(("wpctl".into(), error)),
    }

    match run_query("pactl", &["get-sink-volume", "@DEFAULT_SINK@"]).await {
        Ok(output) => {
            if let Some(volume) = parse_any_percent(&output) {
                return Ok((volume, "pactl"));
            }
            failures.push((
                "pactl".into(),
                format!("unparseable volume output: {}", output.trim()),
            ));
        }
        Err(error) => failures.push(("pactl".into(), error)),
    }

    match run_query("amixer", &["get", "Master"]).await {
        Ok(output) => {
            if let Some(volume) = parse_any_percent(&output) {
                return Ok((volume, "amixer"));
            }
            failures.push((
                "amixer".into(),
                format!("unparseable volume output: {}", output.trim()),
            ));
        }
        Err(error) => failures.push(("amixer".into(), error)),
    }

    Err(summarize_failures(
        "failed to query current volume",
        &failures,
    ))
}

async fn apply_volume(level: u8) -> Result<&'static str, String> {
    let mut failures: Vec<(String, String)> = Vec::new();
    let volume = format!("{level}%");

    match run_apply("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &volume]).await {
        Ok(_) => return Ok("wpctl"),
        Err(error) => failures.push(("wpctl".into(), error)),
    }

    match run_apply("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &volume]).await {
        Ok(_) => return Ok("pactl"),
        Err(error) => failures.push(("pactl".into(), error)),
    }

    let amixer_value = format!("{level}% unmute");
    match run_apply("amixer", &["set", "Master", &amixer_value]).await {
        Ok(_) => return Ok("amixer"),
        Err(error) => failures.push(("amixer".into(), error)),
    }

    Err(summarize_failures("failed to set volume", &failures))
}

async fn query_current_brightness() -> Result<(u8, &'static str), String> {
    let mut failures: Vec<(String, String)> = Vec::new();

    let current_result = run_query("brightnessctl", &["get"]).await;
    let max_result = run_query("brightnessctl", &["max"]).await;
    match (current_result, max_result) {
        (Ok(current), Ok(max)) => {
            if let (Some(current_value), Some(max_value)) =
                (parse_u64_output(&current), parse_u64_output(&max))
            {
                if max_value > 0 {
                    let percent = ((current_value as f64 / max_value as f64) * 100.0)
                        .round()
                        .clamp(0.0, 100.0) as u8;
                    return Ok((percent, "brightnessctl"));
                }
            }
            failures.push((
                "brightnessctl".into(),
                format!(
                    "unparseable get/max output: get='{}' max='{}'",
                    current.trim(),
                    max.trim()
                ),
            ));
        }
        (Err(current_error), Err(max_error)) => {
            failures.push((
                "brightnessctl".into(),
                format!("get failed: {current_error}; max failed: {max_error}"),
            ));
        }
        (Err(current_error), _) => {
            failures.push((
                "brightnessctl".into(),
                format!("get failed: {current_error}"),
            ));
        }
        (_, Err(max_error)) => {
            failures.push(("brightnessctl".into(), format!("max failed: {max_error}")));
        }
    }

    match run_query(
        "gdbus",
        &[
            "call",
            "--session",
            "--dest",
            "org.gnome.SettingsDaemon.Power",
            "--object-path",
            "/org/gnome/SettingsDaemon/Power",
            "--method",
            "org.freedesktop.DBus.Properties.Get",
            "org.gnome.SettingsDaemon.Power.Screen",
            "Brightness",
        ],
    )
    .await
    {
        Ok(output) => {
            if let Some(percent) = parse_gdbus_brightness_percent(&output) {
                return Ok((percent, "gnome-settingsd"));
            }
            failures.push((
                "gdbus".into(),
                format!("unparseable brightness output: {}", output.trim()),
            ));
        }
        Err(error) => failures.push(("gdbus".into(), error)),
    }

    match run_query("xrandr", &["--verbose"]).await {
        Ok(output) => {
            if let Some(percent) = parse_xrandr_brightness_percent(&output) {
                return Ok((percent, "xrandr-gamma"));
            }
            failures.push((
                "xrandr".into(),
                format!("unparseable brightness output: {}", output.trim()),
            ));
        }
        Err(error) => failures.push(("xrandr".into(), error)),
    }

    Err(summarize_failures(
        "failed to query current brightness",
        &failures,
    ))
}

async fn apply_brightness(level: u8) -> Result<&'static str, String> {
    let mut failures: Vec<(String, String)> = Vec::new();

    let gdbus_value = format!("<int32 {level}>");
    match run_apply(
        "gdbus",
        &[
            "call",
            "--session",
            "--dest",
            "org.gnome.SettingsDaemon.Power",
            "--object-path",
            "/org/gnome/SettingsDaemon/Power",
            "--method",
            "org.freedesktop.DBus.Properties.Set",
            "org.gnome.SettingsDaemon.Power.Screen",
            "Brightness",
            &gdbus_value,
        ],
    )
    .await
    {
        Ok(_) => return Ok("gnome-settingsd"),
        Err(error) => failures.push(("gdbus".into(), error)),
    }

    let brightness = format!("{level}%");
    match run_apply("brightnessctl", &["set", &brightness]).await {
        Ok(_) => return Ok("brightnessctl"),
        Err(error) => failures.push(("brightnessctl".into(), error)),
    }

    let xrandr_output = run_query("xrandr", &[]).await;
    match xrandr_output {
        Ok(output) => {
            if let Some(display) = first_connected_display(&output) {
                let fraction = format!("{:.2}", level as f64 / 100.0);
                match run_apply("xrandr", &["--output", &display, "--brightness", &fraction]).await
                {
                    Ok(_) => return Ok("xrandr-gamma"),
                    Err(error) => failures.push(("xrandr".into(), error)),
                }
            } else {
                failures.push(("xrandr".into(), "no connected display found".into()));
            }
        }
        Err(error) => failures.push(("xrandr".into(), error)),
    }

    Err(summarize_failures("failed to set brightness", &failures))
}

async fn query_wifi_enabled() -> Result<bool, String> {
    let output = run_query("nmcli", &["radio", "wifi"]).await?;
    let normalized = output.trim().to_lowercase();

    if normalized.contains("enabled") || normalized == "on" {
        Ok(true)
    } else if normalized.contains("disabled") || normalized == "off" {
        Ok(false)
    } else {
        Err(format!(
            "unable to parse wifi state from nmcli output: {output}"
        ))
    }
}

async fn apply_wifi(enable: bool) -> Result<(), String> {
    let state = if enable { "on" } else { "off" };
    run_apply("nmcli", &["radio", "wifi", state]).await?;
    Ok(())
}

async fn query_power_plan() -> Result<String, String> {
    let output = run_query("powerprofilesctl", &["get"]).await?;
    Ok(output.trim().to_string())
}

async fn apply_power_plan(plan: &str) -> Result<(), String> {
    run_apply("powerprofilesctl", &["set", plan]).await?;
    Ok(())
}

async fn query_active_wifi_ssid() -> Result<Option<String>, String> {
    let output = run_query(
        "nmcli",
        &["-t", "-f", "ACTIVE,SSID", "device", "wifi", "list"],
    )
    .await?;

    for line in output.lines() {
        let mut parts = line.splitn(2, ':');
        let active = parts.next().unwrap_or_default().trim();
        let ssid = parts.next().unwrap_or_default().trim();
        if active.eq_ignore_ascii_case("yes") && !ssid.is_empty() {
            return Ok(Some(ssid.to_string()));
        }
    }

    Ok(None)
}

async fn apply_connect_wifi(ssid: &str, password: Option<&str>) -> Result<String, String> {
    let mut args: Vec<String> = vec![
        "device".into(),
        "wifi".into(),
        "connect".into(),
        ssid.into(),
    ];
    if let Some(password) = password {
        args.push("password".into());
        args.push(password.into());
    }

    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_apply("nmcli", &refs).await
}

struct SetVolume;

#[async_trait]
impl ToolHandler for SetVolume {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: SetVolumeInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("set_volume not implemented for this OS");
        }

        let requested = input.level.as_u8();

        match query_current_volume().await {
            Ok((current, backend)) if current == requested => {
                return ToolResult::ok(serde_json::json!({
                    "volume": requested,
                    "backend": backend,
                    "changed": false,
                    "already_in_desired_state": true,
                    "message": format!("volume already set to {requested}%"),
                }));
            }
            Ok(_) => {}
            Err(error) => warn!("set_volume pre-flight query failed: {error}"),
        }

        match apply_volume(requested).await {
            Ok(backend) => ToolResult::ok(serde_json::json!({
                "volume": requested,
                "backend": backend,
                "changed": true,
                "already_in_desired_state": false,
            })),
            Err(error) => ToolResult::err(error),
        }
    }
}

struct SetBrightness;

#[async_trait]
impl ToolHandler for SetBrightness {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: SetBrightnessInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("set_brightness not implemented for this OS");
        }

        let requested = input.level.as_u8();

        match query_current_brightness().await {
            Ok((current, backend)) if current == requested => {
                return ToolResult::ok(serde_json::json!({
                    "brightness": requested,
                    "backend": backend,
                    "changed": false,
                    "already_in_desired_state": true,
                    "message": format!("brightness already set to {requested}%"),
                }));
            }
            Ok(_) => {}
            Err(error) => warn!("set_brightness pre-flight query failed: {error}"),
        }

        match apply_brightness(requested).await {
            Ok(backend) => ToolResult::ok(serde_json::json!({
                "brightness": requested,
                "backend": backend,
                "changed": true,
                "already_in_desired_state": false,
            })),
            Err(error) => ToolResult::err(error),
        }
    }
}

struct ToggleWifi;

#[async_trait]
impl ToolHandler for ToggleWifi {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: ToggleWifiInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("toggle_wifi not implemented for this OS");
        }

        match query_wifi_enabled().await {
            Ok(current) if current == input.enable => {
                return ToolResult::ok(serde_json::json!({
                    "wifi": if input.enable { "on" } else { "off" },
                    "changed": false,
                    "already_in_desired_state": true,
                }));
            }
            Ok(_) => {}
            Err(error) => warn!("toggle_wifi pre-flight query failed: {error}"),
        }

        match apply_wifi(input.enable).await {
            Ok(()) => ToolResult::ok(serde_json::json!({
                "wifi": if input.enable { "on" } else { "off" },
                "changed": true,
                "already_in_desired_state": false,
            })),
            Err(error) => ToolResult::err(format!("failed to toggle wifi (nmcli): {error}")),
        }
    }
}

struct SetPowerPlan;

#[async_trait]
impl ToolHandler for SetPowerPlan {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: SetPowerPlanInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("set_power_plan not implemented for this OS");
        }

        let requested = input.plan.trim();
        if requested.is_empty() {
            return ToolResult::err("plan parameter is required");
        }

        match query_power_plan().await {
            Ok(current) if current == requested => {
                return ToolResult::ok(serde_json::json!({
                    "power_plan": requested,
                    "changed": false,
                    "already_in_desired_state": true,
                }));
            }
            Ok(_) => {}
            Err(error) => warn!("set_power_plan pre-flight query failed: {error}"),
        }

        match apply_power_plan(requested).await {
            Ok(()) => ToolResult::ok(serde_json::json!({
                "power_plan": requested,
                "changed": true,
                "already_in_desired_state": false,
            })),
            Err(error) => ToolResult::err(format!(
                "failed to set power plan (powerprofilesctl): {error}"
            )),
        }
    }
}

struct GetPowerPlan;

#[async_trait]
impl ToolHandler for GetPowerPlan {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let _input: EmptyInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::ok(serde_json::json!({ "power_plan": "unsupported" }));
        }

        match query_power_plan().await {
            Ok(plan) => ToolResult::ok(serde_json::json!({ "power_plan": plan })),
            Err(_) => ToolResult::ok(serde_json::json!({ "power_plan": "unknown" })),
        }
    }
}

struct ConnectWifi;

#[async_trait]
impl ToolHandler for ConnectWifi {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: ConnectWifiInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("connect_wifi not implemented for this OS");
        }

        let ssid = input.ssid.trim();
        if ssid.is_empty() {
            return ToolResult::err("ssid parameter is required");
        }

        match query_active_wifi_ssid().await {
            Ok(Some(current)) if current == ssid => {
                return ToolResult::ok(serde_json::json!({
                    "connected": ssid,
                    "changed": false,
                    "already_in_desired_state": true,
                }));
            }
            Ok(_) => {}
            Err(error) => warn!("connect_wifi pre-flight query failed: {error}"),
        }

        match apply_connect_wifi(ssid, input.password.as_deref()).await {
            Ok(output) => ToolResult::ok(serde_json::json!({
                "connected": ssid,
                "changed": true,
                "already_in_desired_state": false,
                "output": output,
            })),
            Err(error) => ToolResult::err(format!("connect_wifi failed: {error}")),
        }
    }
}

struct GetWifiNetworks;

#[async_trait]
impl ToolHandler for GetWifiNetworks {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let _input: EmptyInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        match run_query(
            "nmcli",
            &["-t", "-f", "SSID,SIGNAL,SECURITY", "device", "wifi", "list"],
        )
        .await
        {
            Ok(output) => {
                let networks: Vec<serde_json::Value> = output
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| {
                        let parts: Vec<&str> = line.splitn(3, ':').collect();
                        serde_json::json!({
                            "ssid": parts.first().copied().unwrap_or_default(),
                            "signal": parts.get(1).copied().unwrap_or_default(),
                            "security": parts.get(2).copied().unwrap_or_default(),
                        })
                    })
                    .collect();

                ToolResult::ok(serde_json::json!({ "networks": networks }))
            }
            Err(error) => ToolResult::err(format!("failed to list wifi networks (nmcli): {error}")),
        }
    }
}

struct SetEnvironmentVariable;

#[async_trait]
impl ToolHandler for SetEnvironmentVariable {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: SetEnvironmentVariableInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let name = input.name.trim();
        if let Err(error) = validate_env_name(name) {
            return ToolResult::err(error);
        }

        if std::env::var(name).ok().as_deref() == Some(input.value.as_str()) {
            return ToolResult::ok(serde_json::json!({
                "name": name,
                "value": input.value,
                "set": true,
                "changed": false,
                "already_in_desired_state": true,
            }));
        }

        std::env::set_var(name, &input.value);
        ToolResult::ok(serde_json::json!({
            "name": name,
            "value": input.value,
            "set": true,
            "changed": true,
            "already_in_desired_state": false,
        }))
    }
}

struct GetEnvironmentVariable;

#[async_trait]
impl ToolHandler for GetEnvironmentVariable {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: EnvironmentVariableInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let name = input.name.trim();
        if let Err(error) = validate_env_name(name) {
            return ToolResult::err(error);
        }

        let value = std::env::var(name).ok();
        ToolResult::ok(serde_json::json!({ "name": name, "value": value }))
    }
}

struct ListEnvironmentVariables;

#[async_trait]
impl ToolHandler for ListEnvironmentVariables {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let _input: EmptyInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let vars: Vec<serde_json::Value> = std::env::vars()
            .filter(|(key, _)| {
                !key.contains("KEY")
                    && !key.contains("SECRET")
                    && !key.contains("TOKEN")
                    && !key.contains("PASSWORD")
            })
            .map(|(name, value)| serde_json::json!({ "name": name, "value": value }))
            .collect();

        ToolResult::ok(serde_json::json!({
            "variables": vars,
            "count": vars.len(),
        }))
    }
}

pub fn register(reg: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        // GREEN
        (
            ToolDef {
                name: "get_power_plan".into(),
                description: "Get current power plan".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetPowerPlan),
        ),
        (
            ToolDef {
                name: "get_environment_variable".into(),
                description: "Get an environment variable value".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param("name", "string", "Variable name", true)],
            },
            Arc::new(GetEnvironmentVariable),
        ),
        (
            ToolDef {
                name: "list_environment_variables".into(),
                description: "List all environment variables (secrets filtered)".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(ListEnvironmentVariables),
        ),
        (
            ToolDef {
                name: "get_wifi_networks".into(),
                description: "List available WiFi networks".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Green,
                min_tier: "standard",
                parameters: vec![],
            },
            Arc::new(GetWifiNetworks),
        ),
        // YELLOW
        (
            ToolDef {
                name: "set_volume".into(),
                description: "Set system audio volume (0-100)".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("level", "integer", "Volume 0-100", true)],
            },
            Arc::new(SetVolume),
        ),
        (
            ToolDef {
                name: "set_brightness".into(),
                description: "Set screen brightness (0-100)".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("level", "integer", "Brightness 0-100", true)],
            },
            Arc::new(SetBrightness),
        ),
        (
            ToolDef {
                name: "toggle_wifi".into(),
                description: "Enable or disable WiFi".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("enable", "boolean", "true=on, false=off", true)],
            },
            Arc::new(ToggleWifi),
        ),
        (
            ToolDef {
                name: "set_power_plan".into(),
                description: "Set power plan (balanced/performance/power-saver)".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("plan", "string", "Power plan name", true)],
            },
            Arc::new(SetPowerPlan),
        ),
        (
            ToolDef {
                name: "connect_wifi".into(),
                description: "Connect to a WiFi network".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "standard",
                parameters: vec![
                    param("ssid", "string", "Network name", true),
                    param("password", "string", "Network password", false),
                ],
            },
            Arc::new(ConnectWifi),
        ),
        // RED
        (
            ToolDef {
                name: "set_environment_variable".into(),
                description: "Set an environment variable".into(),
                category: "system_config".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("name", "string", "Variable name", true),
                    param("value", "string", "Variable value", true),
                ],
            },
            Arc::new(SetEnvironmentVariable),
        ),
    ];

    for (def, handler) in tools {
        reg.register(def, handler);
    }
}
