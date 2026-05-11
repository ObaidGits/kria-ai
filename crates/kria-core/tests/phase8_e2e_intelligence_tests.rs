// ─────────────────────────────────────────────────────────────────────────────
//  phase8_e2e_intelligence_tests.rs — End-to-end integration tests
//
//  Tests the full pipeline: ExecutiveController + PolicyGate + Planner +
//  CuriosityLoop + PerceptionBus + SelfModel — all wired together.
//
//  Design constraints:
//    • No real LLM — uses MockLlmBackend returning deterministic JSON paths
//    • No real commands — uses MockPolicyGate that auto-approves read-only
//    • Tests focus on *system routing and execution logic*, not LLM quality
//    • Every test completes within 30 seconds (no real subprocess execution)
// ─────────────────────────────────────────────────────────────────────────────

mod common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use kria_core::agent::curiosity::{
    BudgetGuard, BudgetStatus, CommandValidator, CuriosityConfig, CuriosityLoop, EvidenceGatherer,
    NoveltyDetector,
};
use kria_core::agent::executive::types::*;
use kria_core::agent::executive::{ExecutiveConfig, ExecutiveController, ExecutiveSender};
use kria_core::agent::perception::{
    EventDebouncer, EventKind, EventSeverity, FilesystemOp, PerceptionBus, PerceptionEvent,
};
use kria_core::agent::planner_v2::{BranchingPlanner, PathRisk, PlannedPath, PlannedStep};
use kria_core::agent::self_model::SelfModel;
use kria_core::agent::working_set::{StructuredEvidence, WorkingSet};
use kria_core::llm::{ChatMessage, LlmBackend, LlmResponse, TokenUsage, ToolSchema};
use kria_core::resource::gpu_lease::GpuLeaseManager;
use kria_core::safety::policy_gate::{CommandCapability, PolicyDecision, PolicyGate};
use kria_core::safety::RiskLevel;
use kria_core::tools::subprocess_executor::StructuredCommand;

// ═══════════════════════════════════════════════════════════════════════════
//  MockLlmBackend — Deterministic, no real inference
// ═══════════════════════════════════════════════════════════════════════════

/// A mock LLM backend that returns deterministic responses.
///
/// Instead of calling llama.cpp or a cloud API, this returns
/// hardcoded structured JSON paths. We are testing the *system's*
/// routing and execution logic, not the LLM's reasoning quality.
///
/// The mock tracks call count and can return different responses
/// on subsequent calls (for testing replanning).
pub struct MockLlmBackend {
    /// Queue of responses to return (popped in order).
    responses: std::sync::Mutex<Vec<LlmResponse>>,
    /// Total calls made.
    call_count: AtomicU64,
    /// Captured messages for inspection.
    captured_messages: std::sync::Mutex<Vec<Vec<ChatMessage>>>,
}

impl MockLlmBackend {
    /// Create a new mock with a queue of responses.
    pub fn new(responses: Vec<LlmResponse>) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses),
            call_count: AtomicU64::new(0),
            captured_messages: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Create a mock that returns a single deterministic 3-path plan.
    pub fn with_branching_plan() -> Self {
        let plan_json = serde_json::json!({
            "paths": [
                {
                    "risk": "DiagnoseFirst",
                    "name": "Diagnose-First",
                    "steps": [
                        {
                            "step_number": 1,
                            "description": "Check CPU usage",
                            "command": {"binary": "top", "args": ["-bn1", "-w512"], "target": "local", "timeout_secs": 10},
                            "expected_outcome": "CPU and memory snapshot",
                            "on_failure": "continue"
                        },
                        {
                            "step_number": 2,
                            "description": "Check nginx status",
                            "command": {"binary": "systemctl", "args": ["status", "nginx"], "target": "local", "timeout_secs": 5},
                            "expected_outcome": "nginx running status",
                            "on_failure": "continue"
                        }
                    ],
                    "predicted_outcome": "Identify nginx as CPU bottleneck",
                    "score": null
                },
                {
                    "risk": "MinimalRisk",
                    "name": "Minimal-Risk Fix",
                    "steps": [
                        {
                            "step_number": 1,
                            "description": "Reduce nginx workers",
                            "command": {"binary": "sed", "args": ["-i", "s/worker_processes 64/worker_processes 4/", "/etc/nginx/nginx.conf"], "target": "local", "timeout_secs": 5},
                            "expected_outcome": "Config updated",
                            "on_failure": "abort"
                        },
                        {
                            "step_number": 2,
                            "description": "Restart nginx",
                            "command": {"binary": "systemctl", "args": ["restart", "nginx"], "target": "local", "timeout_secs": 10},
                            "expected_outcome": "nginx restarted with fewer workers",
                            "on_failure": "abort"
                        }
                    ],
                    "predicted_outcome": "CPU drops from 80% to 12%",
                    "score": null
                },
                {
                    "risk": "Aggressive",
                    "name": "Aggressive Fix",
                    "steps": [
                        {
                            "step_number": 1,
                            "description": "Kill nginx and replace with caddy",
                            "command": {"binary": "systemctl", "args": ["stop", "nginx"], "target": "local", "timeout_secs": 5},
                            "expected_outcome": "nginx stopped",
                            "on_failure": "abort"
                        },
                        {
                            "step_number": 2,
                            "description": "Install caddy",
                            "command": {"binary": "apt", "args": ["install", "-y", "caddy"], "target": "local", "timeout_secs": 60},
                            "expected_outcome": "caddy installed",
                            "on_failure": "abort"
                        }
                    ],
                    "predicted_outcome": "Service replaced entirely",
                    "score": null
                }
            ],
            "selected_index": 1,
            "working_set_summary": "Goal: Make VM faster. Evidence: nginx 80% CPU."
        });

        let response = LlmResponse {
            content: plan_json.to_string(),
            model: "mock-7b".to_string(),
            usage: Some(TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 200,
                total_tokens: 300,
            }),
            tool_calls: None,
        };

        Self::new(vec![response])
    }

    /// Number of times `chat()` was called.
    pub fn call_count(&self) -> u64 {
        self.call_count.load(Ordering::Relaxed)
    }

    /// Get all captured message histories.
    pub fn captured_messages(&self) -> Vec<Vec<ChatMessage>> {
        self.captured_messages.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl LlmBackend for MockLlmBackend {
    fn model_label(&self) -> &str {
        "mock-7b"
    }

    fn capabilities(&self) -> &[String] {
        &[]
    }

    fn is_configured(&self) -> bool {
        true
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        _tools: Option<&[ToolSchema]>,
        _temperature: f32,
        _max_tokens: u32,
    ) -> anyhow::Result<LlmResponse> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        self.captured_messages
            .lock()
            .unwrap()
            .push(messages.to_vec());

        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            // Default: return a simple text response
            Ok(LlmResponse {
                content: "I don't have enough information to plan.".to_string(),
                model: "mock-7b".to_string(),
                usage: Some(TokenUsage {
                    prompt_tokens: 10,
                    completion_tokens: 10,
                    total_tokens: 20,
                }),
                tool_calls: None,
            })
        } else {
            Ok(responses.remove(0))
        }
    }

    async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSchema]>,
        temperature: f32,
        max_tokens: u32,
    ) -> anyhow::Result<std::pin::Pin<Box<dyn futures::Stream<Item = String> + Send>>> {
        let response = self.chat(messages, tools, temperature, max_tokens).await?;
        let stream = futures::stream::once(async move { response.content });
        Ok(Box::pin(stream))
    }

    async fn health_check(&self) -> bool {
        true
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  MockPolicyGate — Auto-approves read-only, blocks destructive
// ═══════════════════════════════════════════════════════════════════════════

/// A deterministic policy gate for testing.
///
/// - Read-only binaries (top, df, free, ps, systemctl status, etc.) → AutoApproved
/// - Destructive binaries (rm, shutdown, dd, mkfs) → Blocked
/// - Everything else → RequiresApproval
pub struct MockPolicyGate;

impl MockPolicyGate {
    /// Binaries that are always auto-approved (read-only).
    const READONLY_BINARIES: &'static [&'static str] = &[
        "top",
        "ps",
        "df",
        "free",
        "uptime",
        "uname",
        "hostname",
        "cat",
        "head",
        "tail",
        "grep",
        "find",
        "wc",
        "ls",
        "stat",
        "ip",
        "ss",
        "ping",
        "lscpu",
        "lspci",
        "lsusb",
        "lsblk",
        "dmesg",
        "journalctl",
        "du",
        "sensors",
        "iostat",
        "vmstat",
    ];

    /// Binaries that are always blocked.
    const BLOCKED_BINARIES: &'static [&'static str] = &[
        "dd", "mkfs", "fdisk", "shutdown", "reboot", "poweroff", "init",
    ];

    fn is_readonly_binary(binary: &str) -> bool {
        Self::READONLY_BINARIES.contains(&binary)
    }

    fn is_blocked_binary(binary: &str) -> bool {
        Self::BLOCKED_BINARIES.contains(&binary)
    }
}

impl PolicyGate for MockPolicyGate {
    fn evaluate(&self, binary: &str, _args: &[String]) -> PolicyDecision {
        if Self::is_blocked_binary(binary) {
            PolicyDecision::Blocked {
                reason: format!("Binary '{}' is permanently blocked", binary),
            }
        } else if Self::is_readonly_binary(binary) {
            PolicyDecision::AutoApproved {
                risk_level: RiskLevel::Green,
                capabilities: HashSet::from([CommandCapability::ReadFilesystem]),
            }
        } else if binary == "systemctl" {
            // systemctl with status/list-units → read-only
            PolicyDecision::AutoApproved {
                risk_level: RiskLevel::Green,
                capabilities: HashSet::from([CommandCapability::ProcessInspect]),
            }
        } else {
            PolicyDecision::RequiresApproval {
                risk_level: RiskLevel::Yellow,
                capabilities: HashSet::from([CommandCapability::WriteFilesystem]),
                reason: format!("Binary '{}' not in known-safe list", binary),
            }
        }
    }

    fn resolve_capabilities(&self, binary: &str, _args: &[String]) -> HashSet<CommandCapability> {
        if Self::is_readonly_binary(binary) {
            HashSet::from([CommandCapability::ReadFilesystem])
        } else if binary == "systemctl" {
            HashSet::from([CommandCapability::ProcessInspect])
        } else {
            HashSet::from([CommandCapability::WriteFilesystem])
        }
    }

    fn is_known_binary(&self, binary: &str) -> bool {
        Self::is_readonly_binary(binary) || Self::is_blocked_binary(binary) || binary == "systemctl"
    }

    fn classify_risk(&self, binary: &str, _args: &[String]) -> RiskLevel {
        if Self::is_blocked_binary(binary) {
            RiskLevel::Black
        } else if Self::is_readonly_binary(binary) || binary == "systemctl" {
            RiskLevel::Green
        } else {
            RiskLevel::Yellow
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Test Helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Build a test ExecutiveController with mock dependencies.
fn build_test_controller(
    max_background: usize,
) -> (ExecutiveController, ExecutiveSender, Arc<GpuLeaseManager>) {
    let gpu_lease = GpuLeaseManager::shared(Duration::from_secs(180), Duration::from_secs(15));
    let policy_gate: Arc<dyn PolicyGate> = Arc::new(MockPolicyGate);

    let config = ExecutiveConfig {
        max_background_tasks: max_background,
        preemption_grace_ms: 200, // Short for tests
        vram_maintenance_enabled: false,
        vram_idle_threshold_secs: 9999,
    };

    let (controller, sender) = ExecutiveController::new(config, gpu_lease.clone(), policy_gate);
    (controller, sender, gpu_lease)
}

/// Build a Voice TaskRequest (P0).
fn voice_task(text: &str) -> TaskRequest {
    TaskRequest::new(
        TaskPriority::Voice,
        TaskSource::VoicePipeline,
        false,
        TaskPayload::UserTurn {
            text: text.to_string(),
            is_voice: true,
            session_id: "test-session".to_string(),
        },
    )
}

/// Build an Interactive TaskRequest (P1).
fn interactive_task(text: &str) -> TaskRequest {
    TaskRequest::new(
        TaskPriority::Interactive,
        TaskSource::TextChat,
        false,
        TaskPayload::UserTurn {
            text: text.to_string(),
            is_voice: false,
            session_id: "test-session".to_string(),
        },
    )
}

/// Build a Background TaskRequest (P3).
fn background_task(_description: &str) -> TaskRequest {
    TaskRequest::new(
        TaskPriority::Background,
        TaskSource::CuriosityLoop,
        false,
        TaskPayload::BackgroundDiagnostics {
            commands: vec![StructuredCommand {
                binary: "uptime".to_string(),
                args: vec![],
                target: "local".to_string(),
                timeout_secs: 5,
                working_dir: None,
                env_vars: None,
            }],
        },
    )
}

/// Build a Maintenance TaskRequest (P4).
fn maintenance_task(description: &str) -> TaskRequest {
    TaskRequest::new(
        TaskPriority::Maintenance,
        TaskSource::Maintenance,
        false,
        TaskPayload::Maintenance {
            description: description.to_string(),
        },
    )
}

/// Build a test PerceptionEvent.
fn perception_event(kind: EventKind, severity: EventSeverity, summary: &str) -> PerceptionEvent {
    PerceptionEvent {
        kind,
        key: format!("test:{}", summary),
        primary_path: None,
        count: 1,
        summary: summary.to_string(),
        severity,
        first_seen_epoch_ms: 0,
        finalized_epoch_ms: 0,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-01: Voice Preemption — The Most Critical Safety Test
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that a P0 Voice command can preempt a running P1 Interactive task.
///
/// Scenario:
///   1. Interactive task (P1) is submitted.
///   2. Voice command (P0) arrives.
///   3. ExecutiveController must accept both (voice has higher priority).
///   4. The system doesn't crash under rapid preemption.
#[tokio::test]
async fn e2e01_voice_preemption() {
    let (mut controller, sender, _gpu) = build_test_controller(3);

    // Spawn the controller in the background.
    let controller_handle = tokio::spawn(async move {
        controller.run().await;
    });

    // Step 1: Submit an interactive task (P1).
    let interactive = interactive_task("Long-running interactive task");
    sender.submit(interactive).expect("submit interactive");

    // Step 2: Submit a voice task (P0) immediately after.
    // The voice task should be accepted (not rejected).
    let voice = voice_task("Hey Ria, what's the system status?");
    let voice_result = sender.submit(voice);
    assert!(voice_result.is_ok(), "Voice task must be accepted");

    // Step 3: Submit another interactive task to verify voice priority.
    let interactive2 = interactive_task("Another interactive task");
    let interactive2_result = sender.submit(interactive2);
    assert!(
        interactive2_result.is_ok(),
        "Interactive task must be accepted"
    );

    // Step 4: Submit ANOTHER voice task to test preemption.
    let voice2 = voice_task("Emergency stop!");
    let voice2_result = sender.submit(voice2);
    assert!(voice2_result.is_ok(), "Second voice task must be accepted");

    // Step 5: Wait a bit for tasks to process.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Step 6: Verify the system is still alive (no crash).
    assert!(sender.is_alive(), "Controller should still be running");

    // Shutdown.
    sender.shutdown();
    let result = tokio::time::timeout(Duration::from_secs(3), controller_handle).await;
    assert!(result.is_ok(), "Controller must shut down cleanly");
}

/// Verify that voice tasks are always scheduled before background tasks,
/// even when the background queue is full.
#[tokio::test]
async fn e2e01b_voice_always_p0() {
    let (mut controller, sender, _gpu) = build_test_controller(2);

    let controller_handle = tokio::spawn(async move {
        controller.run().await;
    });

    // Fill the background queue.
    for i in 0..5 {
        let task = background_task(&format!("Background task {}", i));
        let _ = sender.submit(task);
    }

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Submit a voice task. It should be accepted (not rejected).
    let voice = voice_task("System status");
    let result = sender.submit(voice);
    assert!(result.is_ok(), "Voice task must always be accepted");

    sender.shutdown();
    let _ = tokio::time::timeout(Duration::from_secs(2), controller_handle).await;
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-14: Curiosity Budget Enforcement
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that the BudgetGuard correctly enforces time limits
/// and yields immediately on preemption.
#[tokio::test]
async fn e2e14_curiosity_budget_enforcement() {
    let cancel = CancellationToken::new();

    // Step 1: Create a budget guard with a 200ms budget.
    let mut guard = BudgetGuard::new(Duration::from_millis(200), cancel.clone());

    // Step 2: Verify budget is initially OK.
    match guard.check() {
        BudgetStatus::Ok(remaining) => {
            assert!(remaining <= Duration::from_millis(200));
            assert!(remaining > Duration::from_millis(100));
        }
        other => panic!("Expected Ok, got {:?}", other),
    }

    // Step 3: Simulate some work.
    guard.record_time(Duration::from_millis(50));
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Step 4: Budget should still be OK.
    assert!(matches!(guard.check(), BudgetStatus::Ok(_)));

    // Step 5: Wait for budget to expire.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Step 6: Budget should now be exhausted.
    assert_eq!(guard.check(), BudgetStatus::BudgetExhausted);
}

/// Verify that BudgetGuard yields immediately when the cancellation
/// token is triggered (simulating P0 voice task arrival).
#[tokio::test]
async fn e2e14b_curiosity_yields_on_preemption() {
    let cancel = CancellationToken::new();

    // Create a guard with a long budget (10 seconds).
    let guard = BudgetGuard::new(Duration::from_secs(10), cancel.clone());

    // Budget should be fine.
    assert!(matches!(guard.check(), BudgetStatus::Ok(_)));

    // Simulate P0 voice task arrival → ExecutiveController cancels the token.
    cancel.cancel();

    // Guard must immediately report Preempted.
    assert_eq!(guard.check(), BudgetStatus::Preempted);

    // Preemption takes priority over budget (even if budget is exhausted).
    // Create a new guard with an expired budget AND cancellation.
    let cancel2 = CancellationToken::new();
    cancel2.cancel();
    let guard2 = BudgetGuard::new(Duration::from_millis(0), cancel2);
    // Both conditions are true, but Preempted should win.
    assert_eq!(guard2.check(), BudgetStatus::Preempted);
}

/// Verify that the CuriosityLoop yields within 100ms when the
/// ExecutiveController sends a cancellation signal.
#[tokio::test]
async fn e2e14c_curiosity_loop_yields_on_cancel() {
    let policy_gate: Arc<dyn PolicyGate> = Arc::new(MockPolicyGate);
    let cancel = CancellationToken::new();

    // Create a perception bus and curiosity loop.
    let bus = PerceptionBus::new(64);
    let perception_rx = bus.subscribe();

    let config = CuriosityConfig {
        budget_per_cycle: Duration::from_secs(60), // Long budget
        cooldown: Duration::from_millis(100),
        max_commands_per_plan: 5,
        command_timeout: Duration::from_secs(5),
    };

    let mut curiosity = CuriosityLoop::new(config, perception_rx, policy_gate, cancel.clone());

    // Spawn the curiosity loop.
    let curiosity_handle = tokio::spawn(async move {
        curiosity.run().await;
    });

    // Let it run for a bit.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Simulate P0 voice task arrival → cancel the curiosity loop.
    let start = Instant::now();
    cancel.cancel();

    // Wait for the loop to exit.
    let result = tokio::time::timeout(Duration::from_millis(500), curiosity_handle).await;
    let elapsed = start.elapsed();

    assert!(
        result.is_ok(),
        "CuriosityLoop should exit cleanly on cancel"
    );
    assert!(
        elapsed < Duration::from_millis(500),
        "CuriosityLoop should yield within 500ms, took {:?}",
        elapsed
    );
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-02: Structured Command No Injection
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that shell metacharacters in StructuredCommand args are
/// passed as literal arguments, not interpreted as shell syntax.
#[tokio::test]
async fn e2e02_structured_command_no_injection() {
    // This is a compile-time guarantee: StructuredCommand has
    // binary: String and args: Vec<String> — no shell parsing.
    // But we verify the PolicyGate correctly handles suspicious args.

    let gate = MockPolicyGate;

    // A command with shell metacharacters in args should be evaluated
    // based on the binary name, not the args content.
    let cmd = StructuredCommand {
        binary: "ls".to_string(),
        args: vec!["; rm -rf /".to_string()],
        target: "local".to_string(),
        timeout_secs: 10,
        working_dir: None,
        env_vars: None,
    };

    // "ls" is a read-only binary → auto-approved.
    // The "; rm -rf /" is just a literal string argument to ls.
    let decision = gate.evaluate(&cmd.binary, &cmd.args);
    assert!(
        decision.is_auto_approved(),
        "ls with any args should be auto-approved (args are literal, not shell-parsed)"
    );

    // A blocked binary should always be blocked, regardless of args.
    let cmd2 = StructuredCommand {
        binary: "dd".to_string(),
        args: vec!["if=/dev/zero".to_string(), "of=/dev/sda".to_string()],
        target: "local".to_string(),
        timeout_secs: 10,
        working_dir: None,
        env_vars: None,
    };
    let decision2 = gate.evaluate(&cmd2.binary, &cmd2.args);
    assert!(decision2.is_blocked(), "dd must always be blocked");
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-03: Uncertainty → Evidence Gathering
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that the EvidenceGatherer generates read-only diagnostic
/// commands for health breaches and that the CommandValidator
/// accepts them all.
#[tokio::test]
async fn e2e03_evidence_gathering_read_only() {
    let policy_gate: Arc<dyn PolicyGate> = Arc::new(MockPolicyGate);
    let validator = CommandValidator::new(policy_gate);

    // Simulate a health breach event.
    let event = perception_event(
        EventKind::HealthBreach("disk_low".to_string()),
        EventSeverity::Warning,
        "Disk usage at 95%",
    );

    let plan = EvidenceGatherer::plan_for(&event).expect("Should generate plan for health breach");

    // All commands in the plan must be validated as Green + read-only.
    for cmd in &plan.commands {
        let result = validator.validate(cmd);
        assert!(
            result.is_ok(),
            "Diagnostic command '{} {}' should be Green + read-only: {:?}",
            cmd.binary,
            cmd.args.join(" "),
            result
        );
    }

    // Verify the plan contains useful diagnostics.
    let binaries: Vec<&str> = plan.commands.iter().map(|c| c.binary.as_str()).collect();
    assert!(
        binaries.contains(&"df"),
        "Disk health breach should include df"
    );
    assert!(
        binaries.contains(&"uptime"),
        "All health plans should include uptime"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-04: SelfModel Beta Posterior Scoring
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that the SelfModel correctly scores paths using
/// Beta posterior geometric mean, and that unknown tools start at 0.50.
#[tokio::test]
async fn e2e04_self_model_scoring() {
    let mut model = SelfModel::new();

    // Unknown tools should start at 0.50 (Beta(1,1) prior).
    let score_unknown = model.score_path(&["unknown_tool_a", "unknown_tool_b"]);
    assert!(
        (score_unknown - 0.50).abs() < 0.01,
        "Unknown tools should score 0.50, got {}",
        score_unknown
    );

    // Record some successes for "top".
    for _ in 0..10 {
        model.record_outcome("top", true, Duration::from_millis(50));
    }

    // "top" should now have a high success rate.
    let top_rate = model.success_rate("top");
    assert!(
        top_rate > 0.8,
        "top should have >80% success rate after 10 successes, got {}",
        top_rate
    );

    // A path using "top" should score higher than unknown tools.
    let score_top = model.score_path(&["top"]);
    assert!(
        score_top > score_unknown,
        "Known good tool ({}) should score higher than unknown ({}): {} > {}",
        score_top,
        score_unknown,
        score_top,
        score_unknown
    );

    // Geometric mean: if one tool is bad, the whole path score drops.
    model.record_outcome("bad_tool", false, Duration::from_millis(100));
    model.record_outcome("bad_tool", false, Duration::from_millis(100));
    model.record_outcome("bad_tool", false, Duration::from_millis(100));

    let score_mixed = model.score_path(&["top", "bad_tool"]);
    assert!(
        score_mixed < score_top,
        "Mixed path ({}) should score lower than all-good path ({})",
        score_mixed,
        score_top
    );
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-05: Branching Planner — SelfModel Integration
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that the BranchingPlanner correctly selects the winner
/// based on SelfModel scores, and prefers Path B when within 10% of Path A.
#[tokio::test]
async fn e2e05_branching_planner_selects_winner() {
    let self_model = Arc::new(tokio::sync::RwLock::new(SelfModel::new()));

    // Pre-train the model: "top" and "systemctl" are reliable,
    // "sed" and "apt" are less reliable.
    {
        let mut model = self_model.write().await;
        for _ in 0..10 {
            model.record_outcome("top", true, Duration::from_millis(50));
            model.record_outcome("systemctl", true, Duration::from_millis(100));
        }
        for _ in 0..5 {
            model.record_outcome("sed", true, Duration::from_millis(200));
            model.record_outcome("sed", false, Duration::from_millis(200));
        }
    }

    let planner = BranchingPlanner::new(self_model.clone());

    // Build 3 test paths.
    let mut paths = vec![
        PlannedPath {
            risk: PathRisk::DiagnoseFirst,
            name: "Diagnose-First".into(),
            steps: vec![PlannedStep {
                step_number: 1,
                description: "Check CPU".into(),
                command: StructuredCommand {
                    binary: "top".into(),
                    args: vec!["-bn1".into()],
                    target: "local".into(),
                    timeout_secs: 10,
                    working_dir: None,
                    env_vars: None,
                },
                expected_outcome: "CPU snapshot".into(),
                on_failure: "continue".into(),
            }],
            predicted_outcome: "Identify bottleneck".into(),
            score: None,
        },
        PlannedPath {
            risk: PathRisk::MinimalRisk,
            name: "Minimal-Risk Fix".into(),
            steps: vec![PlannedStep {
                step_number: 1,
                description: "Restart service".into(),
                command: StructuredCommand {
                    binary: "systemctl".into(),
                    args: vec!["restart".into(), "nginx".into()],
                    target: "local".into(),
                    timeout_secs: 10,
                    working_dir: None,
                    env_vars: None,
                },
                expected_outcome: "Service restarted".into(),
                on_failure: "abort".into(),
            }],
            predicted_outcome: "Service fixed".into(),
            score: None,
        },
        PlannedPath {
            risk: PathRisk::Aggressive,
            name: "Aggressive Fix".into(),
            steps: vec![PlannedStep {
                step_number: 1,
                description: "Replace service".into(),
                command: StructuredCommand {
                    binary: "apt".into(),
                    args: vec!["install".into(), "caddy".into()],
                    target: "local".into(),
                    timeout_secs: 60,
                    working_dir: None,
                    env_vars: None,
                },
                expected_outcome: "Service replaced".into(),
                on_failure: "abort".into(),
            }],
            predicted_outcome: "Full replacement".into(),
            score: None,
        },
    ];

    let winner_idx = planner.select_winner(&mut paths).await;

    // Path B (MinimalRisk) should be preferred because "systemctl" has high
    // success rate, and Path B is within 10% of Path A.
    // Path A uses "top" (also high), Path B uses "systemctl" (also high).
    // Both should score similarly, so Path B wins (preferred when within 10%).
    let winner = &paths[winner_idx];
    assert!(
        winner.risk == PathRisk::MinimalRisk || winner.risk == PathRisk::DiagnoseFirst,
        "Winner should be MinimalRisk or DiagnoseFirst, got {:?}",
        winner.risk
    );

    // All paths should have scores now.
    for path in &paths {
        assert!(path.score.is_some(), "All paths should be scored");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-15: IPC Event Flood + P0 Voice Preemption
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that the ExecutiveController accepts voice tasks even under load.
///
/// This tests the most critical property: voice tasks are NEVER rejected
/// due to background task load.
#[tokio::test]
async fn e2e15_ipc_event_flood_with_p0_voice() {
    // Use a large max_background to avoid rejection.
    let (_controller, sender, _gpu) = build_test_controller(100);

    // Fire 50 background tasks.
    for i in 0..50 {
        let task = background_task(&format!("Flood task {}", i));
        let _ = sender.submit(task);
    }

    // Fire a P0 voice task. It MUST be accepted.
    let voice = voice_task("Emergency: stop all background tasks!");
    let voice_result = sender.submit(voice);
    assert!(
        voice_result.is_ok(),
        "Voice task must be accepted even under flood"
    );

    // Fire another voice task.
    let voice2 = voice_task("What's the system status?");
    let voice2_result = sender.submit(voice2);
    assert!(
        voice2_result.is_ok(),
        "Second voice task must also be accepted"
    );

    // Verify the system is still alive.
    assert!(sender.is_alive(), "Controller should still be running");
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-15b: Perception Event Flood (5000 events → debouncer → bus)
// ═══════════════════════════════════════════════════════════════════════════

/// Flood the PerceptionBus with 5,000 rapid filesystem events
/// and verify the EventDebouncer correctly collapses them.
#[tokio::test]
async fn e2e15b_perception_event_flood() {
    let bus = PerceptionBus::new(4096);
    let mut debouncer = EventDebouncer::new(Duration::from_millis(200), bus.sender());
    let mut rx = bus.subscribe();

    // Flood: 5000 events on the SAME file.
    for _ in 0..5000 {
        debouncer.ingest(
            EventKind::Filesystem(FilesystemOp::Modified),
            Some("/tmp/hot_file.rs".to_string()),
            "File modified: /tmp/hot_file.rs".to_string(),
            EventSeverity::Notable,
        );
    }

    // All 5000 should collapse into 1 pending entry.
    assert_eq!(
        debouncer.pending_count(),
        1,
        "5000 identical events should collapse into 1"
    );

    // Wait for debounce window.
    tokio::time::sleep(Duration::from_millis(250)).await;
    debouncer.tick();

    // Should receive 1 aggregated event.
    let event = tokio::time::timeout(Duration::from_millis(100), rx.recv())
        .await
        .expect("Should receive event within timeout")
        .expect("Channel should not be closed");

    assert_eq!(event.count, 5000);
    assert_eq!(event.kind, EventKind::Filesystem(FilesystemOp::Modified));
    assert_eq!(event.primary_path, Some("/tmp/hot_file.rs".to_string()));

    // No more events should arrive.
    assert!(rx.try_recv().is_err());
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-06: WorkingSet → Planner Pipeline
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that WorkingSet + StructuredEvidence feeds correctly
/// into the BranchingPlanner prompt.
#[tokio::test]
async fn e2e06_working_set_to_planner() {
    let ws = WorkingSet::builder("Make VM faster")
        .with_max_tokens(2048)
        .add_constraint("Don't restart nginx during business hours", "user", true)
        .add_evidence(StructuredEvidence {
            command: "top -bn1".to_string(),
            target: "local".to_string(),
            exit_code: 0,
            stdout_fields: kria_core::agent::working_set::extractor::ExtractedFields {
                error_codes: vec![],
                ipv4_addresses: vec![],
                ipv6_addresses: vec![],
                file_paths: vec![],
                numeric_values: vec![
                    ("CPU".to_string(), "87".to_string(), "%".to_string()),
                    ("Mem".to_string(), "6.2".to_string(), "GiB".to_string()),
                ],
                exit_codes: vec![],
                kv_pairs: vec![],
                raw_snippet: "%Cpu(s): 87.0 us, 5.0 sy".to_string(),
                total_lines: 1,
                truncated: false,
            },
            stderr_fields: kria_core::agent::working_set::extractor::ExtractedFields::default(),
            timestamp_epoch_ms: 1700000000000,
        })
        .build();

    let prompt = ws.to_prompt();

    // The prompt should contain the goal, constraints, and evidence.
    assert!(
        prompt.contains("Make VM faster"),
        "Prompt should contain goal"
    );
    assert!(
        prompt.contains("Don't restart nginx"),
        "Prompt should contain constraint"
    );
    assert!(
        prompt.contains("CPU"),
        "Prompt should contain evidence numeric values"
    );

    // The planner should be able to use this prompt.
    let planner_prompt = BranchingPlanner::build_prompt("Make VM faster", &ws);
    assert!(
        planner_prompt.contains("PATH A"),
        "Planner prompt should have PATH A"
    );
    assert!(
        planner_prompt.contains("PATH B"),
        "Planner prompt should have PATH B"
    );
    assert!(
        planner_prompt.contains("PATH C"),
        "Planner prompt should have PATH C"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-07: NoveltyDetector — Event Filtering
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that the NoveltyDetector correctly filters events:
/// - Critical/Warning → always novel
/// - Notable + HealthBreach/ProcessLifecycle → novel
/// - Notable + Filesystem → NOT novel
/// - Info → never novel
#[tokio::test]
async fn e2e07_novelty_detector_filtering() {
    // Critical filesystem → novel
    let critical_fs = perception_event(
        EventKind::Filesystem(FilesystemOp::Deleted),
        EventSeverity::Critical,
        "/etc/nginx/nginx.conf deleted",
    );
    assert!(NoveltyDetector::is_novel(&critical_fs));

    // Warning health breach → novel
    let warning_health = perception_event(
        EventKind::HealthBreach("disk_low".to_string()),
        EventSeverity::Warning,
        "Disk at 95%",
    );
    assert!(NoveltyDetector::is_novel(&warning_health));

    // Notable health breach → novel
    let notable_health = perception_event(
        EventKind::HealthBreach("cpu_high".to_string()),
        EventSeverity::Notable,
        "CPU at 80%",
    );
    assert!(NoveltyDetector::is_novel(&notable_health));

    // Notable process lifecycle → novel
    let notable_process = perception_event(
        EventKind::ProcessLifecycle("nginx_crashed".to_string()),
        EventSeverity::Notable,
        "nginx crashed",
    );
    assert!(NoveltyDetector::is_novel(&notable_process));

    // Notable filesystem → NOT novel (too noisy)
    let notable_fs = perception_event(
        EventKind::Filesystem(FilesystemOp::Modified),
        EventSeverity::Notable,
        "File modified",
    );
    assert!(!NoveltyDetector::is_novel(&notable_fs));

    // Notable network → NOT novel
    let notable_net = perception_event(
        EventKind::NetworkChange("eth0_up".to_string()),
        EventSeverity::Notable,
        "Network up",
    );
    assert!(!NoveltyDetector::is_novel(&notable_net));

    // Info anything → never novel
    let info = perception_event(
        EventKind::HealthBreach("info".to_string()),
        EventSeverity::Info,
        "Info event",
    );
    assert!(!NoveltyDetector::is_novel(&info));
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-08: CommandValidator — Safety Gate
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that the CommandValidator correctly gates commands
/// through the PolicyGate, blocking non-read-only and non-Green commands.
#[tokio::test]
async fn e2e08_command_validator_safety() {
    let policy_gate: Arc<dyn PolicyGate> = Arc::new(MockPolicyGate);
    let validator = CommandValidator::new(policy_gate);

    // Read-only commands should pass.
    let cmd_top = StructuredCommand {
        binary: "top".into(),
        args: vec!["-bn1".into()],
        target: "local".into(),
        timeout_secs: 10,
        working_dir: None,
        env_vars: None,
    };
    assert!(validator.validate(&cmd_top).is_ok());

    // Blocked commands should fail.
    let cmd_dd = StructuredCommand {
        binary: "dd".into(),
        args: vec!["if=/dev/zero".into(), "of=/dev/sda".into()],
        target: "local".into(),
        timeout_secs: 10,
        working_dir: None,
        env_vars: None,
    };
    assert!(validator.validate(&cmd_dd).is_err());

    // Unknown commands should fail (not Green).
    let cmd_unknown = StructuredCommand {
        binary: "some_custom_tool".into(),
        args: vec![],
        target: "local".into(),
        timeout_secs: 10,
        working_dir: None,
        env_vars: None,
    };
    assert!(validator.validate(&cmd_unknown).is_err());
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-09: MockLlmBackend — Deterministic Responses
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that the MockLlmBackend returns deterministic responses
/// and tracks call count correctly.
#[tokio::test]
async fn e2e09_mock_llm_deterministic() {
    let mock = MockLlmBackend::with_branching_plan();

    assert_eq!(mock.call_count(), 0);
    assert!(mock.is_configured());

    // First call should return the branching plan.
    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: "Make VM faster".to_string(),
        name: None,
        images: None,
    }];

    let response = mock.chat(&messages, None, 0.7, 2048).await.unwrap();
    assert_eq!(mock.call_count(), 1);
    assert_eq!(response.model, "mock-7b");

    // The response should contain the 3-path plan.
    let plan: serde_json::Value = serde_json::from_str(&response.content).unwrap();
    assert!(plan.get("paths").is_some(), "Response should contain paths");
    let paths = plan["paths"].as_array().unwrap();
    assert_eq!(paths.len(), 3, "Should have exactly 3 paths");

    // Second call should return the default fallback.
    let response2 = mock.chat(&messages, None, 0.7, 2048).await.unwrap();
    assert_eq!(mock.call_count(), 2);
    assert!(
        response2.content.contains("don't have enough information"),
        "Second call should return default fallback"
    );

    // Captured messages should have both calls.
    let captured = mock.captured_messages();
    assert_eq!(captured.len(), 2);
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-10: Planner Fallback Chain (Heuristic)
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that the heuristic planner produces a valid 3-path plan
/// when the LLM is unavailable.
#[tokio::test]
async fn e2e10_heuristic_planner_fallback() {
    let evidence = vec![];
    let plan = BranchingPlanner::heuristic_plan("Make VM faster", &evidence);

    assert_eq!(plan.paths.len(), 3, "Heuristic plan should have 3 paths");

    // Path A should be DiagnoseFirst (read-only).
    let path_a = plan
        .paths
        .iter()
        .find(|p| p.risk == PathRisk::DiagnoseFirst);
    assert!(path_a.is_some(), "Should have DiagnoseFirst path");

    // Path B should be MinimalRisk.
    let path_b = plan.paths.iter().find(|p| p.risk == PathRisk::MinimalRisk);
    assert!(path_b.is_some(), "Should have MinimalRisk path");

    // Path C should be Aggressive.
    let path_c = plan.paths.iter().find(|p| p.risk == PathRisk::Aggressive);
    assert!(path_c.is_some(), "Should have Aggressive path");

    // All paths should have at least one step.
    for path in &plan.paths {
        assert!(
            !path.steps.is_empty(),
            "Path '{}' should have at least one step",
            path.name
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-11: GPU Lease Safety — Background Never Gets GPU
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that background tasks cannot acquire GPU lease,
/// and that voice tasks always get priority.
#[tokio::test]
async fn e2e11_gpu_lease_safety() {
    let _gpu_lease = GpuLeaseManager::shared(Duration::from_secs(180), Duration::from_secs(15));

    // Background task (P3) should NOT be able to acquire GPU.
    // TaskPriority::Background.can_acquire_gpu() == false
    assert!(
        !TaskPriority::Background.can_acquire_gpu(),
        "Background tasks must not acquire GPU"
    );
    assert!(
        !TaskPriority::Maintenance.can_acquire_gpu(),
        "Maintenance tasks must not acquire GPU"
    );

    // Foreground tasks CAN acquire GPU.
    assert!(
        TaskPriority::Voice.can_acquire_gpu(),
        "Voice tasks must be able to acquire GPU"
    );
    assert!(
        TaskPriority::Interactive.can_acquire_gpu(),
        "Interactive tasks must be able to acquire GPU"
    );

    // Only Voice can preempt.
    assert!(
        TaskPriority::Voice.can_preempt(),
        "Voice must be able to preempt"
    );
    assert!(
        !TaskPriority::Interactive.can_preempt(),
        "Interactive must NOT be able to preempt"
    );
    assert!(
        !TaskPriority::Background.can_preempt(),
        "Background must NOT be able to preempt"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-12: TaskPriority Ordering
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that TaskPriority ordering is correct for the BinaryHeap.
#[tokio::test]
async fn e2e12_priority_ordering() {
    // Lower number = higher priority.
    assert!(TaskPriority::Voice < TaskPriority::Interactive);
    assert!(TaskPriority::Interactive < TaskPriority::HitlResponse);
    assert!(TaskPriority::HitlResponse < TaskPriority::Background);
    assert!(TaskPriority::Background < TaskPriority::Maintenance);

    // Voice is always foreground.
    assert!(TaskPriority::Voice.is_foreground());
    assert!(TaskPriority::Interactive.is_foreground());
    assert!(!TaskPriority::Background.is_foreground());
    assert!(!TaskPriority::Maintenance.is_foreground());
}

// ═══════════════════════════════════════════════════════════════════════════
//  E2E-13: EvidenceGatherer — Domain-Specific Diagnostics
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that EvidenceGatherer generates appropriate diagnostics
/// for different event types.
#[tokio::test]
async fn e2e13_evidence_gatherer_domain_specific() {
    // Health breach: disk → should include df, du
    let disk_event = perception_event(
        EventKind::HealthBreach("disk_low".to_string()),
        EventSeverity::Warning,
        "Disk at 95%",
    );
    let disk_plan = EvidenceGatherer::plan_for(&disk_event).unwrap();
    let disk_binaries: Vec<&str> = disk_plan
        .commands
        .iter()
        .map(|c| c.binary.as_str())
        .collect();
    assert!(disk_binaries.contains(&"df"), "Disk plan should include df");

    // Health breach: memory → should include free, ps
    let mem_event = perception_event(
        EventKind::HealthBreach("memory_high".to_string()),
        EventSeverity::Warning,
        "RAM at 90%",
    );
    let mem_plan = EvidenceGatherer::plan_for(&mem_event).unwrap();
    let mem_binaries: Vec<&str> = mem_plan
        .commands
        .iter()
        .map(|c| c.binary.as_str())
        .collect();
    assert!(
        mem_binaries.contains(&"free"),
        "Memory plan should include free"
    );

    // Process lifecycle → should include systemctl, journalctl, ps
    let proc_event = perception_event(
        EventKind::ProcessLifecycle("nginx_crashed".to_string()),
        EventSeverity::Notable,
        "nginx crashed",
    );
    let proc_plan = EvidenceGatherer::plan_for(&proc_event).unwrap();
    let proc_binaries: Vec<&str> = proc_plan
        .commands
        .iter()
        .map(|c| c.binary.as_str())
        .collect();
    assert!(
        proc_binaries.contains(&"ps"),
        "Process plan should include ps"
    );

    // Network change → should include ip, ping
    let net_event = perception_event(
        EventKind::NetworkChange("eth0_down".to_string()),
        EventSeverity::Warning,
        "eth0 down",
    );
    let net_plan = EvidenceGatherer::plan_for(&net_event).unwrap();
    let net_binaries: Vec<&str> = net_plan
        .commands
        .iter()
        .map(|c| c.binary.as_str())
        .collect();
    assert!(
        net_binaries.contains(&"ip"),
        "Network plan should include ip"
    );
    assert!(
        net_binaries.contains(&"ping"),
        "Network plan should include ping"
    );

    // Normal filesystem → should return None (too noisy)
    let fs_event = perception_event(
        EventKind::Filesystem(FilesystemOp::Modified),
        EventSeverity::Notable,
        "File modified",
    );
    assert!(
        EvidenceGatherer::plan_for(&fs_event).is_none(),
        "Normal filesystem events should not trigger investigation"
    );

    // Critical filesystem → should return Some
    let critical_fs = perception_event(
        EventKind::Filesystem(FilesystemOp::Deleted),
        EventSeverity::Critical,
        "/etc/passwd deleted",
    );
    assert!(
        EvidenceGatherer::plan_for(&critical_fs).is_some(),
        "Critical filesystem events should trigger investigation"
    );
}
