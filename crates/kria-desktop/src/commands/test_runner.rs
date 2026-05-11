use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestRunProfileRequest {
    pub mode: Option<String>,
    pub allow_destructive: Option<bool>,
    pub snapshot: Option<bool>,
    pub resume: Option<String>,
    pub from_zone: Option<String>,
    pub from_suite: Option<String>,
    pub from_test: Option<String>,
    pub test_groups: Option<Vec<String>>,
    pub vm_host: Option<String>,
    pub vm_port: Option<u16>,
    pub vm_user: Option<String>,
    pub vm_ssh_key: Option<String>,
    pub vm_hostkey_sha256: Option<String>,
    pub docker_fallback: Option<bool>,
    pub target_id: Option<String>,
    pub docker_container_id: Option<String>,
    pub continue_on_failure: Option<bool>,
}

/// A selectable test target — either a fleet VM or a Docker container.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestTargetItem {
    pub id: String,
    pub label: String,
    pub target_type: String, // "vm" | "docker"
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub ssh_key_path: Option<String>,
    pub last_verified_unix_ms: Option<u64>,
    pub state: Option<String>,
}

/// A running Docker container discoverable for testing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DockerContainerItem {
    pub container_id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub ports: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestCheckpointInfo {
    pub run_tag: String,
    pub suite_name: Option<String>,
    pub test_name: Option<String>,
    pub status: Option<String>,
    pub failure_reason: Option<String>,
    pub assertion_details: Option<String>,
    pub timestamp_unix_ms: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestFailureCategory {
    pub category: String,
    pub count: usize,
    pub examples: Vec<TestFailureExample>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestFailureExample {
    pub suite: String,
    pub test: String,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestRunStateView {
    pub running: bool,
    pub started_unix_ms: Option<u64>,
    pub pid: Option<u32>,
    pub mode: Option<String>,
    pub run_label: Option<String>,
    pub command: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestHistoryItem {
    pub run_label: String,
    pub report_path: String,
    pub modified_unix_ms: u64,
}

#[derive(Debug)]
struct ActiveRun {
    child: Child,
    started_unix_ms: u64,
    mode: String,
    run_label: String,
    command: String,
}

static ACTIVE_RUN: LazyLock<Arc<Mutex<Option<ActiveRun>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(None)));

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn workspace_root() -> PathBuf {
    // When launched from the Tauri desktop app, the CWD may be
    // crates/kria-desktop/ rather than the project root.  Walk up the tree
    // looking for the workspace Cargo.toml (contains "[workspace]").
    let start = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut candidate = start.clone();
    for _ in 0..10 {
        if let Ok(content) = std::fs::read_to_string(candidate.join("Cargo.toml")) {
            if content.contains("[workspace]") {
                return candidate;
            }
        }
        if !candidate.pop() {
            break;
        }
    }
    // Fallback: return CWD (original behavior)
    start
}

fn target_tests_dir() -> PathBuf {
    workspace_root().join("tests-logs")
}

fn canonicalize_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn ensure_within_tests_logs(path: &Path) -> Result<(), String> {
    let root = canonicalize_or_self(&target_tests_dir());
    let candidate = canonicalize_or_self(path);
    if candidate.starts_with(&root) {
        Ok(())
    } else {
        Err(format!(
            "Refusing to operate outside tests-logs: '{}'",
            candidate.display()
        ))
    }
}

fn normalize_mode(raw: Option<String>) -> String {
    let value = raw.unwrap_or_else(|| "FULL".to_string());
    match value.trim().to_ascii_uppercase().as_str() {
        "SMOKE" => "SMOKE".to_string(),
        "INFRA" => "INFRA".to_string(),
        "DESTRUCTIVE" | "OS" | "OSLEVEL" => "DESTRUCTIVE".to_string(),
        "APP" | "APPLOGIC" => "APPLOGIC".to_string(),
        "RELEASE" | "PROD" | "PRODUCTION" => "RELEASE".to_string(),
        _ => "FULL".to_string(),
    }
}

fn build_command_args(request: &TestRunProfileRequest) -> (String, Vec<String>) {
    let mode = normalize_mode(request.mode.clone());
    let mut args = vec!["kria-test".to_string(), "--mode".to_string(), mode.clone()];

    if let Some(resume) = request.resume.as_ref().filter(|v| !v.trim().is_empty()) {
        args.push("--resume".to_string());
        args.push(resume.trim().to_string());
    }
    if let Some(from_zone) = request.from_zone.as_ref().filter(|v| !v.trim().is_empty()) {
        args.push("--from-zone".to_string());
        args.push(from_zone.trim().to_string());
    }
    if let Some(from_suite) = request.from_suite.as_ref().filter(|v| !v.trim().is_empty()) {
        args.push("--from-suite".to_string());
        args.push(from_suite.trim().to_string());
    }

    (mode, args)
}

async fn emit_line(app: &AppHandle, stream: &str, line: String) {
    let _ = app.emit(
        "kria://tests/log_line",
        serde_json::json!({
            "stream": stream,
            "line": line,
            "timestamp_unix_ms": now_unix_ms(),
        }),
    );
}

fn latest_report_path(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path.file_name()?.to_string_lossy();
        if !file_name.starts_with("KRIA_TEST_REPORT_") || !file_name.ends_with(".md") {
            continue;
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        match &best {
            Some((prev, _)) if *prev >= modified => {}
            _ => best = Some((modified, path)),
        }
    }
    best.map(|(_, p)| p)
}

#[tauri::command]
pub async fn start_test_run(
    request: TestRunProfileRequest,
    app: AppHandle,
) -> Result<TestRunStateView, String> {
    let mut guard = ACTIVE_RUN.lock().await;
    if guard.is_some() {
        return Err("A test run is already active".to_string());
    }

    let (mode, args) = build_command_args(&request);
    let mut command = Command::new("cargo");
    command.args(&args);
    command.current_dir(workspace_root());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mode_normalized = normalize_mode(request.mode.clone());
    let destructive_mode = matches!(mode_normalized.as_str(), "DESTRUCTIVE" | "FULL" | "RELEASE");
    if request.allow_destructive.unwrap_or(false) || destructive_mode {
        command.env("KRIA_TEST_ALLOW_DESTRUCTIVE", "1");
        command.env("KRIA_DANGEROUS", "1");
    }
    if request.snapshot.unwrap_or(false) {
        command.env("KRIA_TEST_SNAPSHOT", "1");
    }
    if let Some(vm_host) = request.vm_host.as_ref().filter(|v| !v.trim().is_empty()) {
        command.env("KRIA_TEST_VM_HOST", vm_host.trim());
    }
    if let Some(vm_port) = request.vm_port {
        command.env("KRIA_TEST_VM_PORT", vm_port.to_string());
    }
    if let Some(vm_user) = request.vm_user.as_ref().filter(|v| !v.trim().is_empty()) {
        command.env("KRIA_TEST_VM_USER", vm_user.trim());
    }
    if let Some(vm_ssh_key) = request.vm_ssh_key.as_ref().filter(|v| !v.trim().is_empty()) {
        command.env("KRIA_TEST_VM_SSH_KEY", vm_ssh_key.trim());
    }
    if let Some(vm_hostkey) = request
        .vm_hostkey_sha256
        .as_ref()
        .filter(|v| !v.trim().is_empty())
    {
        command.env("KRIA_TEST_VM_HOSTKEY_SHA256", vm_hostkey.trim());
    }
    if request.docker_fallback.unwrap_or(false) {
        command.env("KRIA_TEST_USE_DOCKER_FALLBACK", "1");
    }
    if let Some(target_id) = request.target_id.as_ref().filter(|v| !v.trim().is_empty()) {
        command.env("KRIA_TEST_TARGET_ID", target_id.trim());
    }
    if let Some(container_id) = request
        .docker_container_id
        .as_ref()
        .filter(|v| !v.trim().is_empty())
    {
        command.env("KRIA_TEST_DOCKER_CONTAINER_ID", container_id.trim());
        command.env("KRIA_TEST_USE_DOCKER_FALLBACK", "1");
    }
    if request.continue_on_failure.unwrap_or(true) {
        command.env("KRIA_TEST_CONTINUE_ON_FAILURE", "1");
    }

    let run_label = format!("run-{}", now_unix_ms());
    let command_preview = format!("cargo {}", args.join(" "));
    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to start test run: {e}"))?;
    let pid = child.id();

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let started_unix_ms = now_unix_ms();
    *guard = Some(ActiveRun {
        child,
        started_unix_ms,
        mode: mode.clone(),
        run_label: run_label.clone(),
        command: command_preview.clone(),
    });
    drop(guard);

    if let Some(stdout_pipe) = stdout {
        let app_stdout = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stdout_pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                emit_line(&app_stdout, "stdout", line).await;
            }
        });
    }

    if let Some(stderr_pipe) = stderr {
        let app_stderr = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stderr_pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                emit_line(&app_stderr, "stderr", line).await;
            }
        });
    }

    let run_state = ACTIVE_RUN.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let maybe_finished = {
                let mut guard = run_state.lock().await;
                let Some(active) = guard.as_mut() else {
                    return;
                };
                match active.child.try_wait() {
                    Ok(Some(status)) => {
                        let exit_code = status.code().unwrap_or(-1);
                        let run_label = active.run_label.clone();
                        let mode = active.mode.clone();
                        *guard = None;
                        Some((run_label, mode, exit_code))
                    }
                    Ok(None) => None,
                    Err(_) => {
                        let run_label = active.run_label.clone();
                        let mode = active.mode.clone();
                        *guard = None;
                        Some((run_label, mode, -1))
                    }
                }
            };

            if let Some((run_label, mode, exit_code)) = maybe_finished {
                let report =
                    latest_report_path(&target_tests_dir()).map(|p| p.display().to_string());
                let _ = app.emit(
                    "kria://tests/run_finished",
                    serde_json::json!({
                        "run_label": run_label,
                        "mode": mode,
                        "exit_code": exit_code,
                        "report_path": report,
                        "finished_unix_ms": now_unix_ms(),
                    }),
                );
                return;
            }

            sleep(Duration::from_millis(500)).await;
        }
    });

    Ok(TestRunStateView {
        running: true,
        started_unix_ms: Some(started_unix_ms),
        pid,
        mode: Some(mode),
        run_label: Some(run_label),
        command: Some(command_preview),
    })
}

#[tauri::command]
pub async fn stop_test_run() -> Result<bool, String> {
    let mut guard = ACTIVE_RUN.lock().await;
    let Some(active) = guard.as_mut() else {
        return Ok(false);
    };
    active
        .child
        .kill()
        .await
        .map_err(|e| format!("Failed to stop active test run: {e}"))?;
    Ok(true)
}

#[tauri::command]
pub async fn get_test_run_state() -> Result<TestRunStateView, String> {
    let mut guard = ACTIVE_RUN.lock().await;
    if let Some(active) = guard.as_mut() {
        if let Ok(Some(_)) = active.child.try_wait() {
            *guard = None;
        }
    }
    let view = if let Some(active) = guard.as_ref() {
        TestRunStateView {
            running: true,
            started_unix_ms: Some(active.started_unix_ms),
            pid: active.child.id(),
            mode: Some(active.mode.clone()),
            run_label: Some(active.run_label.clone()),
            command: Some(active.command.clone()),
        }
    } else {
        TestRunStateView {
            running: false,
            started_unix_ms: None,
            pid: None,
            mode: None,
            run_label: None,
            command: None,
        }
    };
    Ok(view)
}

#[tauri::command]
pub async fn list_test_history(limit: Option<usize>) -> Result<Vec<TestHistoryItem>, String> {
    let max_items = limit.unwrap_or(50).clamp(1, 500);
    let mut items = Vec::new();
    let dir = target_tests_dir();
    let entries = std::fs::read_dir(&dir).map_err(|e| {
        format!(
            "Failed to read test history directory '{}': {e}",
            dir.display()
        )
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().map(|n| n.to_string_lossy().to_string()) {
            Some(name) => name,
            None => continue,
        };
        if !name.starts_with("KRIA_TEST_REPORT_") || !name.ends_with(".md") {
            continue;
        }
        let modified_unix_ms = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        items.push(TestHistoryItem {
            run_label: name
                .trim_start_matches("KRIA_TEST_REPORT_")
                .trim_end_matches(".md")
                .to_string(),
            report_path: path.display().to_string(),
            modified_unix_ms,
        });
    }

    items.sort_by(|a, b| b.modified_unix_ms.cmp(&a.modified_unix_ms));
    items.truncate(max_items);
    Ok(items)
}

#[tauri::command]
pub async fn read_test_report(report_path: String) -> Result<String, String> {
    let path = PathBuf::from(report_path);
    ensure_within_tests_logs(&path)?;
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read '{}': {e}", path.display()))
}

#[tauri::command]
pub async fn delete_test_report(report_path: String) -> Result<bool, String> {
    let path = PathBuf::from(report_path);
    ensure_within_tests_logs(&path)?;
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&path)
        .map_err(|e| format!("Failed to delete '{}': {e}", path.display()))?;
    Ok(true)
}

#[tauri::command]
pub async fn delete_all_test_logs() -> Result<bool, String> {
    let guard = ACTIVE_RUN.lock().await;
    if guard.is_some() {
        return Err("Cannot delete logs while a test run is active".to_string());
    }
    drop(guard);

    let root = target_tests_dir();
    if !root.exists() {
        return Ok(false);
    }

    let entries = std::fs::read_dir(&root)
        .map_err(|e| format!("Failed to list '{}': {e}", root.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        ensure_within_tests_logs(&path)?;
        if path.is_dir() {
            std::fs::remove_dir_all(&path)
                .map_err(|e| format!("Failed to delete directory '{}': {e}", path.display()))?;
        } else if path.is_file() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("Failed to delete file '{}': {e}", path.display()))?;
        }
    }
    let _ = std::fs::create_dir_all(root.join("eval_reports"));
    Ok(true)
}

#[allow(dead_code)]
#[tauri::command]
pub async fn get_test_checkpoint() -> Result<Option<TestCheckpointInfo>, String> {
    let dir = target_tests_dir();
    let run_tag = find_latest_run_tag(&dir)?;
    let run_dir = dir.join(format!("kria-test-{}", run_tag));
    let checkpoint_path = run_dir.join("checkpoint.json");

    if !checkpoint_path.exists() {
        return Ok(None);
    }

    let raw = std::fs::read_to_string(&checkpoint_path)
        .map_err(|e| format!("Failed to read checkpoint: {e}"))?;

    #[derive(serde::Deserialize)]
    struct CheckpointState {
        run_tag: String,
        per_test_checkpoint: Option<serde_json::Value>,
    }

    let checkpoint: CheckpointState =
        serde_json::from_str(&raw).map_err(|e| format!("Failed to parse checkpoint: {e}"))?;

    let info = if let Some(ptc) = checkpoint.per_test_checkpoint {
        TestCheckpointInfo {
            run_tag: checkpoint.run_tag,
            suite_name: ptc
                .get("suite_name")
                .and_then(|v| v.as_str())
                .map(String::from),
            test_name: ptc
                .get("test_name")
                .and_then(|v| v.as_str())
                .map(String::from),
            status: ptc.get("status").and_then(|v| v.as_str()).map(String::from),
            failure_reason: ptc
                .get("failure_reason")
                .and_then(|v| v.as_str())
                .map(String::from),
            assertion_details: ptc
                .get("assertion_details")
                .and_then(|v| v.as_str())
                .map(String::from),
            timestamp_unix_ms: ptc
                .get("timestamp_unix_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        }
    } else {
        TestCheckpointInfo {
            run_tag: checkpoint.run_tag,
            suite_name: None,
            test_name: None,
            status: None,
            failure_reason: None,
            assertion_details: None,
            timestamp_unix_ms: 0,
        }
    };

    Ok(Some(info))
}

#[allow(dead_code)]
fn find_latest_run_tag(dir: &Path) -> Result<String, String> {
    let mut latest: Option<(u64, String)> = None;
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("kria-test-") {
            if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                let ts = modified
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                match &latest {
                    Some((prev, _)) if *prev >= ts => {}
                    _ => latest = Some((ts, name.strip_prefix("kria-test-").unwrap().to_string())),
                }
            }
        }
    }
    latest
        .map(|(_, tag)| tag)
        .ok_or_else(|| "No test runs found".to_string())
}

#[allow(dead_code)]
#[tauri::command]
pub async fn get_failure_categories() -> Result<Vec<TestFailureCategory>, String> {
    let dir = target_tests_dir();
    let mut categories: std::collections::HashMap<String, Vec<TestFailureExample>> =
        std::collections::HashMap::new();

    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().map(|e| e == "md").unwrap_or(false)
            && path
                .file_name()
                .map(|n| n.to_string_lossy().contains("REPORT"))
                .unwrap_or(false)
        {
            if let Ok(content) = std::fs::read_to_string(&path) {
                parse_failure_categories(&content, &mut categories);
            }
        }
    }

    let result: Vec<TestFailureCategory> = categories
        .into_iter()
        .map(|(category, examples)| TestFailureCategory {
            category,
            count: examples.len(),
            examples,
        })
        .collect();

    Ok(result)
}

#[allow(dead_code)]
fn parse_failure_categories(
    content: &str,
    categories: &mut std::collections::HashMap<String, Vec<TestFailureExample>>,
) {
    // Parse markdown report for failure information
    let lines: Vec<&str> = content.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        if line.contains("Status: Failed") || line.contains("**FAIL**") {
            // Try to extract suite/test info
            if let Some(suite_line) = lines.get(i.saturating_sub(3)) {
                if suite_line.starts_with("###") {
                    let suite_name = suite_line.trim_start_matches("###").trim().to_string();
                    // Look for assertion details nearby
                    let mut reason = String::new();
                    if let Some(next_line) = lines.get(i + 1) {
                        if next_line.contains("reason:") || next_line.contains("Reason:") {
                            reason = next_line
                                .split(':')
                                .nth(1)
                                .map(|s| s.trim().to_string())
                                .unwrap_or_default();
                        }
                    }
                    // Categorize failure
                    let category = categorize_failure(&reason);
                    categories
                        .entry(category)
                        .or_default()
                        .push(TestFailureExample {
                            suite: suite_name,
                            test: String::new(),
                            reason,
                        });
                }
            }
        }
    }
}

#[allow(dead_code)]
fn categorize_failure(reason: &str) -> String {
    let reason_lower = reason.to_lowercase();
    if reason_lower.contains("hallucination") || reason_lower.contains("hallucinated") {
        "Hallucination".to_string()
    } else if reason_lower.contains("unavailable") || reason_lower.contains("cannot access") {
        "Unavailable".to_string()
    } else if reason_lower.contains("wrong data") || reason_lower.contains("incorrect") {
        "WrongData".to_string()
    } else if reason_lower.contains("timeout") || reason_lower.contains("timed out") {
        "Timeout".to_string()
    } else if reason_lower.contains("connection") || reason_lower.contains("network") {
        "NetworkError".to_string()
    } else if reason_lower.contains("crash") || reason_lower.contains("panic") {
        "Crash".to_string()
    } else if reason_lower.contains("assert") || reason_lower.contains("assertion") {
        "AssertionFailed".to_string()
    } else {
        "Other".to_string()
    }
}

/// List running Docker containers available for testing.
#[tauri::command]
pub async fn list_docker_containers() -> Result<Vec<DockerContainerItem>, String> {
    let output = Command::new("docker")
        .args([
            "ps",
            "--format",
            "{{.ID}}|{{.Names}}|{{.Image}}|{{.Status}}|{{.Ports}}",
        ])
        .output()
        .await
        .map_err(|e| format!("Docker not available: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("docker ps failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut containers = Vec::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(5, '|').collect();
        if parts.len() >= 4 {
            containers.push(DockerContainerItem {
                container_id: parts[0].trim().to_string(),
                name: parts[1].trim().to_string(),
                image: parts[2].trim().to_string(),
                status: parts[3].trim().to_string(),
                ports: parts.get(4).unwrap_or(&"").trim().to_string(),
            });
        }
    }
    Ok(containers)
}

/// List all available test targets — fleet VMs + Docker containers.
/// VMs are sorted by last_verified_unix_ms descending (most recent first).
/// Docker containers are appended after VMs.
#[tauri::command]
pub async fn list_test_targets() -> Result<Vec<TestTargetItem>, String> {
    let mut targets = Vec::new();

    // Load enrolled fleet VMs
    let registry_path = crate::commands::default_target_registry_path();
    if let Ok(registry) = crate::commands::load_fleet_enrollment_registry(&registry_path) {
        let mut vm_entries: Vec<TestTargetItem> = registry
            .targets
            .iter()
            .map(|t| TestTargetItem {
                id: format!("vm:{}", t.target_id),
                label: format!(
                    "🖥️ {} ({}@{}:{})",
                    t.display_name, t.username, t.host, t.port
                ),
                target_type: "vm".to_string(),
                host: Some(t.host.clone()),
                port: Some(t.port),
                username: Some(t.username.clone()),
                ssh_key_path: Some(t.ssh_private_key_path.clone()),
                last_verified_unix_ms: if t.last_verified_unix_ms > 0 {
                    Some(t.last_verified_unix_ms as u64)
                } else {
                    None
                },
                state: None,
            })
            .collect();

        // Sort by last_verified descending — most recently verified first
        vm_entries.sort_by(|a, b| {
            b.last_verified_unix_ms
                .unwrap_or(0)
                .cmp(&a.last_verified_unix_ms.unwrap_or(0))
        });
        targets.extend(vm_entries);
    }

    // Append Docker containers
    match list_docker_containers().await {
        Ok(containers) => {
            for c in containers {
                targets.push(TestTargetItem {
                    id: format!("docker:{}", c.container_id),
                    label: format!("🐳 {} ({})", c.name, c.image),
                    target_type: "docker".to_string(),
                    host: None,
                    port: None,
                    username: None,
                    ssh_key_path: None,
                    last_verified_unix_ms: None,
                    state: Some(c.status.clone()),
                });
            }
        }
        Err(_) => {
            // Docker not available — skip
        }
    }

    Ok(targets)
}
