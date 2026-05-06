use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::exec::{CommandOutput, ExecWrapper, ToolExecutionError};
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
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
    serde_json::from_value(params).map_err(|error| ToolResult::err(format!("invalid parameters: {error}")))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WindowMatchInput {
    title: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MoveWindowInput {
    title: String,
    x: i64,
    y: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ResizeWindowInput {
    title: String,
    width: i64,
    height: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TileWindowsInput {
    windows: Vec<String>,
    #[serde(default = "default_tile_layout")]
    layout: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OpenUrlInput {
    url: String,
}

fn default_tile_layout() -> String {
    "side-by-side".to_string()
}

#[derive(Debug, Clone)]
struct WindowEntry {
    id: String,
    title: String,
    x: Option<i64>,
    y: Option<i64>,
    width: Option<i64>,
    height: Option<i64>,
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
    let trimmed_stderr = stderr.trim().to_string();
    if trimmed_stderr.is_empty() {
        stdout.trim().to_string()
    } else {
        trimmed_stderr
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

async fn run_apply_owned(program: &str, args: Vec<String>) -> Result<String, String> {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_apply(program, &refs).await
}

fn parse_wmctrl_list_line(line: &str) -> Option<WindowEntry> {
    let mut parts = line.split_whitespace();
    let id = parts.next()?.to_string();
    let _desktop = parts.next()?;
    let _host = parts.next()?;
    let title = parts.collect::<Vec<_>>().join(" ").trim().to_string();

    if title.is_empty() {
        return None;
    }

    Some(WindowEntry {
        id,
        title,
        x: None,
        y: None,
        width: None,
        height: None,
    })
}

fn parse_wmctrl_geometry_line(line: &str) -> Option<WindowEntry> {
    let mut parts = line.split_whitespace();

    let id = parts.next()?.to_string();
    let _desktop = parts.next()?;
    let x = parts.next()?.parse::<i64>().ok()?;
    let y = parts.next()?.parse::<i64>().ok()?;
    let width = parts.next()?.parse::<i64>().ok()?;
    let height = parts.next()?.parse::<i64>().ok()?;
    let _host = parts.next()?;
    let title = parts.collect::<Vec<_>>().join(" ").trim().to_string();

    if title.is_empty() {
        return None;
    }

    Some(WindowEntry {
        id,
        title,
        x: Some(x),
        y: Some(y),
        width: Some(width),
        height: Some(height),
    })
}

fn normalize_title(title: &str) -> String {
    title.trim().to_ascii_lowercase()
}

fn find_window_by_title(entries: &[WindowEntry], title: &str) -> Option<WindowEntry> {
    let wanted = normalize_title(title);
    entries
        .iter()
        .find(|entry| normalize_title(&entry.title).contains(&wanted))
        .cloned()
}

async fn query_windows_for_matching() -> Result<Vec<WindowEntry>, String> {
    let output = run_query("wmctrl", &["-l"]).await?;
    Ok(output
        .lines()
        .filter_map(parse_wmctrl_list_line)
        .collect::<Vec<_>>())
}

async fn query_window_geometries() -> Result<Vec<WindowEntry>, String> {
    let output = run_query("wmctrl", &["-lG"]).await?;
    Ok(output
        .lines()
        .filter_map(parse_wmctrl_geometry_line)
        .collect::<Vec<_>>())
}

async fn query_window_id_by_title(title: &str) -> Result<Option<String>, String> {
    let entries = query_windows_for_matching().await?;
    Ok(find_window_by_title(&entries, title).map(|entry| entry.id))
}

fn parse_wm_class(raw: &str) -> String {
    raw.split('=')
        .nth(1)
        .map(|value| value.trim().replace('"', ""))
        .unwrap_or_default()
}

fn parse_screen_dimensions(output: &str) -> Option<(i64, i64)> {
    for line in output.lines() {
        if !line.contains("dimensions:") {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if let Some(dimensions) = parts.get(1) {
            let axis: Vec<&str> = dimensions.split('x').collect();
            if axis.len() == 2 {
                let width = axis[0].parse::<i64>().ok()?;
                let height = axis[1].parse::<i64>().ok()?;
                return Some((width, height));
            }
        }
    }
    None
}

async fn query_screen_dimensions() -> Result<(i64, i64), String> {
    let output = run_query("xdpyinfo", &[]).await?;
    parse_screen_dimensions(&output).ok_or_else(|| "failed to parse screen dimensions".to_string())
}

async fn query_maximized_state(window_id: &str) -> Result<Option<bool>, String> {
    let output = run_query("xprop", &["-id", window_id, "_NET_WM_STATE"]).await?;
    let lower = output.to_ascii_lowercase();

    if !lower.contains("_net_wm_state") {
        return Ok(None);
    }

    let maximized_vert = lower.contains("_net_wm_state_maximized_vert");
    let maximized_horz = lower.contains("_net_wm_state_maximized_horz");
    Ok(Some(maximized_vert && maximized_horz))
}

async fn query_minimized_state(window_id: &str) -> Result<Option<bool>, String> {
    let output = run_query("xprop", &["-id", window_id, "_NET_WM_STATE"]).await?;
    let lower = output.to_ascii_lowercase();

    if !lower.contains("_net_wm_state") {
        return Ok(None);
    }

    Ok(Some(lower.contains("_net_wm_state_hidden")))
}

fn geometry_matches_position(entry: &WindowEntry, x: i64, y: i64) -> bool {
    entry.x == Some(x) && entry.y == Some(y)
}

fn geometry_matches_size(entry: &WindowEntry, width: i64, height: i64) -> bool {
    entry.width == Some(width) && entry.height == Some(height)
}

fn side_by_side_already_tiled(entries: &[WindowEntry], windows: &[String], sw: i64, sh: i64) -> bool {
    if windows.len() < 2 {
        return false;
    }

    let half_width = sw / 2;
    let left = find_window_by_title(entries, &windows[0]);
    let right = find_window_by_title(entries, &windows[1]);

    match (left, right) {
        (Some(left), Some(right)) => {
            left.x == Some(0)
                && left.y == Some(0)
                && left.width == Some(half_width)
                && left.height == Some(sh)
                && right.x == Some(half_width)
                && right.y == Some(0)
                && right.width == Some(half_width)
                && right.height == Some(sh)
        }
        _ => false,
    }
}

fn grid_already_tiled(entries: &[WindowEntry], windows: &[String], sw: i64, sh: i64) -> bool {
    if windows.len() < 2 {
        return false;
    }

    let half_width = sw / 2;
    let half_height = sh / 2;
    let expected = [(0, 0), (half_width, 0), (0, half_height), (half_width, half_height)];

    for (index, title) in windows.iter().enumerate().take(4) {
        let Some(window) = find_window_by_title(entries, title) else {
            return false;
        };

        let (x, y) = expected[index];
        if window.x != Some(x)
            || window.y != Some(y)
            || window.width != Some(half_width)
            || window.height != Some(half_height)
        {
            return false;
        }
    }

    true
}

async fn apply_window_move(title: &str, x: i64, y: i64) -> Result<(), String> {
    let geometry = format!("0,{x},{y},-1,-1");
    run_apply("wmctrl", &["-r", title, "-e", &geometry]).await?;
    Ok(())
}

async fn apply_window_resize(title: &str, width: i64, height: i64) -> Result<(), String> {
    let geometry = format!("0,-1,-1,{width},{height}");
    run_apply("wmctrl", &["-r", title, "-e", &geometry]).await?;
    Ok(())
}

async fn apply_window_maximize(title: &str) -> Result<(), String> {
    run_apply(
        "wmctrl",
        &["-r", title, "-b", "add,maximized_vert,maximized_horz"],
    )
    .await?;
    Ok(())
}

async fn apply_window_minimize(title: &str) -> Result<(), String> {
    run_apply("xdotool", &["search", "--name", title, "windowminimize"]).await?;
    Ok(())
}

async fn apply_tile_side_by_side(windows: &[String], sw: i64, sh: i64) -> Result<(), String> {
    let half_width = sw / 2;

    run_apply(
        "wmctrl",
        &[
            "-r",
            &windows[0],
            "-b",
            "remove,maximized_vert,maximized_horz",
        ],
    )
    .await?;
    let left_geometry = format!("0,0,0,{half_width},{sh}");
    run_apply("wmctrl", &["-r", &windows[0], "-e", &left_geometry]).await?;

    run_apply(
        "wmctrl",
        &[
            "-r",
            &windows[1],
            "-b",
            "remove,maximized_vert,maximized_horz",
        ],
    )
    .await?;
    let right_geometry = format!("0,{half_width},0,{half_width},{sh}");
    run_apply("wmctrl", &["-r", &windows[1], "-e", &right_geometry]).await?;

    Ok(())
}

async fn apply_tile_grid(windows: &[String], sw: i64, sh: i64) -> Result<(), String> {
    let half_width = sw / 2;
    let half_height = sh / 2;
    let positions = [(0, 0), (half_width, 0), (0, half_height), (half_width, half_height)];

    for (index, title) in windows.iter().enumerate().take(4) {
        let (x, y) = positions[index];
        run_apply(
            "wmctrl",
            &["-r", title, "-b", "remove,maximized_vert,maximized_horz"],
        )
        .await?;

        let geometry = format!("0,{x},{y},{half_width},{half_height}");
        run_apply("wmctrl", &["-r", title, "-e", &geometry]).await?;
    }

    Ok(())
}

struct GetActiveWindow;

#[async_trait]
impl ToolHandler for GetActiveWindow {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let _input: EmptyInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("get_active_window not implemented for this OS");
        }

        let title = match run_query("xdotool", &["getactivewindow", "getwindowname"]).await {
            Ok(output) => output,
            Err(error) => return ToolResult::err(error),
        };

        let pid = run_query("xdotool", &["getactivewindow", "getwindowpid"])
            .await
            .unwrap_or_default();

        let window_id = run_query("xdotool", &["getactivewindow"])
            .await
            .unwrap_or_default();

        let wm_class = if window_id.is_empty() {
            String::new()
        } else {
            run_query("xprop", &["-id", &window_id, "WM_CLASS"])
                .await
                .map(|raw| parse_wm_class(&raw))
                .unwrap_or_default()
        };

        ToolResult::ok(serde_json::json!({
            "title": title,
            "pid": pid,
            "window_id": window_id,
            "wm_class": wm_class,
        }))
    }
}

struct MoveWindow;

#[async_trait]
impl ToolHandler for MoveWindow {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: MoveWindowInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("move_window not implemented for this OS");
        }

        let title = input.title.trim();
        if title.is_empty() {
            return ToolResult::err("title parameter is required");
        }

        match query_window_geometries().await {
            Ok(entries) => {
                if let Some(entry) = find_window_by_title(&entries, title) {
                    if geometry_matches_position(&entry, input.x, input.y) {
                        return ToolResult::ok(serde_json::json!({
                            "moved": title,
                            "x": input.x,
                            "y": input.y,
                            "changed": false,
                            "already_in_desired_state": true,
                        }));
                    }
                }
            }
            Err(error) => warn!("move_window pre-flight query failed: {error}"),
        }

        match apply_window_move(title, input.x, input.y).await {
            Ok(()) => ToolResult::ok(serde_json::json!({
                "moved": title,
                "x": input.x,
                "y": input.y,
                "changed": true,
                "already_in_desired_state": false,
            })),
            Err(error) => ToolResult::err(error),
        }
    }
}

struct ResizeWindow;

#[async_trait]
impl ToolHandler for ResizeWindow {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: ResizeWindowInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("resize_window not implemented for this OS");
        }

        let title = input.title.trim();
        if title.is_empty() {
            return ToolResult::err("title parameter is required");
        }

        if input.width <= 0 || input.height <= 0 {
            return ToolResult::err("width and height must be positive integers");
        }

        match query_window_geometries().await {
            Ok(entries) => {
                if let Some(entry) = find_window_by_title(&entries, title) {
                    if geometry_matches_size(&entry, input.width, input.height) {
                        return ToolResult::ok(serde_json::json!({
                            "resized": title,
                            "width": input.width,
                            "height": input.height,
                            "changed": false,
                            "already_in_desired_state": true,
                        }));
                    }
                }
            }
            Err(error) => warn!("resize_window pre-flight query failed: {error}"),
        }

        match apply_window_resize(title, input.width, input.height).await {
            Ok(()) => ToolResult::ok(serde_json::json!({
                "resized": title,
                "width": input.width,
                "height": input.height,
                "changed": true,
                "already_in_desired_state": false,
            })),
            Err(error) => ToolResult::err(error),
        }
    }
}

struct MaximizeWindow;

#[async_trait]
impl ToolHandler for MaximizeWindow {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: WindowMatchInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("maximize_window not implemented for this OS");
        }

        let title = input.title.trim();
        if title.is_empty() {
            return ToolResult::err("title parameter is required");
        }

        match query_window_id_by_title(title).await {
            Ok(Some(window_id)) => match query_maximized_state(&window_id).await {
                Ok(Some(true)) => {
                    return ToolResult::ok(serde_json::json!({
                        "maximized": title,
                        "window_id": window_id,
                        "changed": false,
                        "already_in_desired_state": true,
                    }));
                }
                Ok(Some(false)) | Ok(None) => {}
                Err(error) => warn!("maximize_window pre-flight state query failed: {error}"),
            },
            Ok(None) => {}
            Err(error) => warn!("maximize_window pre-flight window lookup failed: {error}"),
        }

        match apply_window_maximize(title).await {
            Ok(()) => ToolResult::ok(serde_json::json!({
                "maximized": title,
                "changed": true,
                "already_in_desired_state": false,
            })),
            Err(error) => ToolResult::err(error),
        }
    }
}

struct MinimizeWindow;

#[async_trait]
impl ToolHandler for MinimizeWindow {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: WindowMatchInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("minimize_window not implemented for this OS");
        }

        let title = input.title.trim();
        if title.is_empty() {
            return ToolResult::err("title parameter is required");
        }

        match query_window_id_by_title(title).await {
            Ok(Some(window_id)) => match query_minimized_state(&window_id).await {
                Ok(Some(true)) => {
                    return ToolResult::ok(serde_json::json!({
                        "minimized": title,
                        "window_id": window_id,
                        "changed": false,
                        "already_in_desired_state": true,
                    }));
                }
                Ok(Some(false)) | Ok(None) => {}
                Err(error) => warn!("minimize_window pre-flight state query failed: {error}"),
            },
            Ok(None) => {}
            Err(error) => warn!("minimize_window pre-flight window lookup failed: {error}"),
        }

        match apply_window_minimize(title).await {
            Ok(()) => ToolResult::ok(serde_json::json!({
                "minimized": title,
                "changed": true,
                "already_in_desired_state": false,
            })),
            Err(error) => ToolResult::err(error),
        }
    }
}

struct TileWindows;

#[async_trait]
impl ToolHandler for TileWindows {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: TileWindowsInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("tile_windows not implemented for this OS");
        }

        let windows: Vec<String> = input
            .windows
            .iter()
            .map(|title| title.trim().to_string())
            .filter(|title| !title.is_empty())
            .collect();

        if windows.len() < 2 {
            return ToolResult::err("at least 2 window titles required for tiling");
        }

        let (sw, sh) = query_screen_dimensions().await.unwrap_or((1920, 1080));
        let layout = input.layout.trim().to_ascii_lowercase();

        match query_window_geometries().await {
            Ok(entries) => {
                if layout == "side-by-side" && side_by_side_already_tiled(&entries, &windows, sw, sh) {
                    return ToolResult::ok(serde_json::json!({
                        "layout": "side-by-side",
                        "windows": windows,
                        "changed": false,
                        "already_in_desired_state": true,
                    }));
                }

                if layout == "grid" && grid_already_tiled(&entries, &windows, sw, sh) {
                    return ToolResult::ok(serde_json::json!({
                        "layout": "grid",
                        "windows": windows,
                        "changed": false,
                        "already_in_desired_state": true,
                    }));
                }
            }
            Err(error) => warn!("tile_windows pre-flight query failed: {error}"),
        }

        let apply_result = match layout.as_str() {
            "side-by-side" => apply_tile_side_by_side(&windows, sw, sh).await,
            "grid" => apply_tile_grid(&windows, sw, sh).await,
            _ => {
                return ToolResult::err(format!(
                    "unknown layout '{}'. Supported: side-by-side, grid",
                    input.layout
                ));
            }
        };

        match apply_result {
            Ok(()) => ToolResult::ok(serde_json::json!({
                "layout": layout,
                "windows": windows,
                "changed": true,
                "already_in_desired_state": false,
            })),
            Err(error) => ToolResult::err(error),
        }
    }
}

struct OpenUrl;

#[async_trait]
impl ToolHandler for OpenUrl {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: OpenUrlInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let url = input.url.trim();
        if url.is_empty() {
            return ToolResult::err("url parameter is required");
        }

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return ToolResult::err("url must start with http:// or https://");
        }

        let open_result = if cfg!(target_os = "linux") {
            run_apply_owned("xdg-open", vec![url.to_string()]).await
        } else if cfg!(target_os = "macos") {
            run_apply_owned("open", vec![url.to_string()]).await
        } else if cfg!(target_os = "windows") {
            run_apply_owned(
                "cmd",
                vec!["/C".into(), "start".into(), "".into(), url.to_string()],
            )
            .await
        } else {
            Err("open_url not implemented for this OS".to_string())
        };

        match open_result {
            Ok(_) => ToolResult::ok(serde_json::json!({ "opened": url })),
            Err(error) => ToolResult::err(format!("failed to open URL: {error}")),
        }
    }
}

struct ListWindows;

#[async_trait]
impl ToolHandler for ListWindows {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let _input: EmptyInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if !cfg!(target_os = "linux") {
            return ToolResult::err("list_windows not implemented for this OS");
        }

        match run_query("wmctrl", &["-l"]).await {
            Ok(output) => {
                let windows: Vec<serde_json::Value> = output
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| {
                        let parts: Vec<&str> = line.splitn(4, char::is_whitespace).collect();
                        serde_json::json!({
                            "id": parts.first().copied().unwrap_or_default(),
                            "desktop": parts.get(1).copied().unwrap_or_default(),
                            "title": parts.get(3).copied().unwrap_or_default().trim(),
                        })
                    })
                    .collect();

                ToolResult::ok(serde_json::json!({
                    "windows": windows,
                    "count": windows.len(),
                }))
            }
            Err(error) => ToolResult::err(error),
        }
    }
}

pub fn register(reg: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "get_active_window".into(),
                description: "Get the currently focused window title, PID, and application class"
                    .into(),
                category: "desktop".into(),
                default_tier: RiskLevel::Green,
                min_tier: "standard",
                parameters: vec![],
            },
            Arc::new(GetActiveWindow),
        ),
        (
            ToolDef {
                name: "list_windows".into(),
                description: "List all open windows with their titles and desktop numbers".into(),
                category: "desktop".into(),
                default_tier: RiskLevel::Green,
                min_tier: "standard",
                parameters: vec![],
            },
            Arc::new(ListWindows),
        ),
        (
            ToolDef {
                name: "move_window".into(),
                description: "Move a window to a specific position on screen".into(),
                category: "desktop".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "standard",
                parameters: vec![
                    param("title", "string", "Window title (partial match)", true),
                    param("x", "integer", "X position in pixels", true),
                    param("y", "integer", "Y position in pixels", true),
                ],
            },
            Arc::new(MoveWindow),
        ),
        (
            ToolDef {
                name: "resize_window".into(),
                description: "Resize a window to specific dimensions".into(),
                category: "desktop".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "standard",
                parameters: vec![
                    param("title", "string", "Window title (partial match)", true),
                    param("width", "integer", "Width in pixels", true),
                    param("height", "integer", "Height in pixels", true),
                ],
            },
            Arc::new(ResizeWindow),
        ),
        (
            ToolDef {
                name: "maximize_window".into(),
                description: "Maximize a window by title".into(),
                category: "desktop".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "standard",
                parameters: vec![param(
                    "title",
                    "string",
                    "Window title (partial match)",
                    true,
                )],
            },
            Arc::new(MaximizeWindow),
        ),
        (
            ToolDef {
                name: "minimize_window".into(),
                description: "Minimize a window by title".into(),
                category: "desktop".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "standard",
                parameters: vec![param(
                    "title",
                    "string",
                    "Window title (partial match)",
                    true,
                )],
            },
            Arc::new(MinimizeWindow),
        ),
        (
            ToolDef {
                name: "tile_windows".into(),
                description: "Arrange windows in a tiled layout (side-by-side or grid)".into(),
                category: "desktop".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "standard",
                parameters: vec![
                    param("windows", "array", "Window titles to tile", true),
                    param(
                        "layout",
                        "string",
                        "Layout: 'side-by-side' or 'grid'",
                        false,
                    ),
                ],
            },
            Arc::new(TileWindows),
        ),
        (
            ToolDef {
                name: "open_url".into(),
                description: "Open a URL in the default web browser".into(),
                category: "desktop".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param(
                    "url",
                    "string",
                    "URL to open (must start with http:// or https://)",
                    true,
                )],
            },
            Arc::new(OpenUrl),
        ),
    ];

    for (def, handler) in tools {
        reg.register(def, handler);
    }
}
