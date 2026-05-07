use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::time::timeout;

use crate::infra::environment::remote_qemu as rq;
use crate::infra::snapshot::{
    ensure_baseline_snapshot, try_fast_restore_latest_snapshot, SnapshotDriftTolerance,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestMode {
    Smoke,
    Infra,
    Destructive,
    AppLogic,
    Full,
}

impl TestMode {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "SMOKE" => Some(Self::Smoke),
            "INFRA" => Some(Self::Infra),
            "DESTRUCTIVE" | "OS" | "OSLEVEL" => Some(Self::Destructive),
            "APP" | "APPLOGIC" => Some(Self::AppLogic),
            "FULL" => Some(Self::Full),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestZone {
    Infrastructure,
    OsLevel,
    AppLogic,
    Smoke,
}

impl TestZone {
    fn order(self) -> u8 {
        match self {
            Self::Infrastructure => 0,
            Self::OsLevel => 1,
            Self::AppLogic => 2,
            Self::Smoke => 3,
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
        let pinned_hostkey_sha256 = env::var("KRIA_TEST_VM_HOSTKEY_SHA256")
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
        let (reachable, latency_ewma_ms) = if env::var("KRIA_TEST_VM_SKIP_PROBE").is_ok() {
            (false, None)
        } else {
            probe_latency_ewma(&host, port).await
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
}

impl RunnerConfig {
    async fn from_args(args: &[String]) -> Result<Self> {
        let mut mode = None;
        let mut report_root = None;
        let mut iter = args.iter().skip(1);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--mode" => {
                    let Some(raw) = iter.next() else {
                        return Err(anyhow!("--mode requires a value"));
                    };
                    mode = Some(
                        TestMode::parse(raw)
                            .ok_or_else(|| anyhow!("unsupported mode: {raw}"))?,
                    );
                }
                "--report-dir" => {
                    let Some(raw) = iter.next() else {
                        return Err(anyhow!("--report-dir requires a path"));
                    };
                    report_root = Some(PathBuf::from(raw));
                }
                _ => {}
            }
        }

        let mode = match mode {
            Some(mode) => mode,
            None => prompt_mode_interactive().await?,
        };

        let report_root = report_root.unwrap_or_else(|| PathBuf::from("target-tests"));
        let vm = VmEnvironment::detect().await?;

        Ok(Self {
            mode,
            report_root,
            vm,
            fail_fast_infra: true,
        })
    }
}

#[derive(Debug, Clone)]
struct TestCommand {
    name: String,
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

impl TestCommand {
    fn cargo_test(package: &str, test: &str) -> Self {
        Self {
            name: format!("{package}::{test}"),
            program: "cargo".to_string(),
            args: vec!["test".to_string(), "-p".to_string(), package.to_string(), "--test".to_string(), test.to_string()],
            env: Vec::new(),
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

    let run_tag = timestamp_tag();
    let report_path = config
        .report_root
        .join(format!("KRIA_TEST_REPORT_{run_tag}.md"));
    let run_dir = config
        .report_root
        .join(format!("kria-test-{run_tag}"));
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("create run dir {}", run_dir.display()))?;

    let mut suites = build_suites(config.mode);
    suites.sort_by_key(|suite| suite.zone.order());

    let mut suite_reports = Vec::new();
    let mut hmac = HmacReport { total: 0, success: 0 };

    for suite in suites {
        if suite.requires_vm && !config.vm.reachable {
            suite_reports.push(SuiteReport {
                name: suite.name,
                zone: suite.zone,
                status: SuiteStatus::Skipped,
                skip_reason: Some("VM unreachable; set KRIA_TEST_VM_HOST or enable probe".to_string()),
                commands: Vec::new(),
            });
            continue;
        }

        if suite.destructive && env::var("KRIA_TEST_ALLOW_DESTRUCTIVE").ok().as_deref() != Some("1") {
            suite_reports.push(SuiteReport {
                name: suite.name,
                zone: suite.zone,
                status: SuiteStatus::Skipped,
                skip_reason: Some("KRIA_TEST_ALLOW_DESTRUCTIVE=1 not set".to_string()),
                commands: Vec::new(),
            });
            continue;
        }

        let snapshot_hook = if suite.destructive {
            SnapshotHook::prepare(&config.vm, &run_dir).await?
        } else {
            SnapshotHook::Noop {
                reason: "not destructive".to_string(),
            }
        };

        let suite_report = run_suite(&suite, &run_dir).await?;

        if suite.zone == TestZone::Infrastructure {
            hmac.total += 1;
            if suite_report.status == SuiteStatus::Passed {
                hmac.success += 1;
            }
        }

        snapshot_hook.restore().await?;

        let infra_failed = suite.zone == TestZone::Infrastructure
            && suite_report.status == SuiteStatus::Failed
            && config.fail_fast_infra;

        suite_reports.push(suite_report);

        if infra_failed {
            break;
        }
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

async fn run_suite(suite: &TestSuite, run_dir: &Path) -> Result<SuiteReport> {
    let mut commands = Vec::new();
    let mut failed = 0usize;

    for command in &suite.commands {
        let report = run_command(command, run_dir).await?;
        if report.exit_code != Some(0) {
            failed += 1;
        }
        commands.push(report);
    }

    let status = if failed == 0 { SuiteStatus::Passed } else { SuiteStatus::Failed };

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
    for (key, value) in &command.env {
        cmd.env(key, value);
    }
    cmd.env("RUST_BACKTRACE", "1");

    let started = Instant::now();
    let output = cmd.output().await.with_context(|| {
        format!("execute {} {}", command.program, command.args.join(" "))
    })?;
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

fn build_suites(mode: TestMode) -> Vec<TestSuite> {
    let mut suites = Vec::new();

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

    match mode {
        TestMode::Smoke => suites.push(smoke),
        TestMode::Infra => suites.push(infra),
        TestMode::Destructive => suites.push(destructive),
        TestMode::AppLogic => suites.push(app_logic),
        TestMode::Full => {
            suites.push(infra);
            suites.push(destructive);
            suites.push(app_logic);
            suites.push(smoke);
        }
    }

    suites
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

fn render_report(report: &TestReport, run_dir: &Path) -> String {
    let mut out = String::new();
    out.push_str("# KRIA Test Report\n\n");
    out.push_str(&format!("- Mode: {:?}\n", report.mode));
    out.push_str(&format!(
        "- Started: {}\n",
        report.started_at.to_rfc3339()
    ));
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
    out.push_str(&format!("- SSH key: {}\n", report.vm.ssh_key_path.display()));
    out.push_str(&format!("- Reachable: {}\n", report.vm.reachable));
    out.push_str(&format!("- Running inside VM: {}\n", report.vm.running_inside_vm));
    if let Some(ewma) = report.vm.latency_ewma_ms {
        out.push_str(&format!("- Latency EWMA (ms): {:.2}\n", ewma));
    } else {
        out.push_str("- Latency EWMA (ms): n/a\n");
    }
    if let Some(fp) = &report.vm.pinned_hostkey_sha256 {
        out.push_str(&format!("- Pinned host key: {}\n", fp));
    }
    out.push_str("\n");

    out.push_str("## HMAC Verification\n\n");
    if report.hmac.total == 0 {
        out.push_str("- Success rate: n/a\n\n");
    } else {
        let rate = (report.hmac.success as f64 / report.hmac.total as f64) * 100.0;
        out.push_str(&format!(
            "- Success rate: {}/{} ({:.1}%)\n\n",
            report.hmac.success, report.hmac.total, rate
        ));
    }

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
        out.push_str("\n");

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
        out.push_str("\n");
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
        other => TestMode::parse(other)
            .ok_or_else(|| anyhow!("unsupported mode selection: {other}")),
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
        match probe_tcp_latency_ms(host, port).await {
            Ok(sample) => ewma.update(sample),
            Err(_) => {}
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

enum SnapshotHook {
    Noop { reason: String },
    Qemu { env: rq::QemuSshEnvironment },
}

impl SnapshotHook {
    async fn prepare(vm: &VmEnvironment, run_dir: &Path) -> Result<Self> {
        if env::var("KRIA_TEST_SNAPSHOT").ok().as_deref() != Some("1") {
            return Ok(Self::Noop {
                reason: "KRIA_TEST_SNAPSHOT not set".to_string(),
            });
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
            Self::Noop { .. } => Ok(()),
            Self::Qemu { env } => {
                let _ = try_fast_restore_latest_snapshot(&env, SnapshotDriftTolerance::default())
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
