//! Curiosity Engine — autonomous investigation with strict budget control.
//!
//! # Architecture
//!
//! ```text
//! ┌───────────────────┐
//! │  PerceptionEvent  │  ← from PerceptionBus
//! └─────────┬─────────┘
//!           │
//!           ▼
//! ┌───────────────────┐
//! │  NoveltyDetector  │  "Is this interesting enough to investigate?"
//! └─────────┬─────────┘
//!           │
//!           ▼
//! ┌───────────────────┐
//! │ EvidenceGatherer  │  Generate read-only diagnostic commands
//! └─────────┬─────────┘
//!           │
//!           ▼
//! ┌───────────────────┐
//! │   BudgetGuard     │  CPU time limit + CancellationToken yielding
//! └─────────┬─────────┘
//!           │
//!           ▼
//! ┌───────────────────┐
//! │  PolicyGate       │  Green-tier safety check
//! └─────────┬─────────┘
//!           │
//!           ▼
//! ┌───────────────────┐
//! │  Execute (RO)     │  via SubprocessExecutor
//! └───────────────────┘
//! ```
//!
//! # Safety Invariants
//!
//! 1. **Read-only only.** The CuriosityLoop can ONLY execute commands that
//!    the PolicyGate classifies as `RiskLevel::Green` with `ReadFilesystem`
//!    or `ProcessInspect` capabilities. No writes, no network, no control.
//!
//! 2. **Yielding.** The loop checks a `CancellationToken` at every diagnostic
//!    step. If the ExecutiveController sends a cancellation (P0/P1 task
//!    arrived), the loop immediately yields — no grace period, no cleanup.
//!
//! 3. **Budget-capped.** Each investigation cycle has a CPU time budget
//!    (default: 5 seconds). When exhausted, the loop pauses until the
//!    next tick. This prevents curiosity from starving foreground tasks.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;

use crate::agent::perception::{EventKind, EventSeverity, PerceptionEvent};
use crate::safety::policy_gate::PolicyGate;
use crate::safety::RiskLevel;
use crate::tools::subprocess_executor::StructuredCommand;

// ─── Budget Guard ───────────────────────────────────────────────────────────

/// Budget guard that enforces CPU time limits and responds to preemption.
///
/// The CuriosityLoop creates one `BudgetGuard` per investigation cycle.
/// At every diagnostic step, it calls `BudgetGuard::check()` which:
///
/// 1. Returns `Err(BudgetExhausted)` if the time budget is spent.
/// 2. Returns `Err(Preempted)` if the cancellation token was triggered.
/// 3. Returns `Ok(remaining)` otherwise.
///
/// This is a cooperative yielding mechanism — the loop MUST call `check()`
/// frequently (at least once per command execution) to ensure responsiveness.
pub struct BudgetGuard {
    /// Total CPU time budget for this investigation cycle.
    budget: Duration,
    /// When the budget window started.
    started_at: Instant,
    /// Cancellation token from ExecutiveController.
    /// When triggered, the curiosity loop must yield immediately.
    cancel: tokio_util::sync::CancellationToken,
    /// Tracks total time spent in diagnostic commands.
    time_spent: Duration,
}

/// Result of a budget check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetStatus {
    /// Budget remaining. Contains the time left.
    Ok(Duration),
    /// Time budget exhausted. Stop investigation.
    BudgetExhausted,
    /// ExecutiveController signaled preemption (P0/P1 task arrived).
    Preempted,
}

impl BudgetGuard {
    /// Create a new budget guard.
    ///
    /// - `budget`: Total time allowed for this investigation cycle.
    /// - `cancel`: Token from ExecutiveController for preemption.
    pub fn new(budget: Duration, cancel: tokio_util::sync::CancellationToken) -> Self {
        Self {
            budget,
            started_at: Instant::now(),
            cancel,
            time_spent: Duration::ZERO,
        }
    }

    /// Check if the budget is still available and no preemption signal was sent.
    ///
    /// Call this BEFORE every diagnostic step.
    pub fn check(&self) -> BudgetStatus {
        // Preemption takes priority over budget.
        if self.cancel.is_cancelled() {
            return BudgetStatus::Preempted;
        }

        let elapsed = self.started_at.elapsed();
        if elapsed >= self.budget {
            return BudgetStatus::BudgetExhausted;
        }

        BudgetStatus::Ok(self.budget - elapsed)
    }

    /// Record time spent on a diagnostic command.
    pub fn record_time(&mut self, duration: Duration) {
        self.time_spent += duration;
    }

    /// Get total time spent so far.
    pub fn time_spent(&self) -> Duration {
        self.time_spent
    }

    /// Get total elapsed wall-clock time.
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Get remaining budget.
    pub fn remaining(&self) -> Duration {
        self.budget.saturating_sub(self.started_at.elapsed())
    }

    /// Get the cancellation token (for passing to spawned tasks).
    pub fn cancel_token(&self) -> tokio_util::sync::CancellationToken {
        self.cancel.clone()
    }
}

// ─── Novelty Detector ───────────────────────────────────────────────────────

/// Determines whether a perception event is "interesting enough" to investigate.
///
/// Uses simple heuristics (no LLM calls):
/// - Critical/Warning severity → always investigate
/// - Health breaches → always investigate
/// - Process crashes → always investigate
/// - Normal filesystem events → ignore (too noisy)
pub struct NoveltyDetector;

impl NoveltyDetector {
    /// Returns `true` if the event warrants autonomous investigation.
    pub fn is_novel(event: &PerceptionEvent) -> bool {
        match event.severity {
            EventSeverity::Critical | EventSeverity::Warning => true,
            EventSeverity::Notable => Self::is_interesting_kind(&event.kind),
            EventSeverity::Info => false,
        }
    }

    fn is_interesting_kind(kind: &EventKind) -> bool {
        matches!(
            kind,
            EventKind::HealthBreach(_) | EventKind::ProcessLifecycle(_)
        )
    }
}

// ─── Evidence Gatherer ──────────────────────────────────────────────────────

/// Generates read-only diagnostic plans to investigate perception events.
///
/// All commands are Green-tier read-only: `systemctl status`, `journalctl`,
/// `df`, `free`, `ps`, `dmesg`, etc. The PolicyGate enforces this.
pub struct EvidenceGatherer;

/// A diagnostic plan: a sequence of read-only commands to investigate an event.
#[derive(Debug, Clone)]
pub struct DiagnosticPlan {
    /// Human-readable description of what we're investigating.
    pub goal: String,
    /// Commands to execute (in order).
    pub commands: Vec<StructuredCommand>,
    /// Expected maximum execution time.
    pub estimated_duration: Duration,
}

impl EvidenceGatherer {
    /// Generate a diagnostic plan for a perception event.
    ///
    /// Returns `None` if the event doesn't warrant investigation
    /// (e.g., normal filesystem activity).
    pub fn plan_for(event: &PerceptionEvent) -> Option<DiagnosticPlan> {
        match &event.kind {
            EventKind::HealthBreach(detail) => Some(Self::plan_health_breach(detail)),
            EventKind::ProcessLifecycle(detail) => Some(Self::plan_process_lifecycle(detail)),
            EventKind::NetworkChange(detail) => Some(Self::plan_network_change(detail)),
            EventKind::DbusSignal(detail) => Some(Self::plan_dbus_signal(detail)),
            EventKind::Filesystem(_) => {
                // Filesystem events are too noisy for autonomous investigation
                // unless they're critical severity.
                if event.severity >= EventSeverity::Critical {
                    Some(Self::plan_filesystem_critical(event))
                } else {
                    None
                }
            }
        }
    }

    fn plan_health_breach(detail: &str) -> DiagnosticPlan {
        let detail_lower = detail.to_lowercase();
        let mut commands = Vec::new();

        if detail_lower.contains("disk") {
            commands.push(cmd("df", &["-h"]));
            commands.push(cmd("du", &["-sh", "/tmp", "/var/log"]));
        } else if detail_lower.contains("ram") || detail_lower.contains("memory") {
            commands.push(cmd("free", &["-h"]));
            commands.push(cmd("ps", &["aux", "--sort=-rss"]));
        } else if detail_lower.contains("cpu") {
            commands.push(cmd("top", &["-bn1", "-o", "%CPU"]));
            commands.push(cmd("ps", &["aux", "--sort=-%cpu"]));
        } else if detail_lower.contains("battery") {
            commands.push(cmd("cat", &["/sys/class/power_supply/BAT0/capacity"]));
            commands.push(cmd("cat", &["/sys/class/power_supply/BAT0/status"]));
        } else if detail_lower.contains("thermal") {
            commands.push(cmd("cat", &["/sys/class/thermal/thermal_zone0/temp"]));
            commands.push(cmd("sensors", &[]));
        }

        // Always add a general system health snapshot.
        commands.push(cmd("uptime", &[]));

        DiagnosticPlan {
            goal: format!("Investigate health breach: {}", detail),
            commands,
            estimated_duration: Duration::from_secs(3),
        }
    }

    fn plan_process_lifecycle(detail: &str) -> DiagnosticPlan {
        let mut commands = Vec::new();

        // Extract process name if possible.
        if let Some(name) = extract_process_name(detail) {
            commands.push(cmd("systemctl", &["status", &name]));
            commands.push(cmd("journalctl", &["-u", &name, "--no-pager", "-n", "50"]));
        }

        // General process inspection.
        commands.push(cmd("ps", &["aux"]));
        commands.push(cmd("dmesg", &["--level=err", "-T"]));

        DiagnosticPlan {
            goal: format!("Investigate process event: {}", detail),
            commands,
            estimated_duration: Duration::from_secs(3),
        }
    }

    fn plan_network_change(detail: &str) -> DiagnosticPlan {
        let commands = vec![
            cmd("ip", &["addr", "show"]),
            cmd("ip", &["route", "show"]),
            cmd("ping", &["-c", "3", "-W", "2", "8.8.8.8"]),
        ];

        DiagnosticPlan {
            goal: format!("Investigate network change: {}", detail),
            commands,
            estimated_duration: Duration::from_secs(10),
        }
    }

    fn plan_dbus_signal(detail: &str) -> DiagnosticPlan {
        let commands = vec![
            cmd("systemctl", &["list-units", "--type=service", "--state=running"]),
            cmd("journalctl", &["--no-pager", "-n", "30"]),
        ];

        DiagnosticPlan {
            goal: format!("Investigate D-Bus signal: {}", detail),
            commands,
            estimated_duration: Duration::from_secs(3),
        }
    }

    fn plan_filesystem_critical(event: &PerceptionEvent) -> DiagnosticPlan {
        let path = event
            .primary_path
            .as_deref()
            .unwrap_or("/");

        let commands = vec![
            cmd("ls", &["-la", path]),
            cmd("stat", &[path]),
            cmd("dmesg", &["--level=err", "-T"]),
        ];

        DiagnosticPlan {
            goal: format!("Investigate critical filesystem event: {}", event.summary),
            commands,
            estimated_duration: Duration::from_secs(2),
        }
    }
}

/// Build a `StructuredCommand` concisely.
fn cmd(binary: &str, args: &[&str]) -> StructuredCommand {
    StructuredCommand {
        binary: binary.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        target: "local".to_string(),
        timeout_secs: 10,
        working_dir: None,
        env_vars: None,
    }
}

/// Extract a process/service name from a lifecycle event string.
fn extract_process_name(detail: &str) -> Option<String> {
    // Try to find a service name pattern.
    let lower = detail.to_lowercase();
    if lower.contains("crashed") || lower.contains("exited") || lower.contains("started") {
        // Take the first word as the process name.
        detail.split_whitespace().next().map(|s| s.to_string())
    } else {
        None
    }
}

// ─── Command Validator ──────────────────────────────────────────────────────

/// Validates that a command is safe for autonomous execution.
///
/// Uses the PolicyGate to ensure only Green, read-only commands pass.
pub struct CommandValidator {
    policy_gate: Arc<dyn PolicyGate>,
}

impl CommandValidator {
    pub fn new(policy_gate: Arc<dyn PolicyGate>) -> Self {
        Self { policy_gate }
    }

    /// Validate a command for autonomous execution.
    ///
    /// Returns `Ok(())` if the command is Green and read-only.
    /// Returns `Err(reason)` if the command is unsafe.
    pub fn validate(&self, command: &StructuredCommand) -> Result<(), String> {
        let decision = self.policy_gate.evaluate(&command.binary, &command.args);

        // Blocked commands are never allowed.
        if decision.is_blocked() {
            return Err(format!(
                "Command '{}' is blocked. Autonomous investigation only allows Green, read-only commands.",
                command.binary
            ));
        }

        // Only Green-tier commands are allowed for autonomous investigation.
        let risk = decision.risk_level();
        if risk != RiskLevel::Green {
            return Err(format!(
                "Command '{}' requires {} risk level. Autonomous investigation only allows Green.",
                command.binary, risk
            ));
        }

        // Verify it's read-only.
        let capabilities = self.policy_gate.resolve_capabilities(&command.binary, &command.args);
        for cap in &capabilities {
            if !cap.is_read_only() {
                return Err(format!(
                    "Command '{}' has non-read-only capability {:?}. Autonomous investigation is read-only.",
                    command.binary, cap
                ));
            }
        }

        Ok(())
    }
}

// ─── Curiosity Loop ─────────────────────────────────────────────────────────

/// Configuration for the curiosity loop.
#[derive(Debug, Clone)]
pub struct CuriosityConfig {
    /// CPU time budget per investigation cycle.
    pub budget_per_cycle: Duration,
    /// Minimum time between investigation cycles.
    pub cooldown: Duration,
    /// Maximum number of commands per diagnostic plan.
    pub max_commands_per_plan: usize,
    /// Command timeout for individual diagnostic commands.
    pub command_timeout: Duration,
}

impl Default for CuriosityConfig {
    fn default() -> Self {
        Self {
            budget_per_cycle: Duration::from_secs(5),
            cooldown: Duration::from_secs(10),
            max_commands_per_plan: 5,
            command_timeout: Duration::from_secs(10),
        }
    }
}

/// The main curiosity loop — investigates interesting perception events.
///
/// Subscribes to the PerceptionBus, filters for novel events, generates
/// read-only diagnostic plans, and executes them within budget constraints.
/// Yields immediately when the ExecutiveController signals preemption.
pub struct CuriosityLoop {
    config: CuriosityConfig,
    perception_rx: broadcast::Receiver<PerceptionEvent>,
    policy_gate: Arc<dyn PolicyGate>,
    cancel: tokio_util::sync::CancellationToken,
}

impl CuriosityLoop {
    pub fn new(
        config: CuriosityConfig,
        perception_rx: broadcast::Receiver<PerceptionEvent>,
        policy_gate: Arc<dyn PolicyGate>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Self {
        Self {
            config,
            perception_rx,
            policy_gate,
            cancel,
        }
    }

    /// Run the curiosity loop. Returns when `cancel` is triggered.
    pub async fn run(&mut self) {
        let validator = CommandValidator::new(Arc::clone(&self.policy_gate));

        tracing::info!("CuriosityLoop started");

        loop {
            if self.cancel.is_cancelled() {
                tracing::info!("CuriosityLoop received shutdown signal");
                break;
            }

            // Wait for the next perception event (with timeout for cooldown).
            let event = tokio::select! {
                _ = self.cancel.cancelled() => {
                    tracing::info!("CuriosityLoop shutting down");
                    break;
                }
                result = self.perception_rx.recv() => {
                    match result {
                        Ok(event) => event,
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("CuriosityLoop lagged by {} events", n);
                            continue;
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            tracing::info!("Perception bus closed, CuriosityLoop shutting down");
                            break;
                        }
                    }
                }
            };

            // Check if this event is worth investigating.
            if !NoveltyDetector::is_novel(&event) {
                continue;
            }

            tracing::info!(
                "CuriosityLoop: novel event detected: {} (severity: {:?})",
                event.summary,
                event.severity
            );

            // Generate diagnostic plan.
            let plan = match EvidenceGatherer::plan_for(&event) {
                Some(plan) => plan,
                None => continue,
            };

            // Execute the plan within budget.
            self.execute_plan(&plan, &validator).await;

            // Cooldown before next investigation.
            tokio::select! {
                _ = self.cancel.cancelled() => break,
                _ = tokio::time::sleep(self.config.cooldown) => {}
            }
        }

        tracing::info!("CuriosityLoop shut down");
    }

    /// Execute a diagnostic plan within the budget guard.
    async fn execute_plan(&self, plan: &DiagnosticPlan, validator: &CommandValidator) {
        let mut guard = BudgetGuard::new(self.config.budget_per_cycle, self.cancel.clone());

        tracing::info!("Executing diagnostic plan: {}", plan.goal);

        for (i, command) in plan.commands.iter().enumerate() {
            // Check budget before each command.
            match guard.check() {
                BudgetStatus::Ok(remaining) => {
                    tracing::debug!(
                        "Budget check: {} remaining (command {}/{})",
                        humantime::format_duration(remaining),
                        i + 1,
                        plan.commands.len()
                    );
                }
                BudgetStatus::BudgetExhausted => {
                    tracing::info!(
                        "Budget exhausted after {} commands. Stopping investigation.",
                        i
                    );
                    break;
                }
                BudgetStatus::Preempted => {
                    tracing::info!(
                        "Preempted by higher-priority task after {} commands. Yielding immediately.",
                        i
                    );
                    return;
                }
            }

            // Validate command safety.
            if let Err(reason) = validator.validate(command) {
                tracing::warn!("Skipping unsafe command '{}': {}", command.binary, reason);
                continue;
            }

            // Execute the command (read-only, short timeout).
            let start = Instant::now();
            let result = self.execute_command(command).await;
            let elapsed = start.elapsed();
            guard.record_time(elapsed);

            match result {
                Ok(output) => {
                    tracing::debug!(
                        "Diagnostic '{}' completed in {:?}: {}",
                        command.binary,
                        elapsed,
                        truncate(&output.stdout, 200)
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        "Diagnostic '{}' failed in {:?}: {}",
                        command.binary,
                        elapsed,
                        e
                    );
                }
            }
        }

        tracing::info!(
            "Diagnostic plan '{}' complete. Total time: {:?}",
            plan.goal,
            guard.time_spent()
        );
    }

    /// Execute a single read-only command.
    ///
    /// This is a simplified execution path for autonomous diagnostics.
    /// In production, this would delegate to the SubprocessExecutor.
    async fn execute_command(&self, command: &StructuredCommand) -> Result<CommandOutput, String> {
        let cancel = self.cancel.clone();
        let timeout = Duration::from_secs(command.timeout_secs);
        let binary = command.binary.clone();
        let args = command.args.clone();

        tokio::select! {
            _ = cancel.cancelled() => {
                Err("Preempted".to_string())
            }
            result = tokio::time::timeout(timeout, run_command(&binary, &args)) => {
                match result {
                    Ok(r) => r,
                    Err(_) => Err(format!("Command '{}' timed out after {:?}", binary, timeout)),
                }
            }
        }
    }
}

/// Output from a diagnostic command.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct CommandOutput {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

/// Run a command as a subprocess (read-only diagnostics).
async fn run_command(binary: &str, args: &[String]) -> Result<CommandOutput, String> {
    let output = tokio::process::Command::new(binary)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("Failed to execute '{}': {}", binary, e))?;

    Ok(CommandOutput {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}

/// Truncate a string to `max_len` characters.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

// ─── Humantime formatting (inline, avoids dependency) ───────────────────────

mod humantime {
    use std::time::Duration;

    pub struct FormattedDuration(Duration);

    impl std::fmt::Display for FormattedDuration {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let ms = self.0.as_millis();
            if ms < 1000 {
                write!(f, "{}ms", ms)
            } else if ms < 60_000 {
                write!(f, "{:.1}s", ms as f64 / 1000.0)
            } else {
                write!(f, "{}m{}s", ms / 60_000, (ms % 60_000) / 1000)
            }
        }
    }

    pub fn format_duration(d: Duration) -> FormattedDuration {
        FormattedDuration(d)
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::perception::{EventKind, EventSeverity, FilesystemOp, PerceptionEvent};

    // ── BudgetGuard Tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_budget_guard_ok_within_budget() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let guard = BudgetGuard::new(Duration::from_secs(5), cancel);

        match guard.check() {
            BudgetStatus::Ok(remaining) => {
                assert!(remaining > Duration::from_secs(4));
                assert!(remaining <= Duration::from_secs(5));
            }
            other => panic!("Expected Ok, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_budget_guard_exhausted() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let guard = BudgetGuard::new(Duration::from_millis(50), cancel);

        // Wait for budget to expire.
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(guard.check(), BudgetStatus::BudgetExhausted);
    }

    #[tokio::test]
    async fn test_budget_guard_preempted() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let guard = BudgetGuard::new(Duration::from_secs(60), cancel.clone());

        // Verify budget is OK initially.
        assert!(matches!(guard.check(), BudgetStatus::Ok(_)));

        // Simulate preemption from ExecutiveController.
        cancel.cancel();

        assert_eq!(guard.check(), BudgetStatus::Preempted);
    }

    #[tokio::test]
    async fn test_budget_guard_preemption_takes_priority_over_exhaustion() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let guard = BudgetGuard::new(Duration::from_millis(1), cancel.clone());

        // Wait for budget to expire.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Cancel after budget expired.
        cancel.cancel();

        // Preemption should still take priority.
        assert_eq!(guard.check(), BudgetStatus::Preempted);
    }

    #[tokio::test]
    async fn test_budget_guard_time_recording() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut guard = BudgetGuard::new(Duration::from_secs(5), cancel);

        assert_eq!(guard.time_spent(), Duration::ZERO);

        guard.record_time(Duration::from_millis(100));
        guard.record_time(Duration::from_millis(200));

        assert_eq!(guard.time_spent(), Duration::from_millis(300));
    }

    #[tokio::test]
    async fn test_budget_guard_remaining_decreases() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let guard = BudgetGuard::new(Duration::from_secs(5), cancel);

        let remaining1 = guard.remaining();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let remaining2 = guard.remaining();

        assert!(remaining1 > remaining2);
        assert!(remaining2 > Duration::from_secs(4));
    }

    // ── NoveltyDetector Tests ────────────────────────────────────────────────

    #[test]
    fn test_novelty_detector_critical_always_novel() {
        let event = make_event(EventSeverity::Critical, EventKind::Filesystem(FilesystemOp::Deleted));
        assert!(NoveltyDetector::is_novel(&event));
    }

    #[test]
    fn test_novelty_detector_warning_always_novel() {
        let event = make_event(EventSeverity::Warning, EventKind::HealthBreach("disk_low".to_string()));
        assert!(NoveltyDetector::is_novel(&event));
    }

    #[test]
    fn test_novelty_detector_info_never_novel() {
        let event = make_event(EventSeverity::Info, EventKind::Filesystem(FilesystemOp::Modified));
        assert!(!NoveltyDetector::is_novel(&event));
    }

    #[test]
    fn test_novelty_detector_notable_health_is_novel() {
        let event = make_event(
            EventSeverity::Notable,
            EventKind::HealthBreach("cpu_high".to_string()),
        );
        assert!(NoveltyDetector::is_novel(&event));
    }

    #[test]
    fn test_novelty_detector_notable_process_is_novel() {
        let event = make_event(
            EventSeverity::Notable,
            EventKind::ProcessLifecycle("nginx_crashed".to_string()),
        );
        assert!(NoveltyDetector::is_novel(&event));
    }

    #[test]
    fn test_novelty_detector_notable_filesystem_not_novel() {
        let event = make_event(
            EventSeverity::Notable,
            EventKind::Filesystem(FilesystemOp::Modified),
        );
        assert!(!NoveltyDetector::is_novel(&event));
    }

    #[test]
    fn test_novelty_detector_notable_network_not_novel() {
        let event = make_event(
            EventSeverity::Notable,
            EventKind::NetworkChange("eth0_up".to_string()),
        );
        assert!(!NoveltyDetector::is_novel(&event));
    }

    // ── EvidenceGatherer Tests ───────────────────────────────────────────────

    #[test]
    fn test_evidence_gatherer_health_breach_disk() {
        let event = make_event(
            EventSeverity::Warning,
            EventKind::HealthBreach("disk_low".to_string()),
        );
        let plan = EvidenceGatherer::plan_for(&event).expect("Expected a diagnostic plan");
        assert!(plan.goal.contains("disk"));
        assert!(plan.commands.iter().any(|c| c.binary == "df"));
    }

    #[test]
    fn test_evidence_gatherer_health_breach_memory() {
        let event = make_event(
            EventSeverity::Warning,
            EventKind::HealthBreach("memory_high".to_string()),
        );
        let plan = EvidenceGatherer::plan_for(&event).expect("Expected a diagnostic plan");
        assert!(plan.commands.iter().any(|c| c.binary == "free"));
    }

    #[test]
    fn test_evidence_gatherer_process_lifecycle() {
        let event = make_event(
            EventSeverity::Notable,
            EventKind::ProcessLifecycle("nginx_crashed".to_string()),
        );
        let plan = EvidenceGatherer::plan_for(&event).expect("Expected a diagnostic plan");
        assert!(plan.commands.iter().any(|c| c.binary == "ps"));
    }

    #[test]
    fn test_evidence_gatherer_network_change() {
        let event = make_event(
            EventSeverity::Warning,
            EventKind::NetworkChange("eth0_down".to_string()),
        );
        let plan = EvidenceGatherer::plan_for(&event).expect("Expected a diagnostic plan");
        assert!(plan.commands.iter().any(|c| c.binary == "ip"));
    }

    #[test]
    fn test_evidence_gatherer_normal_filesystem_returns_none() {
        let event = make_event(
            EventSeverity::Notable,
            EventKind::Filesystem(FilesystemOp::Modified),
        );
        assert!(EvidenceGatherer::plan_for(&event).is_none());
    }

    #[test]
    fn test_evidence_gatherer_critical_filesystem_returns_plan() {
        let event = make_event(
            EventSeverity::Critical,
            EventKind::Filesystem(FilesystemOp::Deleted),
        );
        let plan = EvidenceGatherer::plan_for(&event).expect("Expected plan for critical FS event");
        assert!(plan.commands.iter().any(|c| c.binary == "dmesg"));
    }

    // ── Preemption Yield Test ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_curiosity_yields_on_preemption() {
        // Simulate the preemption scenario:
        // 1. CuriosityLoop starts investigating.
        // 2. ExecutiveController cancels the token (P0 task arrived).
        // 3. CuriosityLoop must yield immediately.

        let cancel = tokio_util::sync::CancellationToken::new();
        let guard = BudgetGuard::new(Duration::from_secs(60), cancel.clone());

        // Start investigation.
        assert!(matches!(guard.check(), BudgetStatus::Ok(_)));

        // Simulate P0 task arrival → ExecutiveController cancels.
        cancel.cancel();

        // The very next check must return Preempted.
        assert_eq!(guard.check(), BudgetStatus::Preempted);

        // Verify the loop can detect this and break.
        // (In the real CuriosityLoop::execute_plan, this causes an immediate return.)
    }

    #[tokio::test]
    async fn test_curiosity_yields_mid_plan() {
        // Simulate yielding in the middle of a diagnostic plan.
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut guard = BudgetGuard::new(Duration::from_secs(60), cancel.clone());

        let commands = vec![
            cmd("uptime", &[]),
            cmd("free", &["-h"]),
            cmd("df", &["-h"]),
        ];

        let mut executed = 0;
        for (i, _cmd) in commands.iter().enumerate() {
            match guard.check() {
                BudgetStatus::Ok(_) => {
                    executed += 1;
                    guard.record_time(Duration::from_millis(50));

                    // Cancel after the second command.
                    if i == 1 {
                        cancel.cancel();
                    }
                }
                BudgetStatus::Preempted => {
                    break;
                }
                BudgetStatus::BudgetExhausted => {
                    break;
                }
            }
        }

        // Should have executed exactly 2 commands before yielding.
        assert_eq!(executed, 2);
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn make_event(severity: EventSeverity, kind: EventKind) -> PerceptionEvent {
        PerceptionEvent {
            kind,
            key: "test".to_string(),
            primary_path: None,
            count: 1,
            summary: "test event".to_string(),
            severity,
            first_seen_epoch_ms: 0,
            finalized_epoch_ms: 0,
        }
    }
}
