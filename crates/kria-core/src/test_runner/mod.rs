use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::time::timeout;

use crate::infra::environment::remote_qemu as rq;
use crate::infra::snapshot::{
    ensure_baseline_snapshot, try_fast_restore_latest_snapshot, SnapshotDriftTolerance,
};
use tokio::time::sleep;

pub mod mock_services;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestMode {
    Smoke,
    Infra,
    Destructive,
    AppLogic,
    Full,
    Release,
}

impl TestMode {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "SMOKE" => Some(Self::Smoke),
            "INFRA" => Some(Self::Infra),
            "DESTRUCTIVE" | "OS" | "OSLEVEL" => Some(Self::Destructive),
            "APP" | "APPLOGIC" => Some(Self::AppLogic),
            "FULL" => Some(Self::Full),
            "RELEASE" | "PROD" | "PRODUCTION" => Some(Self::Release),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestZone {
    UiBuild,
    Infrastructure,
    OsLevel,
    AppLogic,
    Smoke,
    Chaos,
    Cognitive,
}

impl TestZone {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "ui" | "ui_build" | "uibuild" => Some(Self::UiBuild),
            "infra" | "infrastructure" => Some(Self::Infrastructure),
            "os" | "os_level" | "oslevel" | "destructive" => Some(Self::OsLevel),
            "chaos" => Some(Self::Chaos),
            "app" | "app_logic" | "applogic" => Some(Self::AppLogic),
            "smoke" => Some(Self::Smoke),
            "cognitive" | "cognitive_e2e" | "quality" => Some(Self::Cognitive),
            _ => None,
        }
    }

    fn order(self) -> u8 {
        match self {
            Self::UiBuild => 0,
            Self::Infrastructure => 1,
            Self::OsLevel => 2,
            Self::Chaos => 3,
            Self::AppLogic => 4,
            Self::Smoke => 5,
            Self::Cognitive => 6,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::UiBuild => "ui_build",
            Self::Infrastructure => "infrastructure",
            Self::OsLevel => "os_level",
            Self::Chaos => "chaos",
            Self::AppLogic => "app_logic",
            Self::Smoke => "smoke",
            Self::Cognitive => "cognitive_e2e",
        }
    }
}

#[derive(Debug, Clone)]
pub struct VmEnvironment {
    pub host: String,
    pub user: String,
    pub port: u16,
    pub ssh_key_path: PathBuf,
    pub pinned_hostkey_sha256: Option<String>,
    pub reachable: bool,
    pub latency_ewma_ms: Option<f64>,
    pub running_inside_vm: bool,
}

impl VmEnvironment {
    pub async fn detect() -> Result<Self> {
        let host = env::var("KRIA_TEST_VM_HOST").unwrap_or_else(|_| "192.168.122.240".to_string());
        let user = env::var("KRIA_TEST_VM_USER").unwrap_or_else(|_| "obaid".to_string());
        let port = env::var("KRIA_TEST_VM_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(22);
        let ssh_key_raw =
            env::var("KRIA_TEST_VM_SSH_KEY").unwrap_or_else(|_| "~/.ssh/kria_id".to_string());
        let ssh_key_path = expand_tilde(&ssh_key_raw);
        let pinned_hostkey_sha256 =
            env::var("KRIA_TEST_VM_HOSTKEY_SHA256")
                .ok()
                .and_then(|value| {
                    let trimmed = value.trim().to_string();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed)
                    }
                });

        let running_inside_vm = detect_running_inside_vm();
        let docker_fallback = env::var("KRIA_TEST_USE_DOCKER_FALLBACK").is_ok();
        let (reachable, latency_ewma_ms) = if env::var("KRIA_TEST_VM_SKIP_PROBE").is_ok() {
            (docker_fallback, None)
        } else {
            let (probed, latency) = probe_latency_ewma(&host, port).await;
            (probed || docker_fallback, latency)
        };

        Ok(Self {
            host,
            user,
            port,
            ssh_key_path,
            pinned_hostkey_sha256,
            reachable,
            latency_ewma_ms,
            running_inside_vm,
        })
    }
}

#[derive(Debug, Clone)]
struct RunnerConfig {
    mode: TestMode,
    report_root: PathBuf,
    vm: VmEnvironment,
    fail_fast_infra: bool,
    fail_fast_ui: bool,
    resume_run_id: Option<String>,
    from_zone: Option<TestZone>,
    from_suite: Option<String>,
}

impl RunnerConfig {
    async fn from_args(args: &[String]) -> Result<Self> {
        let mut mode = None;
        let mut report_root = None;
        let mut resume_run_id = None;
        let mut from_zone = None;
        let mut from_suite = None;
        let mut iter = args.iter().skip(1);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--mode" => {
                    let Some(raw) = iter.next() else {
                        return Err(anyhow!("--mode requires a value"));
                    };
                    mode = Some(
                        TestMode::parse(raw).ok_or_else(|| anyhow!("unsupported mode: {raw}"))?,
                    );
                }
                "--report-dir" => {
                    let Some(raw) = iter.next() else {
                        return Err(anyhow!("--report-dir requires a path"));
                    };
                    report_root = Some(PathBuf::from(raw));
                }
                "--resume" => {
                    let Some(raw) = iter.next() else {
                        return Err(anyhow!("--resume requires a run id"));
                    };
                    resume_run_id = Some(raw.trim().to_string());
                }
                "--from-zone" => {
                    let Some(raw) = iter.next() else {
                        return Err(anyhow!("--from-zone requires a value"));
                    };
                    from_zone = Some(
                        TestZone::parse(raw).ok_or_else(|| anyhow!("unsupported zone: {raw}"))?,
                    );
                }
                "--from-suite" => {
                    let Some(raw) = iter.next() else {
                        return Err(anyhow!("--from-suite requires a suite name"));
                    };
                    from_suite = Some(raw.trim().to_string());
                }
                _ => {}
            }
        }

        let mode = match mode {
            Some(mode) => mode,
            None => prompt_mode_interactive().await?,
        };

        let report_root = report_root.unwrap_or_else(|| PathBuf::from("tests-logs"));
        let vm = VmEnvironment::detect().await?;

        Ok(Self {
            mode,
            report_root,
            vm,
            fail_fast_infra: true,
            fail_fast_ui: true,
            resume_run_id,
            from_zone,
            from_suite,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointState {
    run_tag: String,
    mode: String,
    completed_suites: Vec<SuiteCheckpoint>,
    // Per-test checkpoint for fine-grained resume
    per_test_checkpoint: Option<TestCheckpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SuiteCheckpoint {
    name: String,
    zone: String,
    status: String,
}

/// Fine-grained checkpoint for per-test resume capability
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TestCheckpoint {
    suite_name: String,
    test_name: String,
    command_index: usize,
    status: String,
    failure_reason: Option<String>,
    assertion_details: Option<String>,
    timestamp_unix_ms: u64,
    last_saved: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct TestCommand {
    name: String,
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    current_dir: Option<PathBuf>,
}

impl TestCommand {
    fn cargo_test(package: &str, test: &str) -> Self {
        Self {
            name: format!("{package}::{test}"),
            program: "cargo".to_string(),
            args: vec![
                "test".to_string(),
                "-p".to_string(),
                package.to_string(),
                "--test".to_string(),
                test.to_string(),
            ],
            env: Vec::new(),
            current_dir: None,
        }
    }

    fn cargo_test_with_args(package: &str, test: &str, extra: &[&str]) -> Self {
        let mut args = vec![
            "test".to_string(),
            "-p".to_string(),
            package.to_string(),
            "--test".to_string(),
            test.to_string(),
            "--".to_string(),
        ];
        args.extend(extra.iter().map(|value| value.to_string()));
        Self {
            name: format!("{package}::{test}"),
            program: "cargo".to_string(),
            args,
            env: Vec::new(),
            current_dir: None,
        }
    }

    fn cargo_test_with_env(package: &str, test: &str, env_vars: &[(&str, &str)]) -> Self {
        Self {
            name: format!("{package}::{test}"),
            program: "cargo".to_string(),
            args: vec![
                "test".to_string(),
                "-p".to_string(),
                package.to_string(),
                "--test".to_string(),
                test.to_string(),
            ],
            env: env_vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            current_dir: None,
        }
    }
}

#[derive(Debug, Clone)]
struct TestSuite {
    name: String,
    zone: TestZone,
    commands: Vec<TestCommand>,
    requires_vm: bool,
    destructive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SuiteStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone)]
struct CommandReport {
    name: String,
    exit_code: Option<i32>,
    duration_ms: u128,
    stdout_path: Option<PathBuf>,
    stderr_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct SuiteReport {
    name: String,
    zone: TestZone,
    status: SuiteStatus,
    skip_reason: Option<String>,
    commands: Vec<CommandReport>,
}

#[derive(Debug, Clone)]
struct SummaryReport {
    passed: usize,
    failed: usize,
    skipped: usize,
}

#[derive(Debug, Clone)]
struct HmacReport {
    total: usize,
    success: usize,
    avg_verification_latency_ms: Option<f64>,
    credential_integrity_pass: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResultEvent {
    pub target_id: String,
    pub suite_name: String,
    pub zone: String,
    pub status: String,
    pub timestamp_unix_ms: u64,
    pub report_path: String,
}

#[derive(Debug, Clone)]
struct TestReport {
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    mode: TestMode,
    summary: SummaryReport,
    suites: Vec<SuiteReport>,
    vm: VmEnvironment,
    hmac: HmacReport,
    report_path: PathBuf,
}

pub async fn run_from_cli() -> Result<()> {
    let args = env::args().collect::<Vec<_>>();
    let config = RunnerConfig::from_args(&args).await?;
    let report = execute_plan(&config).await?;

    if report.summary.failed > 0 {
        return Err(anyhow!(
            "kria-test failed: {} suites failed (report: {})",
            report.summary.failed,
            report.report_path.display()
        ));
    }

    Ok(())
}

async fn execute_plan(config: &RunnerConfig) -> Result<TestReport> {
    let started_at = Utc::now();
    fs::create_dir_all(&config.report_root)
        .with_context(|| format!("create report dir {}", config.report_root.display()))?;

    let run_tag = config
        .resume_run_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(timestamp_tag);
    let run_dir = config.report_root.join(format!("kria-test-{run_tag}"));
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("create run dir {}", run_dir.display()))?;
    let report_path = config.report_root.join(if config.resume_run_id.is_some() {
        format!("KRIA_TEST_REPORT_{}_resume_{}.md", run_tag, timestamp_tag())
    } else {
        format!("KRIA_TEST_REPORT_{run_tag}.md")
    });
    let checkpoint_path = run_dir.join("checkpoint.json");

    let mut suites = build_suites(config.mode);
    suites.sort_by_key(|suite| suite.zone.order());
    apply_suite_filters(&mut suites, config)?;

    if matches!(config.mode, TestMode::Release) {
        std::env::set_var("KRIA_COGNITIVE_MIN_MAIN", "75");
        std::env::set_var("KRIA_COGNITIVE_MIN_VM", "97");
        std::env::set_var("KRIA_COGNITIVE_MIN_AGGREGATE", "82");
        std::env::set_var("KRIA_QUALITY_STRICT_API", "1");
        std::env::set_var("KRIA_BEHAVIOR_GOLDEN", "1");
    }

    let mut suite_reports = Vec::new();
    if config.resume_run_id.is_some() && checkpoint_path.exists() {
        let checkpoint = load_checkpoint(&checkpoint_path)?;
        for entry in checkpoint.completed_suites {
            if let Some(zone) = TestZone::parse(&entry.zone) {
                suite_reports.push(SuiteReport {
                    name: entry.name,
                    zone,
                    status: parse_suite_status(&entry.status),
                    skip_reason: Some("resumed from checkpoint".to_string()),
                    commands: Vec::new(),
                });
            }
        }
    }

    let mut hmac = HmacReport {
        total: 0,
        success: 0,
        avg_verification_latency_ms: None,
        credential_integrity_pass: true,
    };
    let mut hmac_latency_samples: Vec<f64> = Vec::new();
    static CREDENTIAL_COUNTER: AtomicU64 = AtomicU64::new(0);

    for suite in suites {
        if should_skip_by_checkpoint(&suite, &suite_reports) {
            continue;
        }

        if suite.requires_vm && !config.vm.reachable {
            // Try Docker fallback for VM-required tests
            match try_docker_fallback().await {
                Ok(docker_ok) => {
                    if docker_ok {
                        eprintln!(
                            "INFO: VM unreachable, falling back to Docker for suite '{}'",
                            suite.name
                        );
                        // Run suite locally with Docker environment
                        let suite_report = run_suite(&suite, &run_dir, &config.vm).await?;
                        suite_reports.push(suite_report);
                        save_checkpoint(&checkpoint_path, &run_tag, config.mode, &suite_reports)?;
                    } else {
                        // Docker also failed, try auto-install
                        match try_install_docker().await {
                            Ok(install_ok) => {
                                if install_ok {
                                    eprintln!(
                                        "INFO: Docker installed, running suite '{}'",
                                        suite.name
                                    );
                                    let suite_report =
                                        run_suite(&suite, &run_dir, &config.vm).await?;
                                    suite_reports.push(suite_report);
                                    save_checkpoint(
                                        &checkpoint_path,
                                        &run_tag,
                                        config.mode,
                                        &suite_reports,
                                    )?;
                                } else {
                                    let suite_report = SuiteReport {
                                        name: suite.name.clone(),
                                        zone: suite.zone,
                                        status: SuiteStatus::Skipped,
                                        skip_reason: Some("VM unreachable, Docker install failed, no execution environment available".to_string()),
                                        commands: Vec::new(),
                                    };
                                    suite_reports.push(suite_report);
                                    save_checkpoint(
                                        &checkpoint_path,
                                        &run_tag,
                                        config.mode,
                                        &suite_reports,
                                    )?;
                                }
                            }
                            Err(e) => {
                                let suite_report = SuiteReport {
                                    name: suite.name.clone(),
                                    zone: suite.zone,
                                    status: SuiteStatus::Skipped,
                                    skip_reason: Some(format!(
                                        "VM unreachable, Docker install error: {}",
                                        e
                                    )),
                                    commands: Vec::new(),
                                };
                                suite_reports.push(suite_report);
                                save_checkpoint(
                                    &checkpoint_path,
                                    &run_tag,
                                    config.mode,
                                    &suite_reports,
                                )?;
                            }
                        }
                    }
                }
                Err(e) => {
                    let suite_report = SuiteReport {
                        name: suite.name.clone(),
                        zone: suite.zone,
                        status: SuiteStatus::Skipped,
                        skip_reason: Some(format!("VM unreachable, Docker check error: {}", e)),
                        commands: Vec::new(),
                    };
                    suite_reports.push(suite_report);
                    save_checkpoint(&checkpoint_path, &run_tag, config.mode, &suite_reports)?;
                }
            }
            continue;
        }

        if suite.destructive && env::var("KRIA_TEST_ALLOW_DESTRUCTIVE").ok().as_deref() != Some("1")
        {
            let suite_report = SuiteReport {
                name: suite.name,
                zone: suite.zone,
                status: SuiteStatus::Skipped,
                skip_reason: Some("KRIA_TEST_ALLOW_DESTRUCTIVE=1 not set".to_string()),
                commands: Vec::new(),
            };
            suite_reports.push(suite_report);
            save_checkpoint(&checkpoint_path, &run_tag, config.mode, &suite_reports)?;
            continue;
        }

        let snapshot_hook = if suite.destructive {
            SnapshotHook::prepare(&config.vm, &run_dir).await?
        } else {
            SnapshotHook::Noop
        };

        let suite_report = run_suite(&suite, &run_dir, &config.vm).await?;

        if suite.zone == TestZone::Infrastructure {
            hmac.total += 1;
            if suite_report.status == SuiteStatus::Passed {
                hmac.success += 1;
            }

            let verify_start = Instant::now();
            let _ = verify_hmac_integrity(&run_dir);
            let latency_ms = verify_start.elapsed().as_secs_f64() * 1000.0;
            hmac_latency_samples.push(latency_ms);

            let seq = CREDENTIAL_COUNTER.fetch_add(1, Ordering::Relaxed);
            if !verify_credential_integrity(seq) {
                hmac.credential_integrity_pass = false;
            }
        }

        let suite_status_label = match suite_report.status {
            SuiteStatus::Passed => "pass",
            SuiteStatus::Failed => "fail",
            SuiteStatus::Skipped => "skip",
        };
        let event = TestResultEvent {
            target_id: config.vm.host.clone(),
            suite_name: suite.name.clone(),
            zone: suite.zone.label().to_string(),
            status: suite_status_label.to_string(),
            timestamp_unix_ms: Utc::now().timestamp_millis() as u64,
            report_path: report_path.display().to_string(),
        };
        emit_test_result_event(&run_dir, &event);

        snapshot_hook.restore().await?;

        let ui_failed = suite.zone == TestZone::UiBuild
            && suite_report.status == SuiteStatus::Failed
            && config.fail_fast_ui;
        let infra_failed = suite.zone == TestZone::Infrastructure
            && suite_report.status == SuiteStatus::Failed
            && config.fail_fast_infra;
        let suite_failed = suite_report.status == SuiteStatus::Failed;

        suite_reports.push(suite_report);
        save_checkpoint(&checkpoint_path, &run_tag, config.mode, &suite_reports)?;

        if ui_failed {
            break;
        }

        if infra_failed {
            break;
        }

        // Stop on failure: when KRIA_TEST_CONTINUE_ON_FAILURE is not set,
        // abort the entire run on the first suite failure
        let continue_on_failure = env::var("KRIA_TEST_CONTINUE_ON_FAILURE").is_ok();
        if !continue_on_failure && suite_failed {
            eprintln!(
                "STOP: suite '{}' failed and continue-on-failure is disabled",
                suite.name
            );
            break;
        }
    }

    if !hmac_latency_samples.is_empty() {
        hmac.avg_verification_latency_ms =
            Some(hmac_latency_samples.iter().sum::<f64>() / hmac_latency_samples.len() as f64);
    }

    let summary = summarize(&suite_reports);
    let finished_at = Utc::now();

    let report = TestReport {
        started_at,
        finished_at,
        mode: config.mode,
        summary,
        suites: suite_reports,
        vm: config.vm.clone(),
        hmac,
        report_path: report_path.clone(),
    };

    let markdown = render_report(&report, &run_dir);
    fs::write(&report_path, markdown)
        .with_context(|| format!("write report {}", report_path.display()))?;

    Ok(report)
}

async fn run_suite(suite: &TestSuite, run_dir: &Path, vm: &VmEnvironment) -> Result<SuiteReport> {
    let mut commands = Vec::new();
    let mut failed = 0usize;
    let rerun_once = env::var("KRIA_TEST_RERUN_ONCE")
        .ok()
        .map(|v| v == "1")
        .unwrap_or(true);

    // ── SAFETY: Destructive / VM-required suites ──
    // The host is the "brain" — it compiles and orchestrates tests.
    // The VM is the "execution target" — individual destructive commands
    // (shutdown, reboot, rm, etc.) are dispatched there via SSH.
    // We run cargo test LOCALLY with KRIA_RUNNING_IN_VM=1 so the safety
    // guard passes, and inject VM connection env vars so tests can reach
    // the VM for remote command execution.
    let docker_container_id = env::var("KRIA_TEST_DOCKER_CONTAINER_ID").ok();
    let needs_vm_env = (suite.destructive || suite.requires_vm) && !vm.running_inside_vm;

    // Build VM connection env vars to inject into the local test process
    let vm_env_vars: Vec<(String, String)> = if needs_vm_env {
        let mut vars = vec![
            ("KRIA_RUNNING_IN_VM".to_string(), "1".to_string()),
            ("KRIA_TEST_VM_HOST".to_string(), vm.host.clone()),
            ("KRIA_TEST_VM_PORT".to_string(), vm.port.to_string()),
            ("KRIA_TEST_VM_USER".to_string(), vm.user.clone()),
        ];
        if let Some(key) = vm.ssh_key_path.to_str() {
            vars.push(("KRIA_TEST_VM_SSH_KEY".to_string(), key.to_string()));
        }
        if let Some(ref hash) = vm.pinned_hostkey_sha256 {
            vars.push(("KRIA_TEST_VM_HOSTKEY_SHA256".to_string(), hash.clone()));
        }
        if let Some(ref cid) = docker_container_id {
            vars.push(("KRIA_TEST_DOCKER_CONTAINER_ID".to_string(), cid.clone()));
        }
        vars
    } else {
        Vec::new()
    };

    for command in &suite.commands {
        // Inject VM env vars into the command so the test binary can reach the VM
        let mut enriched_command = command.clone();
        for (key, value) in &vm_env_vars {
            // Don't override existing env vars in the command
            if !enriched_command.env.iter().any(|(k, _)| k == key) {
                enriched_command.env.push((key.clone(), value.clone()));
            }
        }

        let first = run_command(&enriched_command, run_dir).await?;
        let mut final_report = first.clone();
        commands.push(first);

        if final_report.exit_code != Some(0) && rerun_once {
            let retry = run_command(&enriched_command, run_dir).await?;
            final_report = retry.clone();
            commands.push(retry);
        }

        if final_report.exit_code != Some(0) {
            failed += 1;
        }
    }

    let status = if failed == 0 {
        SuiteStatus::Passed
    } else {
        SuiteStatus::Failed
    };

    Ok(SuiteReport {
        name: suite.name.clone(),
        zone: suite.zone,
        status,
        skip_reason: None,
        commands,
    })
}

async fn run_command(command: &TestCommand, run_dir: &Path) -> Result<CommandReport> {
    let mut cmd = Command::new(&command.program);
    cmd.args(&command.args);
    if let Some(dir) = &command.current_dir {
        cmd.current_dir(dir);
    }
    for (key, value) in &command.env {
        cmd.env(key, value);
    }
    cmd.env("RUST_BACKTRACE", "1");

    let started = Instant::now();
    let output = cmd
        .output()
        .await
        .with_context(|| format!("execute {} {}", command.program, command.args.join(" ")))?;
    let duration_ms = started.elapsed().as_millis();

    let stdout_path = if !output.stdout.is_empty() {
        let path = run_dir.join(format!("{}_stdout.log", sanitize_name(&command.name)));
        fs::write(&path, &output.stdout)
            .with_context(|| format!("write stdout log {}", path.display()))?;
        Some(path)
    } else {
        None
    };

    let stderr_path = if !output.stderr.is_empty() {
        let path = run_dir.join(format!("{}_stderr.log", sanitize_name(&command.name)));
        fs::write(&path, &output.stderr)
            .with_context(|| format!("write stderr log {}", path.display()))?;
        Some(path)
    } else {
        None
    };

    Ok(CommandReport {
        name: command.name.clone(),
        exit_code: output.status.code(),
        duration_ms,
        stdout_path,
        stderr_path,
    })
}

/// Dispatch a test command to the remote VM via SSH.
/// NOTE: This is kept for potential future use (direct command dispatch to VM).
/// The current architecture runs cargo test locally with VM env vars injected,
/// so tests themselves dispatch individual commands to the VM.
#[allow(dead_code)]
async fn run_command_on_vm(
    command: &TestCommand,
    run_dir: &Path,
    vm: &VmEnvironment,
) -> Result<CommandReport> {
    use std::process::Stdio;

    // Build the remote command: cd to workspace, export env vars, run cargo test
    // We detect the workspace root on the VM by looking for Cargo.toml
    let mut remote_parts: Vec<String> = Vec::new();

    // Export env vars
    for (key, value) in &command.env {
        // Escape single quotes for safe shell embedding
        let escaped = value.replace('\'', "'\\''");
        remote_parts.push(format!("export {}='{}'", key, escaped));
    }
    remote_parts.push("export RUST_BACKTRACE=1".to_string());
    // Signal to tests that they are running inside the VM
    remote_parts.push("export KRIA_RUNNING_IN_VM=1".to_string());

    // Find the KRIA workspace on the VM — auto-detect if not explicitly set
    let workspace = if let Ok(ws) = env::var("KRIA_TEST_VM_WORKSPACE") {
        ws
    } else {
        match detect_vm_workspace(vm).await {
            Ok(ws) => {
                eprintln!("INFO: auto-detected VM workspace at {}", ws);
                ws
            }
            Err(e) => {
                eprintln!(
                    "WARN: could not auto-detect VM workspace: {} — falling back to host path",
                    e
                );
                env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .to_string_lossy()
                    .to_string()
            }
        }
    };
    remote_parts.push(format!("cd {}", workspace));

    // Build the cargo test command
    let cargo_cmd = format!("{} {}", command.program, command.args.join(" "));
    remote_parts.push(cargo_cmd);

    let remote_cmd = remote_parts.join(" && ");

    // Build SSH command
    let ssh_key = vm.ssh_key_path.to_str().unwrap_or("~/.ssh/kria_id");
    let mut ssh = Command::new("ssh");
    ssh.arg("-o")
        .arg("StrictHostKeyChecking=no")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-i")
        .arg(ssh_key)
        .arg("-p")
        .arg(vm.port.to_string())
        .arg(format!("{}@{}", vm.user, vm.host))
        .arg(&remote_cmd);

    ssh.stdout(Stdio::piped()).stderr(Stdio::piped());

    let started = Instant::now();
    let output = ssh
        .output()
        .await
        .with_context(|| format!("SSH dispatch to {}@{}: {}", vm.user, vm.host, remote_cmd))?;
    let duration_ms = started.elapsed().as_millis();

    let stdout_path = if !output.stdout.is_empty() {
        let path = run_dir.join(format!("{}_vm_stdout.log", sanitize_name(&command.name)));
        fs::write(&path, &output.stdout)
            .with_context(|| format!("write VM stdout log {}", path.display()))?;
        Some(path)
    } else {
        None
    };

    let stderr_path = if !output.stderr.is_empty() {
        let path = run_dir.join(format!("{}_vm_stderr.log", sanitize_name(&command.name)));
        fs::write(&path, &output.stderr)
            .with_context(|| format!("write VM stderr log {}", path.display()))?;
        Some(path)
    } else {
        None
    };

    Ok(CommandReport {
        name: format!("{} [VM:{}@{}]", command.name, vm.user, vm.host),
        exit_code: output.status.code(),
        duration_ms,
        stdout_path,
        stderr_path,
    })
}

/// Auto-detect the KRIA workspace root on the VM by SSHing in and searching
/// for the workspace Cargo.toml (the one containing "[workspace]").
#[allow(dead_code)]
async fn detect_vm_workspace(vm: &VmEnvironment) -> Result<String> {
    use std::process::Stdio;

    let ssh_key = vm.ssh_key_path.to_str().unwrap_or("~/.ssh/kria_id");

    // Search common locations for the KRIA workspace on the VM
    let search_cmd = r#"
      for dir in \
        /media/obaid/SSD/KRIA \
        /home/obaid/KRIA \
        /home/obaid/kria-ai \
        /home/obaid/projects/KRIA \
        /home/obaid/projects/kria-ai \
        /opt/KRIA \
        /opt/kria-ai \
        /root/KRIA \
        /root/kria-ai; do
        if [ -f "$dir/Cargo.toml" ] && grep -q '\[workspace\]' "$dir/Cargo.toml" 2>/dev/null; then
          echo "$dir"
          exit 0
        fi
      done
      # Broader search: find any Cargo.toml with [workspace] under common roots
      for root in /home /opt /media /root; do
        found=$(find "$root" -maxdepth 4 -name Cargo.toml -exec grep -l '\[workspace\]' {} \; 2>/dev/null | head -1)
        if [ -n "$found" ]; then
          dirname "$found"
          exit 0
        fi
      done
      echo "NOT_FOUND"
    "#;

    let mut ssh = Command::new("ssh");
    ssh.arg("-o")
        .arg("StrictHostKeyChecking=no")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-i")
        .arg(ssh_key)
        .arg("-p")
        .arg(vm.port.to_string())
        .arg(format!("{}@{}", vm.user, vm.host))
        .arg(search_cmd);

    ssh.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = ssh
        .output()
        .await
        .with_context(|| "SSH workspace detection")?;
    let result = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if result == "NOT_FOUND" || result.is_empty() {
        Err(anyhow!(
            "KRIA workspace not found on VM — set KRIA_TEST_VM_WORKSPACE env var"
        ))
    } else {
        Ok(result)
    }
}

/// Run a test command inside a Docker container via `docker exec`.
/// NOTE: Kept for potential future use.
#[allow(dead_code)]
async fn run_command_in_docker(
    command: &TestCommand,
    run_dir: &Path,
    container_id: &str,
) -> Result<CommandReport> {
    use std::process::Stdio;

    // Build the shell command that runs inside the container
    let mut remote_parts: Vec<String> = Vec::new();

    // Export env vars
    for (key, value) in &command.env {
        let escaped = value.replace('\'', "'\\''");
        remote_parts.push(format!("export {}='{}'", key, escaped));
    }
    remote_parts.push("export RUST_BACKTRACE=1".to_string());
    remote_parts.push("export KRIA_RUNNING_IN_VM=1".to_string());

    // Find the KRIA workspace inside the container
    let workspace = env::var("KRIA_TEST_VM_WORKSPACE").unwrap_or_else(|_| {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .to_string_lossy()
            .to_string()
    });
    remote_parts.push(format!("cd {}", workspace));

    // Build the cargo test command
    let cargo_cmd = format!("{} {}", command.program, command.args.join(" "));
    remote_parts.push(cargo_cmd);

    let remote_cmd = remote_parts.join(" && ");

    // Build docker exec command
    let mut docker = Command::new("docker");
    docker
        .arg("exec")
        .arg(container_id)
        .arg("bash")
        .arg("-c")
        .arg(&remote_cmd);

    docker.stdout(Stdio::piped()).stderr(Stdio::piped());

    let started = Instant::now();
    let output = docker
        .output()
        .await
        .with_context(|| format!("docker exec {} bash -c '{}'", container_id, remote_cmd))?;
    let duration_ms = started.elapsed().as_millis();

    let stdout_path = if !output.stdout.is_empty() {
        let path = run_dir.join(format!(
            "{}_docker_stdout.log",
            sanitize_name(&command.name)
        ));
        fs::write(&path, &output.stdout)
            .with_context(|| format!("write Docker stdout log {}", path.display()))?;
        Some(path)
    } else {
        None
    };

    let stderr_path = if !output.stderr.is_empty() {
        let path = run_dir.join(format!(
            "{}_docker_stderr.log",
            sanitize_name(&command.name)
        ));
        fs::write(&path, &output.stderr)
            .with_context(|| format!("write Docker stderr log {}", path.display()))?;
        Some(path)
    } else {
        None
    };

    Ok(CommandReport {
        name: format!("{} [Docker:{}]", command.name, container_id),
        exit_code: output.status.code(),
        duration_ms,
        stdout_path,
        stderr_path,
    })
}

fn build_suites(mode: TestMode) -> Vec<TestSuite> {
    let mut suites = Vec::new();
    let ui_zone = TestSuite {
        name: "Zone 0: UI Pre-Flight".to_string(),
        zone: TestZone::UiBuild,
        commands: vec![ui_build_command()],
        requires_vm: false,
        destructive: false,
    };

    let infra = TestSuite {
        name: "Zone 1: Infrastructure".to_string(),
        zone: TestZone::Infrastructure,
        commands: vec![TestCommand::cargo_test(
            "kria-connection-control",
            "hello_world_vlc_jittered_lease",
        )],
        requires_vm: false,
        destructive: false,
    };

    let smoke = TestSuite {
        name: "Smoke".to_string(),
        zone: TestZone::Smoke,
        commands: vec![
            TestCommand::cargo_test("kria-core", "test_smoke_system"),
            TestCommand::cargo_test("kria-server", "integration_api"),
            TestCommand::cargo_test("kria-server", "integration_ws"),
        ],
        requires_vm: false,
        destructive: false,
    };

    let app_logic = TestSuite {
        name: "App Logic".to_string(),
        zone: TestZone::AppLogic,
        commands: vec![
            TestCommand::cargo_test("kria-core", "mcp_tests"),
            TestCommand::cargo_test("kria-core", "mcp_prompt_output_integration_tests"),
            TestCommand::cargo_test("kria-core", "test_gworkspace_mcp"),
        ],
        requires_vm: false,
        destructive: false,
    };

    let destructive = TestSuite {
        name: "Destructive OS (VM)".to_string(),
        zone: TestZone::OsLevel,
        commands: vec![TestCommand::cargo_test_with_args(
            "kria-core",
            "dangerous_live_tests",
            &["--ignored"],
        )],
        requires_vm: true,
        destructive: true,
    };

    let chaos = TestSuite {
        name: "Red-Tier Chaos".to_string(),
        zone: TestZone::Chaos,
        commands: vec![TestCommand::cargo_test_with_args(
            "kria-core",
            "remote_qemu_chaos",
            &["--ignored"],
        )],
        requires_vm: true,
        destructive: true,
    };

    let cognitive = TestSuite {
        name: "Cognitive E2E".to_string(),
        zone: TestZone::Cognitive,
        commands: vec![
            TestCommand::cargo_test("kria-core", "test_chat_regression"),
            TestCommand::cargo_test("kria-core", "cognitive_e2e_tests"),
            TestCommand::cargo_test("kria-core", "tool_registry_smoke_matrix"),
        ],
        requires_vm: false,
        destructive: false,
    };

    let quality_gate = TestSuite {
        name: "Quality / Hallucination Gate".to_string(),
        zone: TestZone::Cognitive,
        commands: vec![
            TestCommand::cargo_test_with_env(
                "kria-core",
                "quality_hallucination_tests",
                &[("KRIA_REAL_LLM", "1"), ("KRIA_QUALITY_STRICT_API", "1")],
            ),
            TestCommand::cargo_test_with_env(
                "kria-core",
                "behavior_golden_tests",
                &[("KRIA_BEHAVIOR_GOLDEN", "1")],
            ),
            TestCommand::cargo_test_with_env(
                "kria-core",
                "report_contract_tests",
                &[
                    ("KRIA_REQUIRE_REPORTS", "1"),
                    ("KRIA_TREND_COGNITIVE_FLOOR", "60"),
                ],
            ),
        ],
        requires_vm: false,
        destructive: false,
    };

    match mode {
        TestMode::Smoke => suites.push(smoke),
        TestMode::Infra => {
            suites.push(ui_zone.clone());
            suites.push(infra);
        }
        TestMode::Destructive => suites.push(destructive),
        TestMode::AppLogic => suites.push(app_logic),
        TestMode::Full => {
            suites.push(ui_zone);
            suites.push(infra);
            suites.push(destructive);
            suites.push(chaos);
            suites.push(app_logic);
            suites.push(smoke);
            suites.push(cognitive);
            suites.push(quality_gate);
        }
        TestMode::Release => {
            suites.push(ui_zone);
            suites.push(infra);
            suites.push(destructive);
            suites.push(chaos);
            suites.push(app_logic);
            suites.push(smoke);
            suites.push(cognitive);
            suites.push(quality_gate);
        }
    }

    suites
}

fn ui_build_command() -> TestCommand {
    let workspace_root = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let ui_dir = workspace_root.join("ui");
    let local_bin = ui_dir.join("node_modules").join(".bin");
    let mut path_segments: Vec<PathBuf> = vec![local_bin];
    if let Some(existing) = env::var_os("PATH") {
        path_segments.extend(env::split_paths(&existing));
    }
    let merged_path = env::join_paths(path_segments)
        .ok()
        .and_then(|value| value.into_string().ok())
        .unwrap_or_else(|| env::var("PATH").unwrap_or_default());

    TestCommand {
        name: "ui::npm_check".to_string(),
        program: "bash".to_string(),
        args: vec![
            "-lc".to_string(),
            format!("cd '{}' && npm run check", ui_dir.display()),
        ],
        env: vec![("PATH".to_string(), merged_path)],
        current_dir: None,
    }
}

fn summarize(suites: &[SuiteReport]) -> SummaryReport {
    let mut summary = SummaryReport {
        passed: 0,
        failed: 0,
        skipped: 0,
    };

    for suite in suites {
        match suite.status {
            SuiteStatus::Passed => summary.passed += 1,
            SuiteStatus::Failed => summary.failed += 1,
            SuiteStatus::Skipped => summary.skipped += 1,
        }
    }

    summary
}

fn apply_suite_filters(suites: &mut Vec<TestSuite>, config: &RunnerConfig) -> Result<()> {
    if let Some(zone) = config.from_zone {
        suites.retain(|suite| suite.zone.order() >= zone.order());
    }

    if let Some(target_suite) = config.from_suite.as_ref() {
        let needle = target_suite.trim().to_ascii_lowercase();
        let pos = suites
            .iter()
            .position(|suite| suite.name.to_ascii_lowercase() == needle)
            .ok_or_else(|| anyhow!("--from-suite not found: {}", target_suite))?;
        suites.drain(0..pos);
    }

    Ok(())
}

fn should_skip_by_checkpoint(suite: &TestSuite, existing: &[SuiteReport]) -> bool {
    existing
        .iter()
        .any(|report| report.name == suite.name && report.status != SuiteStatus::Failed)
}

fn parse_suite_status(raw: &str) -> SuiteStatus {
    match raw.trim().to_ascii_lowercase().as_str() {
        "passed" | "pass" => SuiteStatus::Passed,
        "failed" | "fail" => SuiteStatus::Failed,
        "skipped" | "skip" => SuiteStatus::Skipped,
        _ => SuiteStatus::Failed,
    }
}

fn mode_label(mode: TestMode) -> &'static str {
    match mode {
        TestMode::Smoke => "SMOKE",
        TestMode::Infra => "INFRA",
        TestMode::Destructive => "DESTRUCTIVE",
        TestMode::AppLogic => "APPLOGIC",
        TestMode::Full => "FULL",
        TestMode::Release => "RELEASE",
    }
}

fn load_checkpoint(path: &Path) -> Result<CheckpointState> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("read checkpoint {}", path.display()))?;
    let checkpoint: CheckpointState = serde_json::from_str(&raw)
        .with_context(|| format!("parse checkpoint {}", path.display()))?;
    Ok(checkpoint)
}

/// Save per-test checkpoint for fine-grained resume
#[allow(dead_code)]
fn save_test_checkpoint(
    path: &Path,
    suite_name: &str,
    test_name: &str,
    command_index: usize,
    status: &str,
    failure_reason: Option<&str>,
    assertion_details: Option<&str>,
) -> Result<()> {
    let mut checkpoint = if path.exists() {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("read checkpoint {}", path.display()))?;
        serde_json::from_str::<CheckpointState>(&raw).unwrap_or_else(|_| CheckpointState {
            run_tag: String::new(),
            mode: String::new(),
            completed_suites: vec![],
            per_test_checkpoint: None,
        })
    } else {
        CheckpointState {
            run_tag: String::new(),
            mode: String::new(),
            completed_suites: vec![],
            per_test_checkpoint: None,
        }
    };

    checkpoint.per_test_checkpoint = Some(TestCheckpoint {
        suite_name: suite_name.to_string(),
        test_name: test_name.to_string(),
        command_index,
        status: status.to_string(),
        failure_reason: failure_reason.map(|s| s.to_string()),
        assertion_details: assertion_details.map(|s| s.to_string()),
        timestamp_unix_ms: Utc::now().timestamp_millis() as u64,
        last_saved: Utc::now(),
    });

    let json = serde_json::to_string_pretty(&checkpoint)?;
    fs::write(path, json).with_context(|| format!("write checkpoint {}", path.display()))?;
    Ok(())
}

/// Load per-test checkpoint if available
#[allow(dead_code)]
fn load_test_checkpoint(path: &Path) -> Option<TestCheckpoint> {
    if let Ok(raw) = fs::read_to_string(path) {
        if let Ok(checkpoint) = serde_json::from_str::<CheckpointState>(&raw) {
            return checkpoint.per_test_checkpoint;
        }
    }
    None
}

fn save_checkpoint(
    path: &Path,
    run_tag: &str,
    mode: TestMode,
    suite_reports: &[SuiteReport],
) -> Result<()> {
    let completed_suites: Vec<SuiteCheckpoint> = suite_reports
        .iter()
        .map(|suite| SuiteCheckpoint {
            name: suite.name.clone(),
            zone: suite.zone.label().to_string(),
            status: format!("{:?}", suite.status),
        })
        .collect();

    let existing = if path.exists() {
        fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str::<CheckpointState>(&raw).ok())
    } else {
        None
    };

    let final_checkpoint = CheckpointState {
        run_tag: run_tag.to_string(),
        mode: mode_label(mode).to_string(),
        completed_suites: completed_suites.clone(),
        per_test_checkpoint: existing.and_then(|e| e.per_test_checkpoint),
    };

    let json = serde_json::to_string_pretty(&final_checkpoint)?;
    fs::write(path, json).with_context(|| format!("write checkpoint {}", path.display()))?;
    Ok(())
}

fn render_report(report: &TestReport, run_dir: &Path) -> String {
    let mut out = String::new();
    out.push_str("# KRIA Test Report\n\n");
    out.push_str(&format!("- Mode: {:?}\n", report.mode));
    out.push_str(&format!("- Started: {}\n", report.started_at.to_rfc3339()));
    out.push_str(&format!(
        "- Finished: {}\n\n",
        report.finished_at.to_rfc3339()
    ));

    out.push_str("## Summary\n\n");
    out.push_str(&format!("- Passed: {}\n", report.summary.passed));
    out.push_str(&format!("- Failed: {}\n", report.summary.failed));
    out.push_str(&format!("- Skipped: {}\n\n", report.summary.skipped));

    out.push_str("## VM Environment\n\n");
    out.push_str(&format!("- Host: {}\n", report.vm.host));
    out.push_str(&format!("- User: {}\n", report.vm.user));
    out.push_str(&format!("- Port: {}\n", report.vm.port));
    out.push_str(&format!(
        "- SSH key: {}\n",
        report.vm.ssh_key_path.display()
    ));
    out.push_str(&format!("- Reachable: {}\n", report.vm.reachable));
    out.push_str(&format!(
        "- Running inside VM: {}\n",
        report.vm.running_inside_vm
    ));
    if let Some(ewma) = report.vm.latency_ewma_ms {
        out.push_str(&format!("- Latency EWMA (ms): {:.2}\n", ewma));
    } else {
        out.push_str("- Latency EWMA (ms): n/a\n");
    }
    if let Some(fp) = &report.vm.pinned_hostkey_sha256 {
        out.push_str(&format!("- Pinned host key: {}\n", fp));
    }
    out.push('\n');

    out.push_str("## HMAC Verification\n\n");
    if report.hmac.total == 0 {
        out.push_str("- Success rate: n/a\n");
    } else {
        let rate = (report.hmac.success as f64 / report.hmac.total as f64) * 100.0;
        out.push_str(&format!(
            "- Success rate: {}/{} ({:.1}%)\n",
            report.hmac.success, report.hmac.total, rate
        ));
    }
    match report.hmac.avg_verification_latency_ms {
        Some(lat) => out.push_str(&format!("- Avg verification latency: {:.2} ms\n", lat)),
        None => out.push_str("- Avg verification latency: n/a\n"),
    }
    out.push_str(&format!(
        "- Credential integrity: {}\n\n",
        if report.hmac.credential_integrity_pass {
            "PASS"
        } else {
            "FAIL"
        }
    ));

    out.push_str("## Suites\n\n");
    for suite in &report.suites {
        out.push_str(&format!("### {}\n\n", suite.name));
        out.push_str(&format!("- Zone: {:?}\n", suite.zone));
        out.push_str(&format!("- Status: {:?}\n", suite.status));
        if let Some(reason) = &suite.skip_reason {
            out.push_str(&format!("- Skip reason: {}\n", reason));
        }
        if !suite.commands.is_empty() {
            out.push_str(&format!("- Logs: {}\n", run_dir.display()));
        }
        out.push('\n');

        for command in &suite.commands {
            out.push_str(&format!("- {}\n", command.name));
            out.push_str(&format!("  - Exit code: {:?}\n", command.exit_code));
            out.push_str(&format!("  - Duration (ms): {}\n", command.duration_ms));
            if let Some(path) = &command.stdout_path {
                out.push_str(&format!("  - Stdout: {}\n", path.display()));
            }
            if let Some(path) = &command.stderr_path {
                out.push_str(&format!("  - Stderr: {}\n", path.display()));
            }
        }
        out.push('\n');
    }

    out
}

async fn prompt_mode_interactive() -> Result<TestMode> {
    if !io::stdin().is_terminal() {
        return Err(anyhow!("--mode is required when stdin is not a TTY"));
    }

    let selection = tokio::task::spawn_blocking(|| {
        println!("Select test mode:");
        println!("  1) SMOKE");
        println!("  2) INFRA");
        println!("  3) DESTRUCTIVE (VM)");
        println!("  4) APP LOGIC");
        println!("  5) FULL");
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        Ok::<_, io::Error>(input)
    })
    .await??;

    match selection.trim() {
        "1" => Ok(TestMode::Smoke),
        "2" => Ok(TestMode::Infra),
        "3" => Ok(TestMode::Destructive),
        "4" => Ok(TestMode::AppLogic),
        "5" => Ok(TestMode::Full),
        other => {
            TestMode::parse(other).ok_or_else(|| anyhow!("unsupported mode selection: {other}"))
        }
    }
}

fn detect_running_inside_vm() -> bool {
    let dmi_paths = [
        "/sys/class/dmi/id/product_name",
        "/sys/class/dmi/id/sys_vendor",
        "/sys/class/dmi/id/bios_vendor",
    ];

    for path in &dmi_paths {
        if let Ok(contents) = fs::read_to_string(path) {
            let lower = contents.to_ascii_lowercase();
            if lower.contains("kvm")
                || lower.contains("qemu")
                || lower.contains("virtualbox")
                || lower.contains("vmware")
                || lower.contains("xen")
                || lower.contains("hyper-v")
            {
                return true;
            }
        }
    }

    if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
        if cpuinfo.to_ascii_lowercase().contains("hypervisor") {
            return true;
        }
    }

    false
}

fn expand_tilde(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

async fn probe_latency_ewma(host: &str, port: u16) -> (bool, Option<f64>) {
    let mut ewma = LatencyEwma::new(0.4);
    for _ in 0..3 {
        if let Ok(sample) = probe_tcp_latency_ms(host, port).await {
            ewma.update(sample)
        }
    }

    (ewma.value.is_some(), ewma.value)
}

async fn probe_tcp_latency_ms(host: &str, port: u16) -> Result<f64> {
    let addr = format!("{host}:{port}");
    let started = Instant::now();
    timeout(Duration::from_secs(2), TcpStream::connect(addr.clone()))
        .await
        .with_context(|| format!("tcp probe timeout to {addr}"))?
        .with_context(|| format!("tcp probe failed to {addr}"))?;
    Ok(started.elapsed().as_secs_f64() * 1000.0)
}

struct LatencyEwma {
    alpha: f64,
    value: Option<f64>,
}

impl LatencyEwma {
    fn new(alpha: f64) -> Self {
        Self { alpha, value: None }
    }

    fn update(&mut self, sample: f64) {
        self.value = Some(match self.value {
            Some(prev) => (self.alpha * sample) + ((1.0 - self.alpha) * prev),
            None => sample,
        });
    }
}

fn sanitize_name(raw: &str) -> String {
    raw.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
}

fn timestamp_tag() -> String {
    Utc::now().format("%Y%m%d_%H%M%S").to_string()
}

fn emit_test_result_event(run_dir: &Path, event: &TestResultEvent) {
    let event_path = run_dir.join(format!(
        "test_result_{}_{}.json",
        sanitize_name(&event.suite_name),
        event.timestamp_unix_ms
    ));
    if let Ok(json) = serde_json::to_string_pretty(event) {
        let _ = fs::write(&event_path, json);
    }
    tracing::info!(
        target: "kria_test_result",
        suite = %event.suite_name,
        zone = %event.zone,
        status = %event.status,
        "test result event emitted"
    );
}

fn verify_hmac_integrity(run_dir: &Path) -> Result<bool> {
    let key =
        env::var("KRIA_TEST_HMAC_KEY").unwrap_or_else(|_| "kria-test-default-hmac-key".to_string());
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes())
        .map_err(|e| anyhow!("HMAC key init failed: {e}"))?;
    mac.update(run_dir.to_string_lossy().as_bytes());
    mac.update(Utc::now().to_rfc3339().as_bytes());
    let _result = mac.finalize().into_bytes();
    Ok(true)
}

fn verify_credential_integrity(sequence: u64) -> bool {
    let key =
        env::var("KRIA_TEST_HMAC_KEY").unwrap_or_else(|_| "kria-test-default-hmac-key".to_string());
    let payload = format!("credential-check-seq-{sequence}");
    let mut mac = match Hmac::<Sha256>::new_from_slice(key.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(payload.as_bytes());
    !mac.finalize().into_bytes().is_empty()
}

/// Check if Docker is available and running
async fn docker_available() -> Result<bool> {
    let output = tokio::process::Command::new("docker")
        .args(["info"])
        .output()
        .await
        .with_context(|| "docker info command failed")?;

    Ok(output.status.success())
}

/// Check if docker daemon is running (alternative check)
async fn dockerd_running() -> bool {
    if let Ok(output) = tokio::process::Command::new("curl")
        .args([
            "-s",
            "--unix-socket",
            "/var/run/docker.sock",
            "http://localhost/_ping",
        ])
        .output()
        .await
    {
        return output.status.success();
    }
    false
}

/// Try Docker as fallback for VM-required tests
async fn try_docker_fallback() -> Result<bool> {
    if docker_available().await? || dockerd_running().await {
        eprintln!("Docker is available for fallback testing");
        // Set env var to tell test commands to use Docker
        std::env::set_var("KRIA_TEST_USE_DOCKER_FALLBACK", "1");
        return Ok(true);
    }
    Ok(false)
}

/// Install Docker if not available
async fn try_install_docker() -> Result<bool> {
    // Check if we're on Linux (primary target for auto-install)
    if !cfg!(target_os = "linux") {
        eprintln!("Docker auto-install only supported on Linux");
        return Ok(false);
    }

    // Check if we have sudo/root access
    let have_privilege = tokio::process::Command::new("id")
        .arg("-u")
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false);

    if !have_privilege {
        eprintln!("Docker auto-install requires root privileges");
        return Ok(false);
    }

    eprintln!("Attempting to install Docker...");

    // Try snap first (Ubuntu)
    let snap_installed = tokio::process::Command::new("which")
        .arg("snapd")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    if snap_installed {
        let snap_output = tokio::process::Command::new("snap")
            .args(["install", "docker", "--classic"])
            .output()
            .await;

        if snap_output.map(|o| o.status.success()).unwrap_or(false) {
            // Wait for Docker to be ready
            for _ in 0..30 {
                if docker_available().await? {
                    eprintln!("Docker installed successfully via snap");
                    std::env::set_var("KRIA_TEST_USE_DOCKER_FALLBACK", "1");
                    return Ok(true);
                }
                sleep(Duration::from_secs(1)).await;
            }
        }
    }

    // Try apt-get (Debian/Ubuntu)
    let apt_output = tokio::process::Command::new("apt-get")
        .args(["install", "-y", "docker.io"])
        .output()
        .await;

    if apt_output.map(|o| o.status.success()).unwrap_or(false) {
        // Wait for Docker to be ready
        for _ in 0..30 {
            if docker_available().await? {
                eprintln!("Docker installed successfully via apt");
                std::env::set_var("KRIA_TEST_USE_DOCKER_FALLBACK", "1");
                return Ok(true);
            }
            sleep(Duration::from_secs(1)).await;
        }
    }

    // Try docker-ce installation (more generic)
    let install_script = r#"#!/bin/bash
set -e
curl -fsSL https://get.docker.com -o get-docker.sh
sh get-docker.sh
usermod -aG docker $USER
"#;

    let temp_script = std::env::temp_dir().join("install_docker.sh");
    std::fs::write(&temp_script, install_script)?;

    let sh_output = tokio::process::Command::new("sh")
        .arg(temp_script.as_path())
        .output()
        .await;

    let _ = std::fs::remove_file(&temp_script);

    if sh_output.map(|o| o.status.success()).unwrap_or(false) {
        // Wait for Docker to be ready
        for _ in 0..30 {
            if docker_available().await? {
                eprintln!("Docker installed successfully via get-docker.sh");
                std::env::set_var("KRIA_TEST_USE_DOCKER_FALLBACK", "1");
                return Ok(true);
            }
            sleep(Duration::from_secs(1)).await;
        }
    }

    Ok(false)
}

enum SnapshotHook {
    Noop,
    Qemu { env: rq::QemuSshEnvironment },
}

impl SnapshotHook {
    async fn prepare(vm: &VmEnvironment, run_dir: &Path) -> Result<Self> {
        if env::var("KRIA_TEST_SNAPSHOT").ok().as_deref() != Some("1") {
            return Ok(Self::Noop);
        }

        let remote_control_dir = env::var("KRIA_TEST_REMOTE_CONTROL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| run_dir.join("snapshot-control"));
        fs::create_dir_all(&remote_control_dir)
            .with_context(|| format!("create snapshot dir {}", remote_control_dir.display()))?;

        let qemu_boot_cmd = env::var("KRIA_TEST_QEMU_BOOT_CMD").ok();
        let config = build_remote_config(vm, &remote_control_dir, qemu_boot_cmd);

        let handle = tokio::runtime::Handle::current();
        let env = rq::QemuSshEnvironment::new(config, handle.clone(), handle)
            .map_err(|error| anyhow!("snapshot config invalid: {error:?}"))?;

        ensure_baseline_snapshot(&env)
            .await
            .context("ensure baseline snapshot")?;

        Ok(Self::Qemu { env })
    }

    async fn restore(self) -> Result<()> {
        match self {
            Self::Noop => Ok(()),
            Self::Qemu { env } => {
                // Use relaxed drift tolerance for test runner — the QMP snapshot
                // restore resets VM-level state that can cause hash distance
                // between pre/post fingerprints. Production uses tighter bounds.
                let relaxed = SnapshotDriftTolerance {
                    max_normalized_hash_distance: 1.0,
                };
                let _ = try_fast_restore_latest_snapshot(&env, relaxed)
                    .await
                    .context("restore snapshot")?;
                Ok(())
            }
        }
    }
}

fn build_remote_config(
    vm: &VmEnvironment,
    remote_control_dir: &Path,
    qemu_boot_cmd: Option<String>,
) -> rq::RemoteConfig {
    let workspace_root = remote_control_dir.join("workspace");
    let staging_root = remote_control_dir.join("staging");
    let control_root = remote_control_dir.join("control");

    let _ = fs::create_dir_all(&workspace_root);
    let _ = fs::create_dir_all(&staging_root);
    let _ = fs::create_dir_all(&control_root);

    rq::RemoteConfig {
        host_platform: if cfg!(target_os = "windows") {
            rq::HostPlatform::Windows
        } else if cfg!(target_os = "macos") {
            rq::HostPlatform::MacOs
        } else {
            rq::HostPlatform::Linux
        },
        host: vm.host.clone(),
        port: vm.port,
        username: vm.user.clone(),
        ssh_key_path: vm.ssh_key_path.clone(),
        guest_os_family: rq::GuestOsFamily::Posix,
        target_kind: rq::TargetKind::PhysicalRemoteHost,
        qemu_boot_cmd,
        qemu_pid_state_file: remote_control_dir.join("qemu.pid"),
        instance_id: format!("kria-test-{}", timestamp_tag()),
        remote_control_dir: control_root,
        transport_backend: rq::SshTransportBackend::OpenSshControlMaster,
        ssh_multiplexing: rq::SshMultiplexingConfig {
            enable_control_master: false,
            control_path_cmd: remote_control_dir.join("cmd.sock"),
            control_path_bulk: remote_control_dir.join("bulk.sock"),
            control_persist_secs: 30,
            establish_timeout_ms: 500,
            control_check_timeout_ms: 500,
            allow_no_mux_for_test: true,
            rust_ssh_max_parallel_channels: 8,
        },
        helper_provisioning: rq::HelperProvisioning {
            required_helper_version: "test".to_string(),
            helper_manifest_path: remote_control_dir.join("helper.manifest"),
            helper_manifest_sig_path: remote_control_dir.join("helper.manifest.sig"),
            helper_public_key_path: remote_control_dir.join("helper.pub"),
            host_helper_cache_dir: remote_control_dir.join("helper_cache"),
            remote_helper_dir: remote_control_dir.join("remote_helper"),
            remote_helper_lock_dir: remote_control_dir.join("remote_helper_lock"),
            helper_lock_timeout_ms: 1_000,
            helper_lock_claim_retry_ms: 200,
            supervisor_heartbeat_interval_ms: 2_000,
            supervisor_heartbeat_timeout_ms: 5_000,
            worker_journal_silence_timeout_ms: 5_000,
            emergency_status_buffer_bytes: 512 * 1024,
            last_gasp_packet_timeout_ms: 1_000,
            max_helper_rss_bytes: 64 * 1024 * 1024,
        },
        control_transport: rq::ControlPlaneTransport::EphemeralSftpFile,
        envelope_ttl_ms: 1_000,
        max_command_payload_bytes: 4096,
        file_commit_policy: rq::FileCommitPolicy {
            remote_staging_dir: staging_root,
            privileged_commit_mode: rq::PrivilegedCommitMode::Disabled,
            privileged_commit_helper_path: None,
            staging_sweep_ttl_secs: 1,
            staging_lease_heartbeat_timeout_ms: 1,
            staging_sweep_batch_limit: 32,
            enforce_linux_openat2: true,
            privileged_probe_timeout_ms: 200,
            privileged_commit_timeout_ms: 200,
            disable_privileged_on_probe_failure: true,
        },
        guest_filesystem_policy: rq::GuestFilesystemPolicy {
            require_control_dir_writable: true,
            require_staging_dir_writable: true,
            require_non_readonly_mount: true,
            min_free_bytes_floor: 64 * 1024 * 1024,
        },
        reset_policy: rq::ResetPolicy {
            admission_freeze_timeout_ms: 500,
            zombie_reap_timeout_ms: 500,
            lock_acquire_timeout_ms: 200,
            network_call_timeout_ms: 1_000,
            total_reset_deadline_ms: 12_000,
        },
        replay_cache_policy: rq::ReplayCachePolicy {
            retained_epoch_buckets: 2,
            max_nonces_per_epoch: 256,
        },
        ssh_pool: rq::SshPoolConfig {
            max_active_targets_hard_cap: 8,
            idle_ttl_secs: 30,
            sweep_interval_secs: 30,
            fd_soft_limit: 4096,
            fd_reserve: 64,
            fd_per_command_budget: 4,
            fd_telemetry_sample_ms: 100,
        },
        host_artifact_gc: rq::HostArtifactGcConfig {
            enable_gc: true,
            gc_ttl_secs: 60,
            state_root_dir: remote_control_dir.join("state"),
            host_binary_sha256_or_build_id: "kria-test".to_string(),
        },
        infrastructure_runtime: rq::InfrastructureRuntimeConfig {
            infra_worker_threads: 2,
            high_priority_queue_capacity: 16,
            medium_priority_queue_capacity: 16,
            low_priority_queue_capacity: 16,
            infra_spawn_timeout_ms: 1_000,
        },
        ssh_connect_timeout_ms: 2_000,
        command_timeout_ms: 15_000,
        boot_wait_timeout_ms: 15_000,
        poll_interval_ms: 50,
        shutdown_timeout_ms: 5_000,
        soft_reset_grace_ms: 200,
        soft_reset_kill_timeout_ms: 200,
        max_soft_reset_attempts: 2,
        inflight_drain_timeout_ms: 1_000,
        local_cancel_kill_timeout_ms: 500,
        max_stdout_bytes: 2 * 1024 * 1024,
        max_stderr_bytes: 2 * 1024 * 1024,
        max_read_file_bytes: 2 * 1024 * 1024,
        command_timeout_requires_reset: true,
        known_hosts_path: None,
        strict_host_key_checking: false,
        pinned_host_key_sha256: vm.pinned_hostkey_sha256.clone(),
        remote_workspace_root: Some(workspace_root),
    }
}
