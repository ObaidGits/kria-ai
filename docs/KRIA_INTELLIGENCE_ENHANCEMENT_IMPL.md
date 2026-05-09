# KRIA Intelligence Enhancement — Implementation Plan v2

**Status:** Active Implementation (Major Revision)  
**Date:** 2026-05-08  
**Owner:** Systems Architecture  
**Reference:** `docs/KRIA_INTELLIGENCE_MASTERPLAN.md`  
**Scope:** Backend (`kria-core`, `kria-server`, `kria-desktop`) + Frontend (`ui/`)  
**Hardware:** RTX 4050 (6GB VRAM) + 16GB System RAM  

---

## Table of Contents

1. [Executive Summary of Changes from v1](#1-executive-summary-of-changes-from-v1)
2. [Implementation Overview](#2-implementation-overview)
3. [Codebase Map: Existing → New](#3-codebase-map-existing--new)
4. [Phase A: Executive Controller + Task Scheduler](#phase-a-executive-controller--task-scheduler)
5. [Phase B: Command Execution + Policy Engine](#phase-b-command-execution--policy-engine)
6. [Phase C: Uncertainty Engine + Structured Planner](#phase-c-uncertainty-engine--structured-planner)
7. [Phase D: Memory, Skill Learning + Quarantine](#phase-d-memory-skill-learning--quarantine)
8. [Phase E: Event-Driven Perception + Curiosity](#phase-e-event-driven-perception--curiosity)
9. [Phase F: Browser Agent + Dynamic Tools](#phase-f-browser-agent--dynamic-tools)
10. [Frontend Implementation](#10-frontend-implementation)
11. [Server API Changes](#11-server-api-changes)
12. [Integration Testing Strategy](#12-integration-testing-strategy)
13. [Cascading Changes Matrix](#13-cascading-changes-matrix)
14. [Rollout & Rollback](#14-rollout--rollback)
15. [Critical Audit Responses](#15-critical-audit-responses)

---

## 1. Executive Summary of Changes from v1

### Audit Points Addressed

| # | Flaw in v1 | Resolution |
|---|-----------|------------|
| 1 | Raw `execute_shell` string passthrough — unmanageable attack surface | **Replaced** with AST-based `SubprocessExecutor` using structured `CommandRequest` (binary + args). Deterministic `PolicyGate` trait evaluates against allowlist/blocklist rules. |
| 2 | SelfModel raw percentage (`success/total`) — massive small-sample bias | **Replaced** with Beta(α,β) posterior estimation (Bayesian). Unseen tools start at 0.50 neutral prior. |
| 3 | WorkingSet LLM summarization destroys error codes, paths, exact data | **Replaced** with `StructuredExtractor` — preserves raw log snippets with priority-based truncation (error codes > exit codes > structured fields > prose). |
| 4 | Auto-promoting compiled skills after N=3 — risky without human review | **Added** `QuarantineRegistry` with tiered auto-promotion: read-only tools auto-promote; write/destructive tools require HITL approval. |
| 5 | No central brain — parallel modules (Voice, Curiosity) can OOM VRAM | **Added** `ExecutiveController` as Phase A (Priority 0). Central Tokio MPSC event loop with 4-tier priority queue and GPU lease preemption. |
| 6 | 12-week solo timeline — unrealistic | **Extended** to 24 weeks (6 months). Reordered phases: Executive Controller first. |

### Additional Hidden Flaws Found & Fixed

| # | Hidden Flaw | Fix |
|---|------------|-----|
| 7 | No retry/backoff on execution path | Added `ExecutionPolicy` with exponential backoff, max 3 retries per step |
| 8 | Reflection is LLM-only (no deterministic verification) | Added `GoalVerifier` with deterministic post-conditions (file exists, service running, exit code 0) |
| 9 | CuriosityLoop has no resource budget — can starve foreground tasks | Added `BudgetGuard` — max 10% CPU, 0 VRAM, yields on any foreground request |
| 10 | Skill Compiler has no rollback for broken compiled skills | Added `CircuitBreaker` — 3 consecutive failures auto-disables skill, reverts to LLM planning |
| 11 | Dynamic Tool Generation uses LLM-parsed OpenAPI — fragile | Added `SchemaValidator` — generated ToolDef must pass JSON Schema validation + sandbox dry-run before registration |
| 12 | PlanStep lacks dependency graph — sequential-only execution | Added `DependencyGraph` with parallel execution of independent steps |
| 13 | 7B model availability not handled — no fallback path | Added `PlannerFallbackChain`: local 7B → cloud Gemini Flash → simplified heuristic planner |
| 14 | WorkingSet compressor was a stub (`summarize()` placeholder) | Implemented `StructuredExtractor` with deterministic field extraction (no LLM call) |

---

## 2. Implementation Overview

### Guiding Principles

1. **Backward compatible** — Every change degrades gracefully when feature flags are off
2. **Test-driven** — Every module has unit tests before integration tests
3. **Incremental** — Each phase independently deployable
4. **Production-grade** — Error handling, logging, timeouts, circuit breakers, retries
5. **Voice-first** — All latency budgets measured against voice interaction targets (P0 < 100ms dispatch)
6. **Safety-first** — Structured commands, not raw strings. Deterministic policy, not regex.

### Revised Dependency Graph

```
Phase A (Weeks 1-4) — Executive Controller
├── A1: TaskPriority + TaskRequest enums
├── A2: ExecutiveController (Tokio MPSC event loop)
├── A3: TaskScheduler (priority queue + GPU preemption)
└── A4: Voice preemption integration
    ↓
Phase B (Weeks 5-8) — Command Execution + Policy Engine
├── B1: SubprocessExecutor (structured CommandRequest, NOT raw shell)
├── B2: PolicyGate trait + rule engine
├── B3: Code Interpreter (sandboxed script execution)
└── B4: CommandObservability (structured result parsing)
    ↓
Phase C (Weeks 9-13) — Uncertainty + Structured Planner
├── C1: Uncertainty Engine (belief graph + confidence scoring)
├── C2: Evidence Gatherer (read-only diagnostics via PolicyGate)
├── C3: WorkingSet (StructuredExtractor, NOT LLM summarization)
├── C4: Structured Branching Planner (3 forced paths)
├── C5: SelfModel (Beta posterior, NOT raw percentage)
├── C6: GoalVerifier (deterministic post-conditions)
└── C7: PlannerFallbackChain (7B → cloud → heuristic)
    ↓
Phase D (Weeks 14-18) — Memory + Skill Learning
├── D1: World Model (persistent fact store)
├── D2: Failure Analyzer (failure pattern extraction)
├── D3: Skill Compiler (N=3 gating + variable abstraction)
├── D4: QuarantineRegistry (HITL for high-risk skills)
└── D5: CircuitBreaker for compiled skills
    ↓
Phase E (Weeks 19-22) — Perception + Curiosity
├── E1: Event-Driven Perception (inotify + dbus + netlink)
├── E2: CuriosityLoop (with BudgetGuard)
└── E3: Proactive Nudges
    ↓
Phase F (Weeks 23-24) — Browser + Dynamic Tools
├── F1: Browser Agent (ephemeral Python sidecar)
├── F2: Dynamic Tool Generation (with SchemaValidator)
└── F3: Prompt Optimizer
```

### VRAM Budget (Unchanged — Strict Hardware Isolation)

```
CPU RESIDENCY (System RAM — always available):
├── Qwen2.5-0.5B Q4_K_M (Router LLM)     → ~400MB RAM
├── Piper TTS                                → ~200MB RAM
├── Silero VAD                               → ~50MB RAM
└── FastEmbed (multilingual-e5-small)        → ~100MB RAM

GPU RESIDENCY (6GB VRAM — always hot):
├── Qwen2.5-7B-Instruct Q4_K_M (Planner)  → ~4.5GB VRAM
└── Headroom for inference KV cache          → ~1.5GB VRAM

EXPLICIT EVICTION ONLY (user-invoked):
├── Vision Model (Qwen2.5-VL)              → Evicts Planner ONLY when user attaches image
└── Image Generator (ComfyUI)               → Evicts Planner ONLY when user requests image gen
```

**Critical Rule:** The Planner LLM is **permanently resident in VRAM**. It is NEVER evicted for TTS, routing, or background tasks. The Executive Controller enforces this by rejecting GPU lease requests from background tasks when the Planner is active.

---

## 3. Codebase Map: Existing → New

### Existing Modules (Already Built)

| Module | File | Purpose | Used By |
|--------|------|---------|---------|
| Agent Loop | `agent/loop_engine/mod.rs` | Main execution loop | All phases |
| Turn Gate | `agent/turn_gate.rs` | Intent classification + resource planning | Phase A, C |
| Tool Registry | `tools/registry.rs` | Tool registration + execution | Phase B, D |
| Router | `routing/mod.rs` | Semantic routing (Phases 1-5) | Phase C |
| Context | `routing/context.rs` | Conversation context | Phase C |
| Tool Index | `routing/tool_index.rs` | Semantic tool matching | Phase C |
| Intent Classifier | `routing/intent_classifier.rs` | Fine-tuned classifier (ONNX) | Phase C |
| Feedback | `routing/feedback.rs` | Feedback collection | Phase D |
| Memory Store | `memory/store.rs` | SQLite conversation storage | Phase D |
| Fact Manager | `memory/facts.rs` | Fact extraction | Phase D |
| **GPU Lease** | `resource/gpu_lease.rs` | **VRAM management with preemption** | **Phase A** |
| **Safety** | `safety/` | **HITL, PIN, RiskLevel, audit, rollback** | **Phase B** |
| Voice Pipeline | `voice/v2/pipeline.rs` | Voice processing | Phase A |
| **Exec Wrapper** | `infra/isolation.rs` | **Task-level isolation + timeout** | **Phase B** |
| **Environment** | `infra/environment/` | **Local, Docker, QEMU (CommandRequest)** | **Phase B** |
| Proactive | `automation/proactive.rs` | Background monitoring | Phase E |
| Planner | `agent/planner.rs` | Basic numbered-step parser | Phase C (replaced) |

### New Modules (To Be Created)

| Module | Path | Phase | Priority |
|--------|------|-------|----------|
| **Executive Controller** | `agent/executive/` | **A** | **P0** |
| **SubprocessExecutor** | `tools/subprocess_executor.rs` | **B** | **P0** |
| **PolicyGate** | `safety/policy_gate.rs` | **B** | **P0** |
| Uncertainty Engine | `agent/uncertainty/` | C | P0 |
| Working Set (Structured) | `agent/working_set/` | C | P0 |
| Structured Branching Planner | `agent/planner_v2/` | C | P0 |
| Self Model (Bayesian) | `agent/self_model/` | C | P1 |
| Goal Verifier | `agent/goal_verifier.rs` | C | P1 |
| Planner Fallback Chain | `agent/planner_fallback.rs` | C | P1 |
| World Model | `agent/world_model/` | D | P1 |
| Failure Analyzer | `agent/failure_analyzer/` | D | P1 |
| Skill Compiler | `agent/skill_compiler/` | D | P1 |
| Quarantine Registry | `tools/quarantine.rs` | D | P1 |
| Curiosity Loop | `agent/curiosity/` | E | P2 |
| Browser Agent | `tools/browser_agent.rs` | F | P2 |
| Dynamic Tool Generator | `tools/dynamic_gen.rs` | F | P2 |
| Prompt Optimizer | `agent/prompt_optimizer/` | F | P2 |

---

## Phase A: Executive Controller + Task Scheduler

**Duration:** Weeks 1–4  
**Goal:** Central brain that coordinates all subsystems, prevents VRAM OOM, and ensures voice is always P0  
**Risk:** High (touches core execution path) — but backward-compatible via feature flag  
**Why First:** Without this, every other module is a liability. Voice commands will OOM if Curiosity holds the GPU.

### A1: Priority System + Task Model (Week 1)

#### File: `crates/kria-core/src/agent/executive/mod.rs` (NEW)

```rust
//! Executive Controller — Central brain for KRIA.
//!
//! Owns the main event loop. Receives work requests via MPSC,
//! schedules them by priority, manages GPU lease preemption,
//! and ensures voice is always P0.

pub mod scheduler;
pub mod budget_guard;
pub mod preemption;

pub use scheduler::{TaskScheduler, TaskHandle};
pub use budget_guard::BudgetGuard;
pub use preemption::PreemptionManager;
```

#### File: `crates/kria-core/src/agent/executive/types.rs` (NEW)

```rust
use std::time::Duration;

/// Task priority tiers. Lower number = higher priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TaskPriority {
    /// Voice commands, barge-in, emergency stop
    Voice = 0,
    /// Interactive user text commands
    Interactive = 1,
    /// HITL approval responses
    HitlResponse = 2,
    /// Background diagnostics, CuriosityLoop, proactive nudges
    Background = 3,
    /// Maintenance: model downloads, log rotation, skill compilation
    Maintenance = 4,
}

/// Identifies the origin of a task for preemption decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskSource {
    VoicePipeline,
    TextChat,
    HitlGateway,
    CuriosityLoop,
    ProactiveScheduler,
    SkillCompiler,
    Maintenance,
}

/// A unit of work submitted to the Executive Controller.
#[derive(Debug)]
pub struct TaskRequest {
    pub id: uuid::Uuid,
    pub priority: TaskPriority,
    pub source: TaskSource,
    pub requires_gpu: bool,
    pub estimated_gpu_duration: Option<Duration>,
    pub payload: TaskPayload,
    pub cancellation_token: tokio_util::sync::CancellationToken,
}

/// What kind of work the task wants to do.
#[derive(Debug)]
pub enum TaskPayload {
    /// Process a user utterance (voice or text)
    UserTurn { text: String, is_voice: bool },
    /// Execute a planned step
    ExecuteStep { step: crate::agent::planner::PlanStep },
    /// Run background diagnostics
    BackgroundDiagnostics { commands: Vec<DiagnosticCommand> },
    /// Compile a skill from a successful plan
    CompileSkill { plan: SuccessfulPlan },
    /// Gather evidence for uncertainty engine
    GatherEvidence { commands: Vec<DiagnosticCommand> },
    /// Maintenance task (model download, log rotation, etc.)
    Maintenance { description: String },
}

/// Result of task scheduling decision.
#[derive(Debug)]
pub enum ScheduleDecision {
    /// Execute immediately
    Execute(TaskRequest),
    /// Queue (will execute when resources available)
    Enqueue(TaskRequest),
    /// Preempt current task to make room
    Preempt {
        victim: uuid::Uuid,
        replacement: TaskRequest,
    },
    /// Reject (e.g., duplicate voice task)
    Reject { reason: String },
}
```

### A2: ExecutiveController (Weeks 1-2)

#### File: `crates/kria-core/src/agent/executive/controller.rs` (NEW)

```rust
//! Executive Controller — Main event loop coordinator.
//!
//! Architecture:
//! - Single MPSC receiver for all task submissions
//! - Priority queue (BinaryHeap ordered by TaskPriority)
//! - GPU lease integration (acquires/releases via GpuLeaseManager)
//! - Voice preemption (P0 interrupts P3/P4)
//!
//! Concurrency model:
//! - Tasks are submitted via `mpsc::UnboundedSender<TaskRequest>`
//! - Controller runs in a dedicated tokio task
//! - Each accepted task spawns a child tokio task with its own CancellationToken
//! - GPU lease is acquired per-task, not per-controller

use tokio::sync::mpsc;
use std::collections::BinaryHeap;
use std::cmp::Reverse;

pub struct ExecutiveController {
    /// Receives task requests from all subsystems
    rx: mpsc::UnboundedReceiver<TaskRequest>,
    /// Public sender for submitting tasks
    tx: mpsc::UnboundedSender<TaskRequest>,
    /// Priority queue for pending tasks
    queue: BinaryHeap<Reverse<QueuedTask>>,
    /// Currently running tasks (max 1 foreground + unlimited background)
    active_foreground: Option<TaskHandle>,
    active_background: Vec<TaskHandle>,
    /// GPU lease manager (existing)
    gpu_lease: Arc<GpuLeaseManager>,
    /// Policy gate for command execution
    policy_gate: Arc<dyn PolicyGate>,
    /// Max concurrent background tasks
    max_background: usize,
}

struct QueuedTask {
    priority: TaskPriority,
    submitted_at: Instant,
    request: TaskRequest,
}

impl ExecutiveController {
    pub fn new(
        gpu_lease: Arc<GpuLeaseManager>,
        policy_gate: Arc<dyn PolicyGate>,
        max_background: usize,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            rx,
            tx,
            queue: BinaryHeap::new(),
            active_foreground: None,
            active_background: Vec::new(),
            gpu_lease,
            policy_gate,
            max_background,
        }
    }

    /// Get a sender handle for submitting tasks.
    pub fn sender(&self) -> mpsc::UnboundedSender<TaskRequest> {
        self.tx.clone()
    }

    /// Main event loop. Runs until shutdown.
    pub async fn run(&mut self) {
        loop {
            tokio::select! {
                // Receive new task
                Some(task) = self.rx.recv() => {
                    let decision = self.schedule(task);
                    self.execute_decision(decision).await;
                }
                // Foreground task completed
                Some(result) = self.wait_foreground() => {
                    self.on_foreground_complete(result).await;
                }
                // Background task completed
                Some(result) = self.wait_background() => {
                    self.on_background_complete(result);
                }
            }
        }
    }

    fn schedule(&self, task: TaskRequest) -> ScheduleDecision {
        match task.priority {
            TaskPriority::Voice => {
                // Voice is always P0 — preempt background if needed
                if self.active_foreground.is_some() {
                    // Current foreground is also voice/interactive — queue briefly
                    ScheduleDecision::Enqueue(task)
                } else if self.active_background.len() >= self.max_background {
                    // Preempt lowest-priority background task
                    if let Some(victim) = self.find_preemption_victim() {
                        ScheduleDecision::Preempt { victim, replacement: task }
                    } else {
                        ScheduleDecision::Execute(task)
                    }
                } else {
                    ScheduleDecision::Execute(task)
                }
            }
            TaskPriority::Interactive => {
                if self.active_foreground.is_some() {
                    ScheduleDecision::Enqueue(task)
                } else {
                    ScheduleDecision::Execute(task)
                }
            }
            TaskPriority::HitlResponse => {
                // HITL responses bypass queue — they unblock blocked tasks
                ScheduleDecision::Execute(task)
            }
            TaskPriority::Background | TaskPriority::Maintenance => {
                if self.active_background.len() >= self.max_background {
                    ScheduleDecision::Reject {
                        reason: "Background task limit reached".into(),
                    }
                } else {
                    ScheduleDecision::Enqueue(task)
                }
            }
        }
    }

    async fn execute_decision(&mut self, decision: ScheduleDecision) {
        match decision {
            ScheduleDecision::Execute(task) => {
                self.spawn_task(task).await;
            }
            ScheduleDecision::Enqueue(task) => {
                self.queue.push(Reverse(QueuedTask {
                    priority: task.priority,
                    submitted_at: Instant::now(),
                    request: task,
                }));
            }
            ScheduleDecision::Preempt { victim, replacement } => {
                self.preempt_and_replace(victim, replacement).await;
            }
            ScheduleDecision::Reject { reason } => {
                tracing::warn!("Task rejected: {}", reason);
            }
        }
    }

    async fn spawn_task(&mut self, task: TaskRequest) {
        let handle = TaskHandle {
            id: task.id,
            priority: task.priority,
            cancel: task.cancellation_token.clone(),
            join: None, // set after spawn
        };

        // Acquire GPU lease if needed
        if task.requires_gpu {
            let turn_id = task.id.to_string();
            let is_foreground = task.priority <= TaskPriority::Interactive;
            match self.gpu_lease.acquire_lease(
                GpuOwner::L1Worker,
                turn_id,
                is_foreground,
            ).await {
                Ok(guard) => {
                    // Spawn task with GPU guard
                    let join = tokio::spawn(Self::run_task_with_guard(task, guard));
                    // ... store handle
                }
                Err(e) => {
                    tracing::error!("GPU lease acquisition failed: {}", e);
                    // Fall back to cloud or reject
                }
            }
        } else {
            // No GPU needed — spawn directly
            let join = tokio::spawn(Self::run_task(task));
            // ... store handle
        }
    }
}
```

### A3: Voice Preemption (Week 3)

#### File: `crates/kria-core/src/agent/executive/preemption.rs` (NEW)

```rust
//! Preemption Manager — Handles priority-based task interruption.
//!
//! When a voice command arrives while background tasks hold the GPU:
//! 1. Signal cancellation to background task(s)
//! 2. Wait up to 500ms for graceful shutdown
//! 3. Force-kill if not stopped
//! 4. Acquire GPU lease for voice task
//! 5. Voice task executes
//! 6. Background task can resume after voice completes

pub struct PreemptionManager {
    /// Grace period before force-kill
    grace_period: Duration,
}

impl PreemptionManager {
    pub async fn preempt_for_voice(
        &self,
        background_tasks: &mut [TaskHandle],
        gpu_lease: &Arc<GpuLeaseManager>,
    ) -> Result<(), PreemptionError> {
        // 1. Cancel all background tasks
        for task in background_tasks.iter() {
            task.cancel.cancel();
        }

        // 2. Wait for graceful shutdown
        let deadline = Instant::now() + self.grace_period;
        for task in background_tasks.iter_mut() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            // Wait for task to finish or timeout
            tokio::select! {
                _ = task.wait() => {},
                _ = tokio::time::sleep(remaining) => {
                    tracing::warn!("Background task {} did not stop in time", task.id);
                }
            }
        }

        // 3. Force-kill remaining
        for task in background_tasks.iter() {
            if !task.is_finished() {
                task.abort();
            }
        }

        // 4. GPU lease is now free (guards dropped with tasks)
        Ok(())
    }
}
```

### A4: BudgetGuard for Background Tasks (Week 4)

#### File: `crates/kria-core/src/agent/executive/budget_guard.rs` (NEW)

```rust
//! Budget Guard — Resource limits for background tasks.
//!
//! Ensures background tasks (CuriosityLoop, Skill Compiler) never
//! starve foreground tasks of CPU, memory, or GPU.

pub struct BudgetGuard {
    max_cpu_percent: f32,        // 10% for background
    max_memory_mb: u64,          // 512MB for background
    requires_gpu: bool,          // Background tasks NEVER get GPU
    yield_on_foreground: bool,   // Yield when foreground task arrives
}

impl BudgetGuard {
    /// Check if the background task should yield.
    pub fn should_yield(&self, executive: &ExecutiveController) -> bool {
        self.yield_on_foreground && executive.has_active_foreground()
    }

    /// Execute a background task with budget enforcement.
    pub async fn run_with_budget<F, Fut, T>(&self, f: F) -> Result<T, BudgetExceeded>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        // Wrap execution with resource monitoring
        // Yield periodically if foreground tasks are waiting
        f().await
    }
}
```

### Tests: `crates/kria-core/tests/phase8_executive_tests.rs` (NEW)

| Test ID | Test Name | Expected Result |
|---------|-----------|----------------|
| EX01 | `voice_always_p0` | Voice tasks scheduled before all others |
| EX02 | `preemption_cancels_background` | Background task cancelled when voice arrives |
| EX03 | `gpu_lease_acquired_for_planner` | Planner tasks get GPU lease |
| EX04 | `background_never_gets_gpu` | Background tasks rejected from GPU |
| EX05 | `hitl_response_bypasses_queue` | HITL responses execute immediately |
| EX06 | `queue_ordering_by_priority` | Queue orders tasks correctly |
| EX07 | `max_background_enforced` | Background task limit enforced |
| EX08 | `preemption_grace_period` | 500ms grace period before force-kill |
| EX09 | `task_cancellation_propagated` | CancellationToken propagates to child tasks |
| EX10 | `budget_guard_yields` | Background yields when foreground arrives |
| EX11 | `concurrent_foreground_rejected` | Only 1 foreground task at a time |
| EX12 | `maintenance_lowest_priority` | Maintenance tasks scheduled last |

---

## Phase B: Command Execution + Policy Engine

**Duration:** Weeks 5–8  
**Goal:** Replace raw shell passthrough with structured, policy-gated command execution  
**Risk:** Medium (replaces tool execution path)  
**Why Second:** The Executive Controller (Phase A) needs something safe to execute. This gives it that.

### B1: SubprocessExecutor — Structured Commands (Weeks 5-6)

**Critical Design Decision:** The LLM NEVER outputs raw shell strings. It outputs structured `CommandRequest` objects (binary + args). This eliminates entire classes of injection attacks.

#### File: `crates/kria-core/src/tools/subprocess_executor.rs` (NEW)

```rust
//! Structured subprocess executor — replaces raw shell execution.
//!
//! The LLM outputs structured commands:
//! ```json
//! {"binary": "systemctl", "args": ["status", "nginx"], "target": "local"}
//! ```
//!
//! NOT raw shell strings like:
//! ```bash
//! systemctl status nginx; rm -rf /  # INJECTION!
//! ```
//!
//! This struct is the single execution path for all external commands.
//! Every command passes through PolicyGate before execution.

use crate::infra::environment::traits::{CommandRequest, CommandResult, EnvironmentProvider};
use crate::safety::policy_gate::PolicyGate;
use crate::safety::RiskLevel;

/// Structured command submitted by the LLM.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StructuredCommand {
    /// The binary to execute (e.g., "systemctl", "ls", "grep")
    pub binary: String,
    /// Arguments (each element is one argv entry — NO shell parsing)
    pub args: Vec<String>,
    /// Target environment: "local", VM name, or Docker container
    #[serde(default = "default_target")]
    pub target: String,
    /// Timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Working directory (optional)
    pub working_dir: Option<String>,
    /// Environment variables to set (optional)
    pub env_vars: Option<std::collections::HashMap<String, String>>,
}

fn default_target() -> String { "local".into() }
fn default_timeout() -> u64 { 30 }

/// Execution result with structured metadata.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    pub execution_time_ms: u64,
    pub risk_level: RiskLevel,
    pub policy_decision: PolicyDecision,
    pub target: String,
}

/// What the policy gate decided.
#[derive(Debug, Clone, serde::Serialize)]
pub enum PolicyDecision {
    /// Auto-approved (green/read-only)
    AutoApproved,
    /// Required and received HITL approval
    HitlApproved { approver_note: Option<String> },
    /// Blocked by policy
    Blocked { reason: String },
    /// Quarantined (unknown binary — needs user approval)
    Quarantined { reason: String },
}

pub struct SubprocessExecutor {
    policy_gate: Arc<dyn PolicyGate>,
    hitl_gateway: Arc<HitlGateway>,
    environments: Arc<EnvironmentRegistry>,
    audit_logger: Arc<AuditLogger>,
}

impl SubprocessExecutor {
    /// Execute a structured command through the full policy pipeline.
    pub async fn execute(&self, cmd: StructuredCommand) -> ExecutionResult {
        // 1. Validate binary exists on target
        // 2. Run through PolicyGate
        // 3. If approved, execute via EnvironmentProvider
        // 4. Log to audit trail
        // 5. Return structured result

        let policy_decision = self.policy_gate.evaluate(&cmd);

        match &policy_decision {
            PolicyDecision::Blocked { reason } => {
                return ExecutionResult {
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: format!("Blocked by policy: {}", reason),
                    truncated: false,
                    execution_time_ms: 0,
                    risk_level: RiskLevel::Black,
                    policy_decision: policy_decision.clone(),
                    target: cmd.target.clone(),
                };
            }
            PolicyDecision::Quarantined { reason } => {
                // Request HITL approval
                let approved = self.hitl_gateway.request_approval_with_id(
                    uuid::Uuid::new_v4().to_string(),
                    "execute_command",
                    serde_json::to_value(&cmd).unwrap_or_default(),
                    RiskLevel::Red,
                    format!("Unknown binary '{}': {}", cmd.binary, reason),
                    true, // rollback_available
                ).await.unwrap_or(false);

                if !approved {
                    return ExecutionResult {
                        exit_code: -1,
                        stdout: String::new(),
                        stderr: "User rejected execution of unknown binary".into(),
                        truncated: false,
                        execution_time_ms: 0,
                        risk_level: RiskLevel::Red,
                        policy_decision: PolicyDecision::Blocked {
                            reason: "User rejected".into(),
                        },
                        target: cmd.target.clone(),
                    };
                }
            }
            PolicyDecision::HitlApproved { .. } => {
                // Already approved by HITL (yellow/red tier)
            }
            PolicyDecision::AutoApproved => {
                // Green tier — proceed
            }
        }

        // Execute via environment provider
        let env = self.environments.get(&cmd.target);
        let request = CommandRequest {
            program: cmd.binary.clone(),
            args: cmd.args.clone(),
            timeout_ms: Some(cmd.timeout_secs * 1000),
            max_bytes: Some(64 * 1024),  // 64KB output limit
            max_lines: Some(500),
        };

        let start = Instant::now();
        let result = env.execute_command(request, /* shell_state */).await;
        let elapsed = start.elapsed();

        // Audit log
        self.audit_logger.log_command(&cmd, &result, elapsed).await;

        ExecutionResult {
            exit_code: result.as_ref().map(|r| r.exit_code).unwrap_or(-1),
            stdout: result.as_ref().map(|r| r.stdout.clone()).unwrap_or_default(),
            stderr: result.as_ref().map(|r| r.stderr.clone()).unwrap_or_default(),
            truncated: result.as_ref().map(|r| r.truncated).unwrap_or(false),
            execution_time_ms: elapsed.as_millis() as u64,
            risk_level: self.policy_gate.classify_risk(&cmd),
            policy_decision,
            target: cmd.target,
        }
    }
}
```

### B2: PolicyGate — Deterministic Safety Rules (Week 6)

**Design Principle:** The PolicyGate is NOT regex-based. It evaluates the **binary name** and **first-level arguments** against a structured rule table. This is deterministic, auditable, and cannot be bypassed by shell obfuscation (because there IS no shell — the binary and args are separate).

#### File: `crates/kria-core/src/safety/policy_gate.rs` (NEW)

```rust
//! PolicyGate — Deterministic command safety evaluation.
//!
//! Evaluates StructuredCommand against a rule table.
//! Rules are ordered: first match wins.
//!
//! # Intelligence Preservation
//!
//! The PolicyGate is designed to NOT make KRIA "dumb":
//! - Read-only commands (ls, cat, top, ps, df, free, systemctl status, etc.)
//!   are auto-approved without any user interaction.
//! - Common write operations (mkdir, cp, mv, chmod) on allowed paths
//!   are auto-approved with audit logging.
//! - Only truly destructive or unknown operations require HITL approval.
//!
//! This means KRIA can diagnose, explore, and fix most issues without
//! ever asking the user for permission.

use crate::tools::subprocess_executor::{StructuredCommand, PolicyDecision};
use crate::safety::RiskLevel;

pub trait PolicyGate: Send + Sync {
    /// Evaluate a command against the policy rules.
    fn evaluate(&self, cmd: &StructuredCommand) -> PolicyDecision;

    /// Classify the risk level of a command.
    fn classify_risk(&self, cmd: &StructuredCommand) -> RiskLevel;

    /// Check if a binary is known (in allowlist or blocklist).
    fn is_known_binary(&self, binary: &str) -> bool;
}

/// Rule-based policy gate with configurable rules.
pub struct RuleBasedPolicyGate {
    rules: Vec<PolicyRule>,
    blocked_binaries: HashSet<String>,
    allowed_readonly_binaries: HashSet<String>,
    allowed_write_binaries: HashSet<String>,
    blocked_arg_patterns: Vec<(String, Vec<String>)>,  // (binary, args that block)
}

/// A single policy rule. First match wins.
#[derive(Debug, Clone)]
pub struct PolicyRule {
    /// Binary name pattern (exact match or glob)
    pub binary_pattern: String,
    /// Argument pattern (first N args to match)
    pub arg_pattern: ArgPattern,
    /// What to do when this rule matches
    pub action: PolicyAction,
    /// Human-readable description
    pub description: String,
}

#[derive(Debug, Clone)]
pub enum ArgPattern {
    /// Any arguments
    Any,
    /// Exact argument match
    Exact(Vec<String>),
    /// First argument starts with prefix
    Prefix(String),
    /// Argument contains substring
    Contains(String),
    /// No arguments (bare binary)
    None,
}

#[derive(Debug, Clone)]
pub enum PolicyAction {
    /// Auto-approve with risk level
    Allow(RiskLevel),
    /// Block with reason
    Block(String),
    /// Require HITL approval
    RequireApproval { risk_level: RiskLevel, reason: String },
}

impl RuleBasedPolicyGate {
    pub fn new() -> Self {
        let mut gate = Self {
            rules: Vec::new(),
            blocked_binaries: HashSet::new(),
            allowed_readonly_binaries: HashSet::new(),
            allowed_write_binaries: HashSet::new(),
            blocked_arg_patterns: Vec::new(),
        };
        gate.load_default_rules();
        gate
    }

    fn load_default_rules(&mut self) {
        // === GREEN: Auto-approved read-only commands ===
        let readonly_binaries = [
            "ls", "cat", "head", "tail", "wc", "grep", "find", "which",
            "top", "htop", "ps", "df", "du", "free", "uptime", "uname",
            "hostname", "id", "whoami", "pwd", "env", "printenv",
            "systemctl", "journalctl", "dmesg",
            "ip", "ss", "ping", "traceroute", "dig", "nslookup", "host",
            "lscpu", "lspci", "lsusb", "lsblk", "lsmod",
            "git",  // git status, log, diff, show are read-only
            "docker", "podman",  // container listing is read-only
            "jq", "awk", "sed", "tr", "sort", "uniq", "cut",  // text processing
            "file", "stat", "md5sum", "sha256sum",
            "python3", "node",  // running scripts (args determine safety)
        ];
        for binary in &readonly_binaries {
            self.allowed_readonly_binaries.insert(binary.to_string());
        }

        // === GREEN rules: specific read-only patterns ===
        self.rules.push(PolicyRule {
            binary_pattern: "systemctl".into(),
            arg_pattern: ArgPattern::Prefix("status".into()),
            action: PolicyAction::Allow(RiskLevel::Green),
            description: "systemctl status is read-only".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "systemctl".into(),
            arg_pattern: ArgPattern::Prefix("list-units".into()),
            action: PolicyAction::Allow(RiskLevel::Green),
            description: "systemctl list-units is read-only".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "git".into(),
            arg_pattern: ArgPattern::Exact(vec!["status".into()]),
            action: PolicyAction::Allow(RiskLevel::Green),
            description: "git status is read-only".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "git".into(),
            arg_pattern: ArgPattern::Exact(vec!["log".into()]),
            action: PolicyAction::Allow(RiskLevel::Green),
            description: "git log is read-only".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "git".into(),
            arg_pattern: ArgPattern::Exact(vec!["diff".into()]),
            action: PolicyAction::Allow(RiskLevel::Green),
            description: "git diff is read-only".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "docker".into(),
            arg_pattern: ArgPattern::Exact(vec!["ps".into()]),
            action: PolicyAction::Allow(RiskLevel::Green),
            description: "docker ps is read-only".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "docker".into(),
            arg_pattern: ArgPattern::Exact(vec!["images".into()]),
            action: PolicyAction::Allow(RiskLevel::Green),
            description: "docker images is read-only".into(),
        });

        // === YELLOW rules: common write operations on allowed paths ===
        self.rules.push(PolicyRule {
            binary_pattern: "mkdir".into(),
            arg_pattern: ArgPattern::Any,
            action: PolicyAction::Allow(RiskLevel::Yellow),
            description: "mkdir is non-destructive".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "cp".into(),
            arg_pattern: ArgPattern::Any,
            action: PolicyAction::Allow(RiskLevel::Yellow),
            description: "cp is non-destructive (creates copy)".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "mv".into(),
            arg_pattern: ArgPattern::Any,
            action: PolicyAction::Allow(RiskLevel::Yellow),
            description: "mv is reversible within same filesystem".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "chmod".into(),
            arg_pattern: ArgPattern::Any,
            action: PolicyAction::Allow(RiskLevel::Yellow),
            description: "chmod is reversible".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "systemctl".into(),
            arg_pattern: ArgPattern::Prefix("restart".into()),
            action: PolicyAction::Allow(RiskLevel::Yellow),
            description: "service restart is non-destructive".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "systemctl".into(),
            arg_pattern: ArgPattern::Prefix("reload".into()),
            action: PolicyAction::Allow(RiskLevel::Yellow),
            description: "service reload is non-destructive".into(),
        });

        // === RED rules: destructive operations requiring HITL ===
        self.rules.push(PolicyRule {
            binary_pattern: "rm".into(),
            arg_pattern: ArgPattern::Any,
            action: PolicyAction::RequireApproval {
                risk_level: RiskLevel::Red,
                reason: "File deletion is destructive".into(),
            },
            description: "rm requires approval".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "systemctl".into(),
            arg_pattern: ArgPattern::Prefix("stop".into()),
            action: PolicyAction::RequireApproval {
                risk_level: RiskLevel::Red,
                reason: "Stopping a service may cause downtime".into(),
            },
            description: "systemctl stop requires approval".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "systemctl".into(),
            arg_pattern: ArgPattern::Prefix("disable".into()),
            action: PolicyAction::RequireApproval {
                risk_level: RiskLevel::Red,
                reason: "Disabling a service persists across reboots".into(),
            },
            description: "systemctl disable requires approval".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "git".into(),
            arg_pattern: ArgPattern::Exact(vec!["push".into()]),
            action: PolicyAction::RequireApproval {
                risk_level: RiskLevel::Red,
                reason: "git push publishes to remote".into(),
            },
            description: "git push requires approval".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "apt".into(),
            arg_pattern: ArgPattern::Prefix("install".into()),
            action: PolicyAction::RequireApproval {
                risk_level: RiskLevel::Red,
                reason: "Package installation modifies system".into(),
            },
            description: "apt install requires approval".into(),
        });
        self.rules.push(PolicyRule {
            binary_pattern: "apt".into(),
            arg_pattern: ArgPattern::Prefix("remove".into()),
            action: PolicyAction::RequireApproval {
                risk_level: RiskLevel::Red,
                reason: "Package removal may break dependencies".into(),
            },
            description: "apt remove requires approval".into(),
        });

        // === BLACK: Never allowed ===
        self.blocked_binaries.insert("dd".into());
        self.blocked_binaries.insert("mkfs".into());
        self.blocked_binaries.insert("fdisk".into());
        self.blocked_binaries.insert("shutdown".into());
        self.blocked_binaries.insert("reboot".into());
        self.blocked_binaries.insert("poweroff".into());
        self.blocked_binaries.insert("init".into());

        // Blocked arg patterns
        self.blocked_arg_patterns.push((
            "rm".into(),
            vec!["-rf".into(), "/".into()],
        ));
        self.blocked_arg_patterns.push((
            "rm".into(),
            vec!["-rf".into(), "--no-preserve-root".into(), "/".into()],
        ));
    }
}

impl PolicyGate for RuleBasedPolicyGate {
    fn evaluate(&self, cmd: &StructuredCommand) -> PolicyDecision {
        // 1. Check blocked binaries
        if self.blocked_binaries.contains(&cmd.binary) {
            return PolicyDecision::Blocked {
                reason: format!("Binary '{}' is permanently blocked", cmd.binary),
            };
        }

        // 2. Check blocked arg patterns
        for (blocked_binary, blocked_args) in &self.blocked_arg_patterns {
            if cmd.binary == *blocked_binary && cmd.args.starts_with(blocked_args) {
                return PolicyDecision::Blocked {
                    reason: format!("Command matches blocked pattern: {} {}",
                        cmd.binary, blocked_args.join(" ")),
                };
            }
        }

        // 3. Check ordered rules (first match wins)
        for rule in &self.rules {
            if self.matches_rule(cmd, rule) {
                return match &rule.action {
                    PolicyAction::Allow(risk) => {
                        if *risk <= RiskLevel::Green {
                            PolicyDecision::AutoApproved
                        } else {
                            PolicyDecision::AutoApproved // Yellow also auto-approves
                        }
                    }
                    PolicyAction::Block(reason) => {
                        PolicyDecision::Blocked { reason: reason.clone() }
                    }
                    PolicyAction::RequireApproval { risk_level, reason } => {
                        PolicyDecision::HitlApproved {
                            approver_note: Some(reason.clone()),
                        }
                    }
                };
            }
        }

        // 4. Default: unknown binary → quarantine
        if !self.is_known_binary(&cmd.binary) {
            return PolicyDecision::Quarantined {
                reason: format!("Binary '{}' is not in the known-safe list", cmd.binary),
            };
        }

        // 5. Known binary, no specific rule → classify by type
        if self.allowed_readonly_binaries.contains(&cmd.binary) {
            PolicyDecision::AutoApproved
        } else if self.allowed_write_binaries.contains(&cmd.binary) {
            PolicyDecision::AutoApproved
        } else {
            PolicyDecision::RequireApproval {
                risk_level: RiskLevel::Yellow,
                reason: format!("No specific policy for '{}'", cmd.binary),
            }
        }
    }

    fn classify_risk(&self, cmd: &StructuredCommand) -> RiskLevel {
        if self.blocked_binaries.contains(&cmd.binary) {
            return RiskLevel::Black;
        }
        if self.allowed_readonly_binaries.contains(&cmd.binary) {
            return RiskLevel::Green;
        }
        if self.allowed_write_binaries.contains(&cmd.binary) {
            return RiskLevel::Yellow;
        }
        RiskLevel::Yellow // Unknown = treat as yellow
    }

    fn is_known_binary(&self, binary: &str) -> bool {
        self.allowed_readonly_binaries.contains(binary)
            || self.allowed_write_binaries.contains(binary)
            || self.blocked_binaries.contains(binary)
            || self.rules.iter().any(|r| r.binary_pattern == binary)
    }
}
```

### B3: Code Interpreter (Week 7)

#### File: `crates/kria-core/src/tools/code_interpreter.rs` (NEW)

```rust
//! Code interpreter — LLM-generated code execution in sandbox.
//!
//! The LLM writes Python or shell scripts which are executed in:
//! 1. QEMU VM (default, safest) — via existing SSH environment
//! 2. Docker container — via existing Docker environment
//! 3. Local machine — only for green-tier code
//!
//! Scripts are written to temp files, executed, and cleaned up.
//! Output is captured with strict limits (64KB, 500 lines).
//!
//! # Safety
//! - Scripts execute inside the target environment's sandbox
//! - Docker: readonly rootfs, seccomp, PID limit, memory limit
//! - QEMU: full VM isolation
//! - Local: PolicyGate must approve the script runner (python3/bash)

pub struct CodeInterpreter {
    executor: Arc<SubprocessExecutor>,
    environments: Arc<EnvironmentRegistry>,
    max_execution_time: Duration,
    max_output_size: usize,
}

pub enum ScriptLanguage {
    Python,
    Shell,
    NodeJs,
}

pub struct CodeExecutionRequest {
    pub code: String,
    pub language: ScriptLanguage,
    pub target: String,        // "local", VM name, Docker container
    pub timeout_secs: u64,
}

impl CodeInterpreter {
    pub async fn execute(&self, request: CodeExecutionRequest) -> ExecutionResult {
        // 1. Write script to temp file on target
        // 2. Execute via SubprocessExecutor (python3 /tmp/kria_code_XXXX.py)
        // 3. Capture output
        // 4. Clean up temp file
        // 5. Return structured result
    }
}
```

### Tests: `crates/kria-core/tests/phase8_policy_gate_tests.rs` (NEW)

| Test ID | Test Name | Expected Result |
|---------|-----------|----------------|
| PG01 | `readonly_commands_auto_approved` | `ls`, `cat`, `top` → AutoApproved |
| PG02 | `systemctl_status_auto_approved` | `systemctl status nginx` → AutoApproved |
| PG03 | `git_status_auto_approved` | `git status` → AutoApproved |
| PG04 | `mkdir_auto_approved` | `mkdir /tmp/test` → AutoApproved (Yellow) |
| PG05 | `rm_requires_approval` | `rm file.txt` → HitlApproved |
| PG06 | `systemctl_stop_requires_approval` | `systemctl stop nginx` → HitlApproved |
| PG07 | `git_push_requires_approval` | `git push` → HitlApproved |
| PG08 | `dd_blocked` | `dd if=/dev/zero of=/dev/sda` → Blocked |
| PG09 | `shutdown_blocked` | `shutdown -h now` → Blocked |
| PG10 | `rm_rf_root_blocked` | `rm -rf /` → Blocked |
| PG11 | `unknown_binary_quarantined` | `some_random_tool` → Quarantined |
| PG12 | `shell_injection_impossible` | StructuredCommand with `;` in args → passed as literal arg, not interpreted |
| PG13 | `docker_ps_auto_approved` | `docker ps` → AutoApproved |
| PG14 | `apt_install_requires_approval` | `apt install nginx` → HitlApproved |
| PG15 | `policy_gate_classifies_risk` | Risk levels match expected for each command |

### Tests: `crates/kria-core/tests/phase8_subprocess_tests.rs` (NEW)

| Test ID | Test Name | Expected Result |
|---------|-----------|----------------|
| SP01 | `execute_structured_command` | Command executes and returns stdout |
| SP02 | `binary_args_separate` | Binary and args passed separately (no shell) |
| SP03 | `timeout_enforced` | Long commands killed at timeout |
| SP04 | `output_truncation` | Output truncated at 64KB/500 lines |
| SP05 | `audit_log_recorded` | Every command logged to audit trail |
| SP06 | `blocked_command_returns_error` | Blocked commands return error, not executed |
| SP07 | `quarantined_command_requests_hitl` | Unknown binaries trigger HITL |
| SP08 | `execution_time_tracked` | Execution time recorded accurately |
| SP09 | `stderr_captured` | Stderr captured separately from stdout |
| SP10 | `ssh_execution_via_structured_command` | VM commands execute via SSH |

### Tests: `crates/kria-core/tests/phase8_code_interpreter_tests.rs` (NEW)

| Test ID | Test Name | Expected Result |
|---------|-----------|----------------|
| CI01 | `execute_python_hello_world` | Returns "Hello, World!" |
| CI02 | `execute_shell_script` | Returns expected output |
| CI03 | `execution_timeout_enforced` | Long scripts killed at timeout |
| CI04 | `output_capture_stdout_stderr` | Both streams captured |
| CI05 | `vm_execution_default` | Scripts execute on VM by default |
| CI06 | `docker_execution` | Scripts execute in Docker when specified |
| CI07 | `error_handling_returns_exit_code` | Non-zero exit codes captured |
| CI08 | `large_output_truncation` | Output truncated at max_output_size |
| CI09 | `temp_file_cleanup` | Temp files cleaned after execution |
| CI10 | `concurrent_executions_safe` | Multiple scripts don't interfere |

---

## Phase C: Uncertainty Engine + Structured Planner

**Duration:** Weeks 9–13  
**Goal:** Add confidence scoring, structured planning, and goal verification  
**Risk:** Medium (modifies planning loop)  
**Why Third:** Requires Phase B's safe execution to gather evidence and execute plans.

### C1: Uncertainty Engine (Weeks 9-10)

#### File: `crates/kria-core/src/agent/uncertainty/mod.rs` (NEW)

```rust
//! Uncertainty Engine — Confidence scoring before planning.
//!
//! Before any planning or execution, the system scores its confidence
//! in understanding the user's goal. If confidence is low, it gathers
//! evidence (read-only commands via PolicyGate) or asks the user.
//!
//! # Key Principle
//! The 0.5B router should NEVER guess. If uncertain, gather evidence
//! or ask the user. The 7B planner is only woken when confidence exceeds
//! the threshold AND the task requires reasoning.

pub mod belief_graph;
pub mod calibrator;
pub mod evidence_gatherer;

pub use belief_graph::{BeliefGraph, BeliefFact, BeliefSource};
pub use calibrator::ConfidenceCalibrator;
pub use evidence_gatherer::EvidenceGatherer;
```

#### File: `crates/kria-core/src/agent/uncertainty/belief_graph.rs` (NEW)

```rust
//! Belief graph — tracks current system state assumptions.
//!
//! Each fact has a confidence score, evidence chain, and source.
//! Facts are updated when new evidence arrives.
//! Old facts decay in confidence over time.

use std::time::Instant;

pub struct BeliefGraph {
    facts: Vec<BeliefFact>,
    /// How fast confidence decays without re-verification
    decay_rate_per_hour: f32,  // 0.05 = lose 5% confidence per hour
}

#[derive(Debug, Clone)]
pub struct BeliefFact {
    pub proposition: String,       // "Nginx is running"
    pub confidence: f32,           // 0.0-1.0
    pub evidence: Vec<String>,     // ["systemctl status nginx: active (exit 0)"]
    pub source: BeliefSource,
    pub last_verified: Instant,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BeliefSource {
    Detected,      // System command output
    UserStated,    // User told us
    Inferred,      // LLM reasoned about it
    Compiled,      // Skill compiler output
}

impl BeliefGraph {
    /// Store or update a fact. If the fact already exists, update confidence
    /// using Bayesian update: new_confidence = prior * likelihood.
    pub fn update(&mut self, proposition: &str, confidence: f32, evidence: String, source: BeliefSource) {
        if let Some(fact) = self.facts.iter_mut().find(|f| f.proposition == proposition) {
            // Bayesian update: combine old and new confidence
            let combined = 1.0 - (1.0 - fact.confidence) * (1.0 - confidence);
            fact.confidence = combined;
            fact.evidence.push(evidence);
            fact.source = source;
            fact.last_verified = Instant::now();
        } else {
            self.facts.push(BeliefFact {
                proposition: proposition.to_string(),
                confidence,
                evidence: vec![evidence],
                source,
                last_verified: Instant::now(),
            });
        }
    }

    /// Decay confidence of all facts based on time since last verification.
    pub fn decay(&mut self) {
        let now = Instant::now();
        for fact in &mut self.facts {
            let hours = now.duration_since(fact.last_verified).as_secs_f32() / 3600.0;
            fact.confidence *= (-self.decay_rate_per_hour * hours).exp();
        }
    }

    /// Get overall confidence for a set of propositions.
    pub fn confidence_for(&self, propositions: &[&str]) -> f32 {
        let relevant: Vec<f32> = self.facts.iter()
            .filter(|f| propositions.iter().any(|p| f.proposition.contains(p)))
            .map(|f| f.confidence)
            .collect();
        if relevant.is_empty() {
            0.0 // No information = zero confidence
        } else {
            // Geometric mean — if any fact is uncertain, overall is uncertain
            let product: f32 = relevant.iter().product();
            product.powf(1.0 / relevant.len() as f32)
        }
    }
}
```

#### File: `crates/kria-core/src/agent/uncertainty/calibrator.rs` (NEW)

```rust
//! Adaptive threshold calibration using Beta distribution.
//!
//! Instead of arbitrary thresholds, we use Bayesian calibration:
//! After each task, we update the Beta(α, β) distribution for each
//! threshold zone. The thresholds converge to their empirically
//! optimal values over time.

pub struct ConfidenceCalibrator {
    /// Historical outcomes: (predicted_confidence, actual_success)
    outcomes: Vec<(f32, bool)>,
    /// Beta distribution parameters for each zone
    plan_alpha: f32,      // Successes in plan zone
    plan_beta: f32,       // Failures in plan zone
    gather_alpha: f32,    // Successes in gather zone
    gather_beta: f32,     // Failures in gather zone
    ask_alpha: f32,       // Successes in ask zone
    ask_beta: f32,        // Failures in ask zone
    /// Current thresholds (recalibrated periodically)
    plan_threshold: f32,
    gather_threshold: f32,
    ask_threshold: f32,
}

impl ConfidenceCalibrator {
    pub fn new() -> Self {
        Self {
            outcomes: Vec::new(),
            plan_alpha: 8.0,    // Prior: 8 successes
            plan_beta: 2.0,     // Prior: 2 failures → threshold ≈ 0.8
            gather_alpha: 6.0,  // Prior: 6 successes
            gather_beta: 4.0,   // Prior: 4 failures → threshold ≈ 0.6
            ask_alpha: 3.0,     // Prior: 3 successes
            ask_beta: 7.0,      // Prior: 7 failures → threshold ≈ 0.3
            plan_threshold: 0.8,
            gather_threshold: 0.6,
            ask_threshold: 0.3,
        }
    }

    pub fn evaluate(&self, confidence: f32) -> UncertaintyAction {
        match confidence {
            c if c >= self.plan_threshold => UncertaintyAction::Plan,
            c if c >= self.gather_threshold => UncertaintyAction::GatherEvidence,
            c if c >= self.ask_threshold => UncertaintyAction::AskUser,
            _ => UncertaintyAction::Refuse,
        }
    }

    /// Record an outcome and recalibrate thresholds.
    pub fn record_outcome(&mut self, confidence: f32, success: bool) {
        self.outcomes.push((confidence, success));

        // Update the appropriate Beta distribution
        if confidence >= self.plan_threshold {
            if success { self.plan_alpha += 1.0; } else { self.plan_beta += 1.0; }
        } else if confidence >= self.gather_threshold {
            if success { self.gather_alpha += 1.0; } else { self.gather_beta += 1.0; }
        } else if confidence >= self.ask_threshold {
            if success { self.ask_alpha += 1.0; } else { self.ask_beta += 1.0; }
        }

        // Recalibrate thresholds (posterior mean of each Beta)
        self.plan_threshold = self.plan_alpha / (self.plan_alpha + self.plan_beta);
        self.gather_threshold = self.gather_alpha / (self.gather_alpha + self.gather_beta);
        self.ask_threshold = self.ask_alpha / (self.ask_alpha + self.ask_beta);

        // Ensure ordering: plan > gather > ask
        self.gather_threshold = self.gather_threshold.min(self.plan_threshold - 0.05);
        self.ask_threshold = self.ask_threshold.min(self.gather_threshold - 0.05);
    }
}

pub enum UncertaintyAction {
    Plan,
    GatherEvidence,
    AskUser,
    Refuse,
}
```

#### File: `crates/kria-core/src/agent/uncertainty/evidence_gatherer.rs` (NEW)

```rust
//! Evidence gathering — read-only diagnostic commands.
//!
//! Uses PolicyGate to ensure ALL diagnostic commands are auto-approved
//! (read-only). No HITL required for evidence gathering.

pub struct EvidenceGatherer {
    /// Known diagnostic playbooks indexed by domain
    diagnostic_playbooks: HashMap<String, Vec<DiagnosticCommand>>,
    /// Subprocess executor for running diagnostics
    executor: Arc<SubprocessExecutor>,
}

pub struct DiagnosticCommand {
    pub binary: String,
    pub args: Vec<String>,
    pub target: String,
    pub expected_pattern: String,  // What we're looking for in output
    pub timeout_secs: u64,
}

impl EvidenceGatherer {
    /// Given a goal, return read-only diagnostic commands to gather evidence.
    pub fn plan_diagnostics(&self, goal: &str, world_model: &WorldModel) -> Vec<StructuredCommand> {
        let mut commands = Vec::new();

        // Always start with basic system info
        commands.push(StructuredCommand {
            binary: "uptime".into(),
            args: vec![],
            target: "local".into(),
            timeout_secs: 5,
            working_dir: None,
            env_vars: None,
        });

        // Domain-specific diagnostics
        let goal_lower = goal.to_lowercase();
        if goal_lower.contains("vm") || goal_lower.contains("server") {
            commands.extend(self.vm_diagnostics());
        }
        if goal_lower.contains("slow") || goal_lower.contains("performance") {
            commands.extend(self.performance_diagnostics());
        }
        if goal_lower.contains("disk") || goal_lower.contains("space") {
            commands.extend(self.disk_diagnostics());
        }
        if goal_lower.contains("network") || goal_lower.contains("connect") {
            commands.extend(self.network_diagnostics());
        }

        commands
    }

    fn vm_diagnostics(&self) -> Vec<StructuredCommand> {
        vec![
            StructuredCommand { binary: "top".into(), args: vec!["-bn1".into(), "-w512".into()], target: "local".into(), timeout_secs: 10, working_dir: None, env_vars: None },
            StructuredCommand { binary: "free".into(), args: vec!["-h".into()], target: "local".into(), timeout_secs: 5, working_dir: None, env_vars: None },
            StructuredCommand { binary: "df".into(), args: vec!["-h".into()], target: "local".into(), timeout_secs: 5, working_dir: None, env_vars: None },
            StructuredCommand { binary: "systemctl".into(), args: vec!["list-units".into(), "--type=service".into(), "--state=running".into()], target: "local".into(), timeout_secs: 10, working_dir: None, env_vars: None },
        ]
    }

    fn performance_diagnostics(&self) -> Vec<StructuredCommand> {
        vec![
            StructuredCommand { binary: "top".into(), args: vec!["-bn1".into(), "-o".into(), "%CPU".into()], target: "local".into(), timeout_secs: 10, working_dir: None, env_vars: None },
            StructuredCommand { binary: "iostat".into(), args: vec!["-x".into(), "1".into(), "1".into()], target: "local".into(), timeout_secs: 10, working_dir: None, env_vars: None },
            StructuredCommand { binary: "vmstat".into(), args: vec!["1".into(), "2".into()], target: "local".into(), timeout_secs: 10, working_dir: None, env_vars: None },
        ]
    }

    fn disk_diagnostics(&self) -> Vec<StructuredCommand> {
        vec![
            StructuredCommand { binary: "df".into(), args: vec!["-h".into()], target: "local".into(), timeout_secs: 5, working_dir: None, env_vars: None },
            StructuredCommand { binary: "du".into(), args: vec!["-sh".into(), "/var/log".into(), "/tmp".into(), "/home".into()], target: "local".into(), timeout_secs: 30, working_dir: None, env_vars: None },
        ]
    }

    fn network_diagnostics(&self) -> Vec<StructuredCommand> {
        vec![
            StructuredCommand { binary: "ip".into(), args: vec!["addr".into(), "show".into()], target: "local".into(), timeout_secs: 5, working_dir: None, env_vars: None },
            StructuredCommand { binary: "ss".into(), args: vec!["-tuln".into()], target: "local".into(), timeout_secs: 5, working_dir: None, env_vars: None },
            StructuredCommand { binary: "ping".into(), args: vec!["-c".into(), "3".into(), "8.8.8.8".into()], target: "local".into(), timeout_secs: 10, working_dir: None, env_vars: None },
        ]
    }
}
```

### C2: WorkingSet — StructuredExtractor (NOT LLM Summarization) (Week 10)

**Critical Design Decision:** The WorkingSet does NOT use LLM summarization. LLMs destroy exact data (error codes, file paths, numeric values) that the Planner needs. Instead, it uses deterministic field extraction with priority-based truncation.

#### File: `crates/kria-core/src/agent/working_set/mod.rs` (NEW)

```rust
//! WorkingSet — Cognitive scratchpad for the Planner.
//!
//! Compresses conversation history, system state, and evidence into
//! a compact representation that fits in the 7B model's context window.
//!
//! # Key Design: StructuredExtractor (NOT LLM Summarization)
//!
//! Previous approach: Use LLM to summarize raw output into prose.
//! Problem: LLMs destroy exact data (error codes, paths, numeric values).
//!
//! New approach: Extract structured fields deterministically:
//! - Error codes → preserved verbatim
//! - Exit codes → preserved verbatim
//! - File paths → preserved verbatim
//! - IP addresses → preserved verbatim
//! - Numeric values → preserved verbatim
//! - Prose → truncated by line count (least priority)
//!
//! This ensures the Planner gets exact data, not lossy summaries.

pub mod extractor;
pub use extractor::StructuredExtractor;

pub struct WorkingSet {
    /// The active goal stack (what we're trying to achieve)
    pub goal_stack: Vec<Goal>,
    /// Unresolved questions from the current task
    pub open_questions: Vec<String>,
    /// Immediate constraints (e.g., "don't restart nginx during business hours")
    pub constraints: Vec<Constraint>,
    /// Key evidence — structured, NOT summarized
    pub evidence: Vec<StructuredEvidence>,
    /// Max tokens for the WorkingSet (prevents context bloat)
    pub max_tokens: usize,
}

/// Structured evidence preserves exact data from command output.
#[derive(Debug, Clone)]
pub struct StructuredEvidence {
    /// What command produced this evidence
    pub source_command: String,
    /// Exit code (exact)
    pub exit_code: Option<i32>,
    /// Extracted error codes (exact, e.g., "ECONNREFUSED", "404")
    pub error_codes: Vec<String>,
    /// Extracted file paths (exact)
    pub file_paths: Vec<String>,
    /// Extracted IP addresses (exact)
    pub ip_addresses: Vec<String>,
    /// Extracted numeric values with context (exact, e.g., "CPU: 87%")
    pub numeric_values: Vec<(String, String)>,
    /// Key-value pairs extracted from structured output
    pub fields: Vec<(String, String)>,
    /// Raw output snippet (truncated, lowest priority)
    pub raw_snippet: String,
    /// Which target produced this
    pub target: String,
}
```

#### File: `crates/kria-core/src/agent/working_set/extractor.rs` (NEW)

```rust
//! StructuredExtractor — Deterministic field extraction from command output.
//!
//! NO LLM calls. Pure regex + heuristics. Fast and reliable.

use regex::Regex;

pub struct StructuredExtractor {
    error_code_re: Regex,
    ip_addr_re: Regex,
    file_path_re: Regex,
    numeric_kv_re: Regex,
    exit_code_re: Regex,
}

impl StructuredExtractor {
    pub fn new() -> Self {
        Self {
            error_code_re: Regex::new(r"\b([A-Z][A-Z0-9_]{2,})\b").unwrap(),
            ip_addr_re: Regex::new(r"\b(\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})\b").unwrap(),
            file_path_re: Regex::new(r"(/[\w./\-]+)").unwrap(),
            numeric_kv_re: Regex::new(r"(\w[\w\s]*?):\s*(\d+\.?\d*)\s*(%|MB|GB|KB|ms|s)?").unwrap(),
            exit_code_re: Regex::new(r"(?:exit|code|status)[=:]?\s*(\d+)").unwrap(),
        }
    }

    /// Extract structured evidence from raw command output.
    pub fn extract(
        &self,
        command: &str,
        stdout: &str,
        stderr: &str,
        exit_code: i32,
        target: &str,
        max_snippet_lines: usize,
    ) -> StructuredEvidence {
        let combined = format!("{}\n{}", stdout, stderr);

        StructuredEvidence {
            source_command: command.to_string(),
            exit_code: Some(exit_code),
            error_codes: self.extract_error_codes(&combined),
            file_paths: self.extract_file_paths(&combined),
            ip_addresses: self.extract_ips(&combined),
            numeric_values: self.extract_numeric_values(&combined),
            fields: self.extract_fields(&combined),
            raw_snippet: self.truncate_lines(&combined, max_snippet_lines),
            target: target.to_string(),
        }
    }

    fn extract_error_codes(&self, text: &str) -> Vec<String> {
        self.error_code_re.captures_iter(text)
            .map(|c| c[1].to_string())
            .filter(|s| s.len() <= 30)  // Reasonable error code length
            .collect()
    }

    fn extract_ips(&self, text: &str) -> Vec<String> {
        self.ip_addr_re.captures_iter(text)
            .map(|c| c[1].to_string())
            .collect()
    }

    fn extract_file_paths(&self, text: &str) -> Vec<String> {
        self.file_path_re.captures_iter(text)
            .map(|c| c[1].to_string())
            .filter(|p| p.len() > 3 && p.contains('/'))
            .collect()
    }

    fn extract_numeric_values(&self, text: &str) -> Vec<(String, String)> {
        self.numeric_kv_re.captures_iter(text)
            .map(|c| {
                let key = c[1].trim().to_string();
                let value = match c.get(3) {
                    Some(unit) => format!("{}{}", &c[2], unit.as_str()),
                    None => c[2].to_string(),
                };
                (key, value)
            })
            .collect()
    }

    fn extract_fields(&self, text: &str) -> Vec<(String, String)> {
        // Extract key: value pairs from structured output
        text.lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(2, ':').collect();
                if parts.len() == 2 && parts[0].len() < 50 {
                    Some((parts[0].trim().to_string(), parts[1].trim().to_string()))
                } else {
                    None
                }
            })
            .collect()
    }

    fn truncate_lines(&self, text: &str, max_lines: usize) -> String {
        text.lines().take(max_lines).collect::<Vec<_>>().join("\n")
    }

    /// Build WorkingSet from current task state.
    /// Priority: error codes > exit codes > structured fields > raw snippet
    pub fn build(
        goal: &str,
        world_model: &WorldModel,
        evidence: &[StructuredEvidence],
        constraints: &[Constraint],
        max_tokens: usize,
    ) -> WorkingSet {
        let mut ws = WorkingSet {
            goal_stack: vec![Goal { description: goal.to_string(), status: GoalStatus::Active }],
            open_questions: Vec::new(),
            constraints: constraints.to_vec(),
            evidence: evidence.to_vec(),
            max_tokens,
        };

        // Truncate evidence by priority if over budget
        ws.fit_to_budget();
        ws
    }

    fn fit_to_budget(&mut self) {
        let mut total_tokens = self.estimate_tokens();
        while total_tokens > self.max_tokens && !self.evidence.is_empty() {
            // Remove lowest-priority evidence (raw snippets first, then oldest)
            if let Some(last) = self.evidence.last_mut() {
                if !last.raw_snippet.is_empty() {
                    last.raw_snippet.clear();
                } else {
                    self.evidence.pop();
                }
            }
            total_tokens = self.estimate_tokens();
        }
    }

    fn estimate_tokens(&self) -> usize {
        // Rough: 1 token ≈ 4 chars
        let text = self.to_prompt_context();
        text.len() / 4
    }

    pub fn to_prompt_context(&self) -> String {
        let mut out = String::new();

        out.push_str("## GOAL\n");
        for goal in &self.goal_stack {
            out.push_str(&format!("- {}\n", goal.description));
        }

        if !self.constraints.is_empty() {
            out.push_str("\n## CONSTRAINTS\n");
            for c in &self.constraints {
                out.push_str(&format!("- {}\n", c.description));
            }
        }

        if !self.evidence.is_empty() {
            out.push_str("\n## EVIDENCE\n");
            for ev in &self.evidence {
                out.push_str(&format!### [{}] (exit: {})\n",
                    ev.source_command,
                    ev.exit_code.map(|c| c.to_string()).unwrap_or("?".into())
                ));
                if !ev.error_codes.is_empty() {
                    out.push_str(&format!("  Error codes: {}\n", ev.error_codes.join(", ")));
                }
                if !ev.numeric_values.is_empty() {
                    for (k, v) in &ev.numeric_values {
                        out.push_str(&format!("  {}: {}\n", k, v));
                    }
                }
                if !ev.raw_snippet.is_empty() {
                    out.push_str(&format!("  Output: {}\n", ev.raw_snippet));
                }
            }
        }

        if !self.open_questions.is_empty() {
            out.push_str("\n## OPEN QUESTIONS\n");
            for q in &self.open_questions {
                out.push_str(&format!("- {}\n", q));
            }
        }

        out
    }
}

#[derive(Debug, Clone)]
pub struct Goal {
    pub description: String,
    pub status: GoalStatus,
}

#[derive(Debug, Clone)]
pub enum GoalStatus {
    Active,
    Achieved,
    Failed { reason: String },
    Suspended,
}

#[derive(Debug, Clone)]
pub struct Constraint {
    pub description: String,
    pub source: String,
    pub hard: bool,  // hard = cannot be violated, soft = prefer not to
}

---

### C3: Structured Branching Planner (Weeks 11-12)

**What:** Replace linear planning with 3 forced templates, scored by SelfModel.

#### File: `crates/kria-core/src/agent/planner_v2/mod.rs` (NEW)

```rust
//! Structured Branching Planner (replaces linear planner).
//!
//! Forces the 7B model to generate exactly 3 paths:
//! - PATH A: Diagnose-First (read-only, safe)
//! - PATH B: Minimal-Risk Fix (reversible)
//! - PATH C: Aggressive Fix (potentially irreversible)
//!
//! Each path is scored against SelfModel for historical success rates.
//! The Planner ONLY reads the WorkingSet, not the full conversation.

pub mod decomposer;
pub mod scaffolding;
pub mod executor;

pub use decomposer::BranchingPlanner;
pub use executor::PlanExecutor;
```

#### File: `crates/kria-core/src/agent/planner_v2/decomposer.rs` (NEW)

```rust
//! Goal decomposition with structured branching.

pub struct BranchingPlanner {
    llm_backend: LlmBackend,  // 7B model via llama.cpp
    self_model: Arc<RwLock<SelfModel>>,
    failure_analyzer: Arc<RwLock<FailureAnalyzer>>,
    scaffolding: ChainOfThoughtScaffolding,
    fallback_chain: PlannerFallbackChain,
}

impl BranchingPlanner {
    pub async fn plan(&self, goal: &str, context: &PlanContext) -> PlanResult {
        // 1. Build WorkingSet (deterministic, no LLM)
        let ws = WorkingSet::build(
            goal,
            &context.world_model,
            &context.evidence,
            &context.constraints,
            2048,
        );

        // 2. Check failure patterns
        let failure_warning = {
            let analyzer = self.failure_analyzer.read().await;
            analyzer.check_goal(goal)
        };

        // 3. Generate 3 structured paths via LLM (with fallback)
        let paths = self.generate_structured_paths(goal, &ws, failure_warning.as_ref()).await;

        // 4. Score each path against SelfModel (Beta posterior)
        let self_model = self.self_model.read().await;
        let scored_paths: Vec<(StructuredPath, f32)> = paths.iter()
            .map(|p| (p.clone(), self_model.score_path(p)))
            .collect();

        // 5. Select winner (highest score, considering risk)
        let winner = self.select_winner(&scored_paths);

        PlanResult::Structured(winner)
    }

    async fn generate_structured_paths(
        &self,
        goal: &str,
        ws: &WorkingSet,
        failure_warning: Option<&FailurePattern>,
    ) -> Vec<StructuredPath> {
        // Try local 7B first
        match self.llm_backend.complete(&self.scaffolding.build_prompt(goal, ws, failure_warning)).await {
            Ok(response) => {
                match self.parse_structured_paths(&response) {
                    paths if paths.len() == 3 => paths,
                    _ => {
                        // Parsing failed — try fallback
                        self.fallback_chain.plan(goal, ws).await
                    }
                }
            }
            Err(_) => {
                // Local 7B unavailable — try fallback
                self.fallback_chain.plan(goal, ws).await
            }
        }
    }

    fn select_winner(&self, scored_paths: &[(StructuredPath, f32)]) -> StructuredPath {
        // Prefer Path B (minimal-risk) if its score is within 10% of Path A
        // This avoids unnecessary diagnostic steps when a fix is straightforward
        let path_a = scored_paths.iter().find(|(p, _)| p.risk_level == RiskLevel::Green);
        let path_b = scored_paths.iter().find(|(p, _)| p.risk_level == RiskLevel::Yellow);

        if let (Some((a, a_score)), Some((b, b_score))) = (path_a, path_b) {
            if b_score >= a_score * 0.9 {
                return b.clone(); // Path B is good enough — skip diagnosis
            }
        }

        // Otherwise, pick highest score
        scored_paths.iter()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(p, _)| p.clone())
            .unwrap_or(scored_paths[0].0.clone())
    }
}
```

#### File: `crates/kria-core/src/agent/planner_v2/scaffolding.rs` (NEW)

```rust
//! Chain-of-Thought scaffolding for the Planner.

pub struct ChainOfThoughtScaffolding;

impl ChainOfThoughtScaffolding {
    pub fn build_prompt(
        &self,
        goal: &str,
        ws: &WorkingSet,
        failure_warning: Option<&FailurePattern>,
    ) -> String {
        let mut prompt = String::from(
            r#"You are KRIA's planning engine. You MUST generate exactly 3 plans using these templates:

SYSTEM STATE:
"#
        );

        prompt.push_str(&ws.to_prompt_context());

        if let Some(failure) = failure_warning {
            prompt.push_str(&format!(
                "\n\nWARNING: Similar goal failed before.\n\
                 Failed plan: {:?}\n\
                 Reason: {}\n\
                 Avoid repeating this pattern.\n",
                failure.failed_plan, failure.failure_reason
            ));
        }

        prompt.push_str(r#"

Generate exactly 3 plans:

PATH A — DIAGNOSE-FIRST (read-only, gather information):
  Commands: [{"binary": "...", "args": ["..."], "target": "..."}]
  Predicted outcome: [what you'll learn]
  Risk: None (read-only)

PATH B — MINIMAL-RISK FIX (reversible changes):
  Commands: [{"binary": "...", "args": ["..."], "target": "..."}]
  Predicted outcome: [what will change]
  Risk: Low (reversible)

PATH C — AGGRESSIVE FIX (may be hard to reverse):
  Commands: [{"binary": "...", "args": ["..."], "target": "..."}]
  Predicted outcome: [what will change]
  Risk: High (potentially irreversible)

SELECT: [A/B/C] because [reasoning based on risk and confidence]

IMPORTANT: All commands MUST be structured JSON: {"binary": "...", "args": [...], "target": "..."}
Do NOT use shell syntax. Each command is a separate binary invocation."#
        );

        prompt
    }
}
```

#### File: `crates/kria-core/src/agent/planner_v2/executor.rs` (NEW)

```rust
//! Plan execution dispatcher with reflection and replanning.

pub struct PlanExecutor {
    executor: Arc<SubprocessExecutor>,
    goal_verifier: Arc<GoalVerifier>,
    max_steps: usize,
    max_replans: usize,
}

impl PlanExecutor {
    pub async fn execute(
        &self,
        plan: &StructuredPath,
        context: &ExecutionContext,
    ) -> ExecutionResult {
        let mut results = Vec::new();
        let mut replan_count = 0;

        for step in &plan.steps {
            // 1. Execute step via SubprocessExecutor (policy-gated)
            let result = self.executor.execute(step.command.clone()).await;
            results.push(result.clone());

            // 2. Verify goal with deterministic checks (NOT LLM)
            let verification = self.goal_verifier.verify(&plan.goal, &results);

            match verification {
                GoalVerification::Achieved => {
                    return ExecutionResult::Success(results);
                }
                GoalVerification::Failed { reason } => {
                    if replan_count < self.max_replans {
                        replan_count += 1;
                        // Replan with error context
                        return ExecutionResult::ReplanNeeded {
                            reason,
                            partial: results,
                        };
                    } else {
                        return ExecutionResult::Failed {
                            reason,
                            partial: results,
                        };
                    }
                }
                GoalVerification::Continue => {
                    // Move to next step
                }
            }
        }

        ExecutionResult::Partial(results)
    }
}
```

### C4: SelfModel — Beta Posterior (Week 12)

#### File: `crates/kria-core/src/agent/self_model/mod.rs` (NEW)

```rust
//! SelfModel — Capability awareness with Bayesian success rates.
//!
//! # Statistical Approach: Beta Posterior
//!
//! Previous approach: raw percentage (success / total)
//! Problem: 1 success = 100%, massive small-sample bias
//!
//! New approach: Beta(α, β) posterior estimation
//! - Prior: Beta(1, 1) — uniform prior, starts at 0.50
//! - Update: success → α += 1, failure → β += 1
//! - Posterior mean: α / (α + β)
//!
//! This naturally handles:
//! - Unknown tools start at 0.50 (neutral)
//! - 1 success → (1+1)/(1+2) = 0.67 (not 1.00)
//! - 10 successes, 0 failures → 0.92 (high confidence)
//! - 1 success, 1 failure → 0.50 (neutral)

pub mod tool_stats;
pub use tool_stats::{ToolStats, SelfModel};
```

#### File: `crates/kria-core/src/agent/self_model/tool_stats.rs` (NEW)

```rust
use std::collections::HashMap;
use std::time::Duration;
use chrono::{DateTime, Utc};

pub struct SelfModel {
    tool_stats: HashMap<String, ToolStats>,
    domain_accuracy: HashMap<String, f32>,
}

pub struct ToolStats {
    pub tool_name: String,
    /// Beta distribution alpha parameter (successes + prior)
    pub alpha: f32,
    /// Beta distribution beta parameter (failures + prior)
    pub beta: f32,
    pub total_calls: usize,
    pub avg_latency: Duration,
    pub last_used: DateTime<Utc>,
    pub known_failure_modes: Vec<String>,
}

impl ToolStats {
    pub fn new(tool_name: &str) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            alpha: 1.0,  // Uniform prior
            beta: 1.0,   // Uniform prior
            total_calls: 0,
            avg_latency: Duration::ZERO,
            last_used: Utc::now(),
            known_failure_modes: Vec::new(),
        }
    }

    /// Posterior mean: α / (α + β)
    pub fn success_rate(&self) -> f32 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Record an outcome. Beta update: success → α += 1, failure → β += 1
    pub fn record(&mut self, success: bool, latency: Duration) {
        if success {
            self.alpha += 1.0;
        } else {
            self.beta += 1.0;
        }
        self.total_calls += 1;
        // Exponential moving average for latency
        let alpha = 0.1;
        self.avg_latency = Duration::from_secs_f64(
            self.avg_latency.as_secs_f64() * (1.0 - alpha) + latency.as_secs_f64() * alpha
        );
        self.last_used = Utc::now();
    }

    /// Confidence interval width (narrower = more confident)
    pub fn confidence_width(&self) -> f32 {
        // Approximate 95% CI width for Beta distribution
        let n = self.alpha + self.beta - 2.0; // Effective sample size
        if n <= 0.0 {
            return 1.0; // Maximum uncertainty
        }
        let p = self.success_rate();
        2.0 * 1.96 * (p * (1.0 - p) / n).sqrt()
    }
}

impl SelfModel {
    pub fn new() -> Self {
        Self {
            tool_stats: HashMap::new(),
            domain_accuracy: HashMap::new(),
        }
    }

    /// Score a path using geometric mean of tool success rates.
    pub fn score_path(&self, path: &StructuredPath) -> f32 {
        let tool_scores: Vec<f32> = path.steps.iter()
            .map(|step| self.tool_stats.get(&step.tool_name)
                .map(|s| s.success_rate())
                .unwrap_or(0.5))  // Unknown tools get neutral Beta(1,1) prior
            .collect();
        if tool_scores.is_empty() {
            return 0.5;
        }
        // Geometric mean (path fails if any step fails)
        tool_scores.iter().product::<f32>().powf(1.0 / tool_scores.len() as f32)
    }

    /// Record an outcome for a tool.
    pub fn record_outcome(&mut self, tool_name: &str, success: bool, latency: Duration) {
        let stats = self.tool_stats.entry(tool_name.to_string())
            .or_insert_with(|| ToolStats::new(tool_name));
        stats.record(success, latency);
    }

    /// Get stats for a specific tool.
    pub fn get_stats(&self, tool_name: &str) -> Option<&ToolStats> {
        self.tool_stats.get(tool_name)
    }

    /// Persist to SQLite (called periodically and on shutdown).
    pub async fn persist(&self, store: &crate::memory::store::MemoryStore) -> Result<(), anyhow::Error> {
        // Serialize tool_stats to SQLite
        // Schema: tool_stats(tool_name TEXT PK, alpha REAL, beta REAL, total_calls INT, ...)
        Ok(())
    }

    /// Load from SQLite (called on startup).
    pub async fn load(store: &crate::memory::store::MemoryStore) -> Result<Self, anyhow::Error> {
        // Deserialize from SQLite
        Ok(Self::new())
    }
}
```

### C5: GoalVerifier — Deterministic Post-Conditions (Week 13)

#### File: `crates/kria-core/src/agent/goal_verifier.rs` (NEW)

```rust
//! Goal Verifier — Deterministic goal achievement checking.
//!
//! Instead of asking the LLM "did we achieve the goal?", we define
//! deterministic post-conditions that can be checked programmatically.
//!
//! For example:
//! - "Make my VM faster" → post-condition: CPU < 50% (checkable via top)
//! - "Fix nginx" → post-condition: systemctl status nginx = active
//! - "Install python3" → post-condition: which python3 returns 0
//!
//! The LLM suggests post-conditions during planning, and we verify
//! them after execution using the SubprocessExecutor.

pub struct GoalVerifier {
    executor: Arc<SubprocessExecutor>,
}

pub enum GoalVerification {
    Achieved,
    Failed { reason: String },
    Continue,
}

pub struct PostCondition {
    pub check_command: StructuredCommand,  // Read-only command to verify
    pub expected_exit_code: i32,
    pub expected_output_contains: Option<String>,
    pub description: String,
}

impl GoalVerifier {
    /// Verify goal achievement using post-conditions.
    pub async fn verify(&self, goal: &str, results: &[ExecutionResult]) -> GoalVerification {
        // Extract post-conditions from goal
        let post_conditions = self.infer_post_conditions(goal);

        if post_conditions.is_empty() {
            // No post-conditions — ask LLM (fallback)
            return GoalVerification::Continue;
        }

        // Check each post-condition
        for condition in &post_conditions {
            let result = self.executor.execute(condition.check_command.clone()).await;

            if result.exit_code != condition.expected_exit_code {
                return GoalVerification::Failed {
                    reason: format!("Post-condition '{}' failed: exit code {} (expected {})",
                        condition.description, result.exit_code, condition.expected_exit_code),
                };
            }

            if let Some(ref expected) = condition.expected_output_contains {
                if !result.stdout.contains(expected) && !result.stderr.contains(expected) {
                    return GoalVerification::Failed {
                        reason: format!("Post-condition '{}' failed: output doesn't contain '{}'",
                            condition.description, expected),
                    };
                }
            }
        }

        GoalVerification::Achieved
    }

    fn infer_post_conditions(&self, goal: &str) -> Vec<PostCondition> {
        let goal_lower = goal.to_lowercase();
        let mut conditions = Vec::new();

        // Service-related goals
        if goal_lower.contains("nginx") || goal_lower.contains("start") {
            conditions.push(PostCondition {
                check_command: StructuredCommand {
                    binary: "systemctl".into(),
                    args: vec!["is-active".into(), "nginx".into()],
                    target: "local".into(),
                    timeout_secs: 5,
                    working_dir: None,
                    env_vars: None,
                },
                expected_exit_code: 0,
                expected_output_contains: Some("active".into()),
                description: "nginx service is active".into(),
            });
        }

        // Performance goals
        if goal_lower.contains("faster") || goal_lower.contains("cpu") || goal_lower.contains("performance") {
            conditions.push(PostCondition {
                check_command: StructuredCommand {
                    binary: "top".into(),
                    args: vec!["-bn1".into(), "-w512".into()],
                    target: "local".into(),
                    timeout_secs: 10,
                    working_dir: None,
                    env_vars: None,
                },
                expected_exit_code: 0,
                expected_output_contains: None, // Parse CPU% from output
                description: "CPU usage checkable".into(),
            });
        }

        conditions
    }
}
```

### C6: PlannerFallbackChain (Week 13)

#### File: `crates/kria-core/src/agent/planner_fallback.rs` (NEW)

```rust
//! Planner Fallback Chain — Handles 7B model unavailability.
//!
//! When the local 7B model is unavailable (VRAM occupied by Vision,
//! model loading failed, etc.), we fall back to:
//! 1. Cloud Gemini Flash (free tier, ~500ms)
//! 2. Simplified heuristic planner (deterministic, 0ms)
//!
//! This ensures KRIA NEVER fails to plan — it just plans differently.

pub struct PlannerFallbackChain {
    local_planner: Option<Arc<BranchingPlanner>>,
    cloud_client: Option<reqwest::Client>,
    cloud_api_key: Option<String>,
    heuristic_planner: HeuristicPlanner,
}

impl PlannerFallbackChain {
    pub async fn plan(&self, goal: &str, ws: &WorkingSet) -> Vec<StructuredPath> {
        // Try local 7B
        if let Some(ref local) = self.local_planner {
            match local.plan(goal, &PlanContext::from_working_set(ws)).await {
                PlanResult::Structured(paths) if paths.len() == 3 => return paths,
                _ => {} // Fall through
            }
        }

        // Try cloud Gemini Flash
        if let (Some(ref client), Some(ref key)) = (&self.cloud_client, &self.cloud_api_key) {
            match self.plan_via_cloud(client, key, goal, ws).await {
                Ok(paths) if paths.len() == 3 => return paths,
                _ => {} // Fall through
            }
        }

        // Heuristic planner (deterministic, no LLM)
        self.heuristic_planner.plan(goal, ws)
    }

    async fn plan_via_cloud(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        goal: &str,
        ws: &WorkingSet,
    ) -> Result<Vec<StructuredPath>, anyhow::Error> {
        // Call Gemini Flash API with structured branching prompt
        // Parse response into 3 StructuredPaths
        todo!("Implement cloud fallback")
    }
}

/// Deterministic heuristic planner — no LLM required.
/// Used as last resort when both local and cloud models are unavailable.
pub struct HeuristicPlanner;

impl HeuristicPlanner {
    pub fn plan(&self, goal: &str, ws: &WorkingSet) -> Vec<StructuredPath> {
        // Generate basic diagnostic + fix plan based on goal keywords
        // This is intentionally simple — it's a fallback, not the primary planner
        let goal_lower = goal.to_lowercase();

        // PATH A: Always start with diagnostics
        let path_a = StructuredPath {
            risk_level: RiskLevel::Green,
            steps: vec![
                PlanStep {
                    step_number: 1,
                    tool_name: "execute_command".into(),
                    description: "Check system status".into(),
                    parameters: serde_json::json!({"binary": "top", "args": ["-bn1"], "target": "local"}),
                    depends_on: vec![],
                    error_handling: "continue".into(),
                },
                PlanStep {
                    step_number: 2,
                    tool_name: "execute_command".into(),
                    description: "Check disk usage".into(),
                    parameters: serde_json::json!({"binary": "df", "args": ["-h"], "target": "local"}),
                    depends_on: vec![],
                    error_handling: "continue".into(),
                },
            ],
            confidence: 0.7,
        };

        // PATH B: Common fix patterns
        let path_b = StructuredPath {
            risk_level: RiskLevel::Yellow,
            steps: vec![
                PlanStep {
                    step_number: 1,
                    tool_name: "execute_command".into(),
                    description: "Restart common services".into(),
                    parameters: serde_json::json!({"binary": "systemctl", "args": ["restart", "nginx"], "target": "local"}),
                    depends_on: vec![],
                    error_handling: "abort".into(),
                },
            ],
            confidence: 0.5,
        };

        // PATH C: Aggressive
        let path_c = StructuredPath {
            risk_level: RiskLevel::Red,
            steps: vec![
                PlanStep {
                    step_number: 1,
                    tool_name: "execute_command".into(),
                    description: "Force kill and restart".into(),
                    parameters: serde_json::json!({"binary": "systemctl", "args": ["kill", "nginx"], "target": "local"}),
                    depends_on: vec![],
                    error_handling: "abort".into(),
                },
            ],
            confidence: 0.3,
        };

        vec![path_a, path_b, path_c]
    }
}
```

### Tests: `crates/kria-core/tests/phase8_uncertainty_tests.rs` (NEW)

| Test ID | Test Name | Expected Result |
|---------|-----------|----------------|
| UE01 | `high_confidence_proceeds_to_plan` | Confidence ≥0.8 → Plan action |
| UE02 | `medium_confidence_gathers_evidence` | Confidence 0.6-0.8 → GatherEvidence |
| UE03 | `low_confidence_asks_user` | Confidence 0.3-0.6 → AskUser |
| UE04 | `very_low_confidence_refuses` | Confidence <0.3 → Refuse |
| UE05 | `evidence_gathering_returns_read_only` | All diagnostic commands are read-only |
| UE06 | `belief_graph_stores_facts` | Facts stored with confidence and source |
| UE07 | `belief_graph_updates_on_evidence` | New evidence updates confidence via Bayesian update |
| UE08 | `belief_graph_decays_over_time` | Old facts lose confidence |
| UE09 | `calibration_adjusts_thresholds` | Beta posterior recalibrates thresholds |
| UE10 | `vm_diagnostic_commands_generated` | VM-specific diagnostics generated |
| UE11 | `local_diagnostic_commands_generated` | Local diagnostics generated |
| UE12 | `confidence_geometric_mean` | Overall confidence is geometric mean of facts |

### Tests: `crates/kria-core/tests/phase8_working_set_tests.rs` (NEW)

| Test ID | Test Name | Expected Result |
|---------|-----------|----------------|
| WS01 | `structured_extractor_preserves_error_codes` | Error codes preserved verbatim |
| WS02 | `structured_extractor_preserves_file_paths` | File paths preserved verbatim |
| WS03 | `structured_extractor_preserves_ips` | IP addresses preserved verbatim |
| WS04 | `structured_extractor_preserves_numeric_values` | Numeric values preserved verbatim |
| WS05 | `raw_snippet_truncated_by_lines` | Raw output truncated at line limit |
| WS06 | `max_tokens_enforced` | WorkingSet never exceeds max_tokens |
| WS07 | `priority_based_truncation` | Raw snippets removed before structured fields |
| WS08 | `to_prompt_context_format` | Output is valid prompt string |
| WS09 | `empty_working_set` | Handles missing data gracefully |
| WS10 | `no_llm_calls_during_extraction` | Extraction is deterministic (no LLM) |

### Tests: `crates/kria-core/tests/phase8_planner_tests.rs` (NEW)

| Test ID | Test Name | Expected Result |
|---------|-----------|----------------|
| PL01 | `generates_three_paths` | Always produces exactly 3 paths |
| PL02 | `path_a_is_read_only` | Diagnose-First path contains only read commands |
| PL03 | `path_b_is_reversible` | Minimal-Risk path contains reversible changes |
| PL04 | `path_c_is_aggressive` | Aggressive path may contain destructive changes |
| PL05 | `self_model_scores_paths_with_beta_posterior` | Paths scored using Beta(α,β) posterior mean |
| PL06 | `failure_pattern_checked` | Known failures avoided |
| PL07 | `working_set_used` | Planner reads WorkingSet, not full history |
| PL08 | `scaffolding_prompt_format` | Prompt follows structured branching template |
| PL09 | `executor_observes_results` | Each step result is observed |
| PL10 | `deterministic_goal_verification` | Goal verified by post-conditions, not LLM |
| PL11 | `goal_achieved_stops_execution` | Execution stops when goal achieved |
| PL12 | `step_timeout_enforced` | Individual steps timeout properly |
| PL13 | `fallback_to_cloud` | Falls back to cloud when local 7B unavailable |
| PL14 | `fallback_to_heuristic` | Falls back to heuristic when cloud also unavailable |
| PL15 | `unknown_tool_neutral_score` | Unknown tools score 0.5 (Beta 1,1 prior) |

### Tests: `crates/kria-core/tests/phase8_self_model_tests.rs` (NEW)

| Test ID | Test Name | Expected Result |
|---------|-----------|----------------|
| SM01 | `new_tool_starts_at_0_5` | Beta(1,1) prior = 0.50 |
| SM02 | `one_success_gives_0_67` | (1+1)/(1+2) = 0.67, NOT 1.00 |
| SM03 | `ten_successes_gives_0_92` | (10+1)/(10+2) = 0.92 |
| SM04 | `one_failure_gives_0_33` | (1)/(1+2) = 0.33 |
| SM05 | `score_path_geometric_mean` | Path score is geometric mean of tool rates |
| SM06 | `confidence_interval_width` | CI width narrows with more data |
| SM07 | `persistence_across_restarts` | Beta parameters survive process restart |
| SM08 | `failure_modes_recorded` | Known failure modes stored |
| SM09 | `latency_ema_updated` | Latency tracked via exponential moving average |

---

## Phase D: Memory, Skill Learning + Quarantine

**Duration:** Weeks 14–18  
**Goal:** True self-improvement through calibrated compilation with safety gates  
**Risk:** Low (additive, async)

### D1: World Model (Weeks 14-15)

#### File: `crates/kria-core/src/agent/world_model/mod.rs` (NEW)

```rust
//! World Model — Persistent facts about the user's systems.
//!
//! Stores facts with confidence scores, evidence chains, and staleness tracking.
//! Uses SQLite for persistence (same DB as MemoryStore).

pub struct WorldModel {
    store: Arc<MemoryStore>,
    cache: RwLock<HashMap<String, WorldFact>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorldFact {
    pub subject: String,         // "VM1"
    pub predicate: String,       // "runs"
    pub object: String,          // "Ubuntu 24.04"
    pub confidence: f32,         // Beta posterior
    pub evidence: Vec<String>,   // ["ssh uname -a: Ubuntu 24.04"]
    pub source: FactSource,
    pub last_verified: DateTime<Utc>,
    pub staleness_hours: f32,    // Hours until fact is considered stale
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum FactSource {
    Detected,    // System command output
    UserStated,  // User told us
    Inferred,    // LLM reasoned about it
    Compiled,    // Skill compiler output
}
```

### D2: Failure Analyzer (Week 16)

#### File: `crates/kria-core/src/agent/failure_analyzer/mod.rs` (NEW)

```rust
//! Failure Analyzer — Learn from mistakes.
//!
//! When a plan fails, the analyzer extracts:
//! - What failed (exact command and error)
//! - Why it failed (root cause from stderr/exit code)
//! - What would have worked (alternative from World Model)
//!
//! Before executing a new plan, it checks against known failure patterns.

pub struct FailureAnalyzer {
    patterns: Vec<FailurePattern>,
    store: Arc<MemoryStore>,
}

#[derive(Debug, Clone)]
pub struct FailurePattern {
    pub trigger: String,                    // "make VM faster"
    pub failed_commands: Vec<StructuredCommand>,
    pub failure_reason: String,             // "nginx config was wrong"
    pub error_codes: Vec<String>,           // ["ECONNREFUSED", "exit 1"]
    pub suggested_alternative: Option<Vec<StructuredCommand>>,
    pub confidence: f32,                    // How reliable this pattern is
    pub occurrences: usize,                 // How many times we've seen this
}
```

### D3: Skill Compiler with QuarantineRegistry (Weeks 17-18)

#### File: `crates/kria-core/src/agent/skill_compiler/mod.rs` (NEW)

```rust
//! Skill Compiler — Calibrated compilation with N=3 gating.
//!
//! A plan is ONLY compiled into a reusable tool schema after it has
//! succeeded in 3 slightly varied contexts.
//!
//! # Quarantine Requirement
//! Compiled skills go to QuarantineRegistry first.
//! - Read-only skills: auto-promote after N=3
//! - Write skills: require HITL approval after N=3
//! - Destructive skills: require HITL approval + PIN after N=3

pub mod pattern_extractor;
pub mod graph_parameterizer;
pub mod trigger_generator;

pub use pattern_extractor::PatternExtractor;
pub use trigger_generator::TriggerGenerator;
```

#### File: `crates/kria-core/src/tools/quarantine.rs` (NEW)

```rust
//! QuarantineRegistry — Safety gate for dynamically generated tools.
//!
//! New tools (compiled skills, dynamically discovered APIs) go here first.
//! They are NOT available for LLM use until promoted to the active registry.
//!
//! Promotion rules:
//! - Green risk (read-only): auto-promote after N=3 successes
//! - Yellow risk (write): require HITL approval after N=3 successes
//! - Red risk (destructive): require HITL approval + PIN after N=3
//! - Black risk: never promoted

pub struct QuarantineRegistry {
    quarantined: RwLock<HashMap<String, QuarantinedTool>>,
    active: Arc<ToolRegistry>,
    hitl: Arc<HitlGateway>,
    circuit_breaker: CircuitBreaker,
}

pub struct QuarantinedTool {
    pub def: ToolDef,
    pub handler: Arc<dyn ToolHandler>,
    pub source: ToolSource,
    pub success_count: usize,
    pub failure_count: usize,
    pub risk_level: RiskLevel,
    pub created_at: DateTime<Utc>,
    pub status: QuarantineStatus,
}

pub enum QuarantineStatus {
    /// Awaiting enough successes for promotion evaluation
    Testing,
    /// Ready for HITL approval (yellow/red risk)
    PendingApproval,
    /// Promoted to active registry
    Active,
    /// Failed circuit breaker (3 consecutive failures)
    Disabled,
    /// User rejected promotion
    Rejected,
}

pub enum ToolSource {
    /// Compiled from successful plan
    SkillCompiler,
    /// Discovered from API/CLI
    DynamicDiscovery,
    /// MCP server provided
    McpServer,
}

/// Circuit breaker for compiled skills.
/// If a skill fails 3 consecutive times, it's automatically disabled.
pub struct CircuitBreaker {
    failure_threshold: usize,  // 3
    reset_timeout: Duration,   // 24 hours
}

impl QuarantineRegistry {
    /// Add a new tool to quarantine.
    pub fn quarantine(&self, def: ToolDef, handler: Arc<dyn ToolHandler>, source: ToolSource) {
        let risk = def.default_tier;
        let mut quarantined = self.quarantined.write().unwrap();
        quarantined.insert(def.name.clone(), QuarantinedTool {
            def,
            handler,
            source,
            success_count: 0,
            failure_count: 0,
            risk_level: risk,
            created_at: Utc::now(),
            status: QuarantineStatus::Testing,
        });
    }

    /// Record a success for a quarantined tool.
    pub fn record_success(&self, tool_name: &str) {
        let mut quarantined = self.quarantined.write().unwrap();
        if let Some(tool) = quarantined.get_mut(tool_name) {
            tool.success_count += 1;
            tool.failure_count = 0; // Reset failure streak

            // Check if ready for promotion
            if tool.success_count >= 3 {
                match tool.risk_level {
                    RiskLevel::Green => {
                        // Auto-promote read-only tools
                        self.promote_to_active(tool_name);
                    }
                    RiskLevel::Yellow | RiskLevel::Red => {
                        // Require HITL approval
                        tool.status = QuarantineStatus::PendingApproval;
                    }
                    RiskLevel::Black => {
                        // Never promote
                        tool.status = QuarantineStatus::Rejected;
                    }
                }
            }
        }
    }

    /// Record a failure for a quarantined tool.
    pub fn record_failure(&self, tool_name: &str) {
        let mut quarantined = self.quarantined.write().unwrap();
        if let Some(tool) = quarantined.get_mut(tool_name) {
            tool.failure_count += 1;
            tool.success_count = 0; // Reset success streak

            // Circuit breaker: 3 consecutive failures → disable
            if tool.failure_count >= 3 {
                tool.status = QuarantineStatus::Disabled;
            }
        }
    }

    /// Execute a HITL approval flow for pending tools.
    pub async fn request_approval(&self, tool_name: &str) -> Result<bool, anyhow::Error> {
        let quarantined = self.quarantined.read().unwrap();
        if let Some(tool) = quarantined.get(tool_name) {
            if let QuarantineStatus::PendingApproval = tool.status {
                let approved = self.hitl.request_approval_with_id(
                    uuid::Uuid::new_v4().to_string(),
                    "promote_skill",
                    serde_json::json!({"tool_name": tool_name, "risk": tool.risk_level}),
                    tool.risk_level,
                    format!("Promote compiled skill '{}' ({} successes, risk: {:?})",
                        tool_name, tool.success_count, tool.risk_level),
                    true,
                ).await?;

                if approved {
                    drop(quarantined);
                    self.promote_to_active(tool_name);
                }
                return Ok(approved);
            }
        }
        Ok(false)
    }

    fn promote_to_active(&self, tool_name: &str) {
        let mut quarantined = self.quarantined.write().unwrap();
        if let Some(mut tool) = quarantined.remove(tool_name) {
            tool.status = QuarantineStatus::Active;
            self.active.register(tool.def, tool.handler);
        }
    }
}
```

### Tests: `crates/kria-core/tests/phase8_skill_compiler_tests.rs` (NEW)

| Test ID | Test Name | Expected Result |
|---------|-----------|----------------|
| SC01 | `extract_ip_variable` | IP addresses become {target_host} |
| SC02 | `extract_service_variable` | Service names become {service_name} |
| SC03 | `parameterize_graph` | Hardcoded values replaced with params |
| SC04 | `generate_triggers` | Trigger patterns generated from goal |
| SC05 | `n3_gating_not_compiled_before_threshold` | Not compiled until 3 successes |
| SC06 | `n3_gating_compiled_at_threshold` | Compiled after 3 varied successes |
| SC07 | `failure_resets_counter` | Counter resets on failure |
| SC08 | `confidence_decay` | Unused skills lose confidence over time |
| SC09 | `skill_matches_intent` | Compiled skill matched by router |
| SC10 | `skill_execution_faster` | Compiled skill executes faster than LLM |

### Tests: `crates/kria-core/tests/phase8_quarantine_tests.rs` (NEW)

| Test ID | Test Name | Expected Result |
|---------|-----------|----------------|
| QT01 | `green_skill_auto_promotes` | Read-only skill auto-promotes after N=3 |
| QT02 | `yellow_skill_requires_hitl` | Write skill requires HITL after N=3 |
| QT03 | `red_skill_requires_hitl_pin` | Destructive skill requires HITL+PIN after N=3 |
| QT04 | `black_skill_never_promotes` | Black-risk skills always rejected |
| QT05 | `circuit_breaker_disables` | 3 consecutive failures → disabled |
| QT06 | `failure_resets_success_streak` | Failure resets success counter |
| QT07 | `success_resets_failure_streak` | Success resets failure counter |
| QT08 | `quarantined_tool_not_in_registry` | Quarantined tools not available to LLM |
| QT09 | `promoted_tool_in_registry` | Promoted tools available to LLM |
| QT10 | `hitl_approval_workflow` | HITL approval flow works correctly |

### Tests: `crates/kria-core/tests/phase8_failure_analyzer_tests.rs` (NEW)

| Test ID | Test Name | Expected Result |
|---------|-----------|----------------|
| FA01 | `extract_failure_pattern` | Pattern extracted from failed plan |
| FA02 | `root_cause_identified` | Root cause identified from error output |
| FA03 | `alternative_suggested` | Alternative plan suggested |
| FA04 | `check_plan_against_failures` | Known failures detected before execution |
| FA05 | `failure_pattern_persistence` | Patterns survive process restart |
| FA06 | `error_codes_preserved` | Exact error codes preserved in pattern |

### Tests: `crates/kria-core/tests/phase8_world_model_tests.rs` (NEW)

| Test ID | Test Name | Expected Result |
|---------|-----------|----------------|
| WM01 | `store_system_fact` | Fact stored in SQLite |
| WM02 | `retrieve_system_fact` | Fact retrieved correctly |
| WM03 | `fact_confidence_tracking` | Confidence tracked and updated |
| WM04 | `fact_source_tracking` | Source recorded (detected/user/inferred) |
| WM05 | `fact_staleness_detection` | Old facts flagged as stale |
| WM06 | `world_model_persistence` | Facts survive process restart |
| WM07 | `constraints_extraction` | Active constraints extracted |
| WM08 | `vector_search_over_facts` | Semantic search over facts |

---

## Phase E: Event-Driven Perception + Curiosity

**Duration:** Weeks 19–22  
**Goal:** Replace polling with real-time event hooks, add background curiosity with resource budgets  
**Risk:** Low (additive, background-only)

### E1: Event-Driven Monitoring (Weeks 19-20)

#### File: `crates/kria-core/src/agent/perception/mod.rs` (NEW)

```rust
//! Event-driven perception layer.
//!
//! Hooks into kernel-level event systems for sub-millisecond awareness:
//! - inotify: filesystem changes (file create/modify/delete)
//! - dbus: system service events (service start/stop/fail)
//! - netlink: network state changes (interface up/down, route changes)
//!
//! Events are broadcast to subscribers (CuriosityLoop, ProactiveScheduler)
//! via tokio::sync::broadcast channels.
```

### E2: CuriosityLoop with BudgetGuard (Weeks 21-22)

#### File: `crates/kria-core/src/agent/curiosity/mod.rs` (NEW)

```rust
//! Curiosity Loop — Background diagnostic engine with resource budgets.
//!
//! When the system is idle, investigates anomalies detected by the
//! perception layer. Only runs read-only diagnostics via PolicyGate.
//!
//! # Resource Budget (BudgetGuard)
//! - Max 10% CPU usage
//! - 0 VRAM (never requests GPU lease)
//! - Yields immediately when any foreground task arrives
//! - Max 10 diagnostic commands per cycle
//! - 60-second cooldown between cycles

pub struct CuriosityLoop {
    perception_rx: broadcast::Receiver<PerceptionEvent>,
    world_model: Arc<RwLock<WorldModel>>,
    evidence_gatherer: Arc<EvidenceGatherer>,
    budget: BudgetGuard,
    executive_tx: mpsc::UnboundedSender<TaskRequest>,
    cooldown: Duration,
    max_commands_per_cycle: usize,
}
```

### Tests: `crates/kria-core/tests/phase8_perception_tests.rs` (NEW)

| Test ID | Test Name | Expected Result |
|---------|-----------|----------------|
| EP01 | `file_change_detected` | inotify event captured |
| EP02 | `service_start_detected` | dbus event captured |
| EP03 | `network_change_detected` | netlink event captured |
| EP04 | `event_latency_sub_ms` | Events delivered in <1ms |
| EP05 | `no_polling_overhead` | CPU usage near zero when idle |

### Tests: `crates/kria-core/tests/phase8_curiosity_tests.rs` (NEW)

| Test ID | Test Name | Expected Result |
|---------|-----------|----------------|
| CL01 | `novelty_detected` | New service flagged as novelty |
| CL02 | `investigation_read_only` | All diagnostics are read-only (PolicyGate) |
| CL03 | `world_model_updated` | Findings stored in World Model |
| CL04 | `budget_guard_enforced` | Never exceeds 10% CPU |
| CL05 | `yields_on_foreground` | Stops immediately when voice command arrives |
| CL06 | `cooldown_respected` | 60s between cycles |
| CL07 | `max_commands_per_cycle` | Never exceeds 10 commands per cycle |

---

## Phase F: Browser Agent + Dynamic Tools

**Duration:** Weeks 23–24  
**Goal:** Web automation + self-extending capabilities  
**Risk:** Low (ephemeral, isolated)

### F1: Browser Agent (Week 23)

#### File: `crates/kria-core/src/tools/browser_agent.rs` (NEW)

```rust
//! Browser agent — LLM-controlled web automation.
//!
//! Uses Browser-Use (Python sidecar) for complex web tasks.
//! Ephemeral sandbox only — Python process dies after task.
//!
//! # Safety
//! - Runs in Docker container (isolated)
//! - No access to host browser credentials
//! - Browsing history logged to audit trail
//! - User approval required for form submissions
```

### F2: Dynamic Tool Generation (Week 24)

#### File: `crates/kria-core/src/tools/dynamic_gen.rs` (NEW)

```rust
//! Dynamic tool generation from API docs and CLI help text.
//!
//! # Schema Validation (NEW)
//! Generated ToolDef must pass:
//! 1. JSON Schema validation (valid parameter types)
//! 2. Sandbox dry-run (command executes without error in Docker)
//! 3. QuarantineRegistry entry (not auto-promoted)
//!
//! This prevents malformed tool definitions from reaching the active registry.
```

### F3: Prompt Optimizer (Week 24)

#### File: `crates/kria-core/src/agent/prompt_optimizer/mod.rs` (NEW)

```rust
//! Prompt Optimizer — DSPy-style prompt improvement.
//!
//! Tracks which prompt variants produce the best outcomes
//! for each task type. Gradually shifts the system prompt
//! toward the best-performing variants.
```

---

## 10. Frontend Implementation

### 10.1 Executive Controller Dashboard (Week 4)

**File:** `ui/src/components/ExecutiveDashboard.tsx` (NEW)

```tsx
// Shows:
// - Active task queue (prioritized list)
// - GPU lease status (who holds it, time remaining)
// - Preemption events (what was preempted for what)
// - Background task status (CuriosityLoop, Skill Compiler)
// - Budget usage (CPU%, memory)
```

### 10.2 Policy Gate Log Viewer (Week 6)

**File:** `ui/src/components/PolicyGateLog.tsx` (NEW)

```tsx
// Shows:
// - Recent command evaluations
// - Policy decisions (AutoApproved, Blocked, Quarantined)
// - Risk levels for each command
// - HITL approval requests
// - Filter by risk level, time range
```

### 10.3 Quarantine Approval Queue (Week 17)

**File:** `ui/src/components/QuarantineQueue.tsx` (NEW)

```tsx
// Shows:
// - Skills awaiting HITL approval
// - Skill details (name, risk level, success count, source)
// - Approve/Reject buttons
// - Skill execution history
// - Circuit breaker status
```

### 10.4 Intelligence Dashboard (Week 13)

**File:** `ui/src/components/IntelligenceDashboard.tsx` (NEW)

```tsx
// Shows:
// - Uncertainty Engine status (confidence scores, belief graph)
// - WorkingSet contents (structured evidence)
// - SelfModel tool success rates (Beta posteriors)
// - Skill Compiler progress (quarantined vs active)
// - Curiosity Loop findings
// - Goal verification results
```

### 10.5 Plan Visualization (Week 13)

**File:** `ui/src/components/PlanVisualization.tsx` (NEW)

```tsx
// Shows the 3 structured paths:
// - Path A (Diagnose-First) — green
// - Path B (Minimal-Risk) — yellow
// - Path C (Aggressive) — red
// - Winner highlighted with SelfModel Beta posterior score
// - Post-condition verification results
```

### 10.6 World Model Viewer (Week 15)

**File:** `ui/src/components/WorldModelViewer.tsx` (NEW)

```tsx
// Shows persistent facts:
// - System facts (VMs, services, hardware)
// - User facts (preferences, habits)
// - Fact confidence scores (Beta posteriors)
// - Fact staleness indicators
// - Evidence chains
```

### 10.7 Frontend Store Updates

**File:** `ui/src/stores/app.ts` (MODIFY)

```typescript
// Add new state:
const [executiveState, setExecutiveState] = createSignal<ExecutiveState>({
  activeTasks: [],
  queuedTasks: [],
  gpuLeaseHolder: null,
  gpuLeaseTimeRemaining: 0,
  backgroundTaskCount: 0,
});

const [policyGateLog, setPolicyGateLog] = createSignal<PolicyGateEntry[]>([]);

const [quarantineState, setQuarantineState] = createSignal<QuarantineState>({
  pendingApproval: [],
  testing: [],
  disabled: [],
});

const [intelligenceState, setIntelligenceState] = createSignal<IntelligenceState>({
  uncertaintyConfidence: 0,
  workingSetTokens: 0,
  selfModelToolCount: 0,
  compiledSkillCount: 0,
  quarantinedSkillCount: 0,
  curiosityFindings: 0,
});

// Add new event listeners:
listen("executive:task_started", (event) => { ... });
listen("executive:task_completed", (event) => { ... });
listen("executive:preemption", (event) => { ... });
listen("policy_gate:evaluation", (event) => { ... });
listen("quarantine:pending_approval", (event) => { ... });
listen("intelligence:uncertainty", (event) => { ... });
listen("intelligence:plan", (event) => { ... });
listen("intelligence:skill_compiled", (event) => { ... });
```

---

## 11. Server API Changes

### 11.1 New Endpoints

**File:** `crates/kria-server/src/routes.rs` (MODIFY)

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/executive/tasks` | GET | Active + queued tasks |
| `/api/executive/tasks/{id}/cancel` | POST | Cancel a task |
| `/api/executive/gpu-status` | GET | GPU lease status |
| `/api/policy-gate/log` | GET | Recent policy evaluations |
| `/api/policy-gate/rules` | GET/PUT | View/update policy rules |
| `/api/quarantine/pending` | GET | Skills awaiting approval |
| `/api/quarantine/{id}/approve` | POST | Approve quarantined skill |
| `/api/quarantine/{id}/reject` | POST | Reject quarantined skill |
| `/api/intelligence/status` | GET | Uncertainty engine, SelfModel stats |
| `/api/intelligence/working-set` | GET | Current WorkingSet contents |
| `/api/intelligence/world-model` | GET/PUT | World Model facts |
| `/api/intelligence/skills` | GET | Compiled skills list |
| `/api/intelligence/plan` | POST | Request structured branching plan |
| `/api/intelligence/feedback` | POST | Submit routing feedback |

### 11.2 New Event Streams

| Event | Purpose |
|-------|---------|
| `executive:task_started` | Task execution began |
| `executive:task_completed` | Task finished (success/failure) |
| `executive:preemption` | Background task preempted |
| `executive:gpu_acquired` | GPU lease acquired |
| `executive:gpu_released` | GPU lease released |
| `policy_gate:evaluation` | Command evaluated by policy |
| `policy_gate:blocked` | Command blocked |
| `policy_gate:quarantined` | Unknown binary quarantined |
| `quarantine:pending_approval` | Skill awaiting HITL |
| `quarantine:promoted` | Skill promoted to active |
| `quarantine:disabled` | Skill disabled by circuit breaker |
| `intelligence:uncertainty` | Uncertainty state changes |
| `intelligence:plan` | Plan generation events |
| `intelligence:execution` | Step execution progress |
| `intelligence:reflection` | Goal verification outcomes |
| `intelligence:skill_compiled` | New skill compiled |
| `intelligence:curiosity` | Curiosity findings |

---

## 12. Integration Testing Strategy

### 12.1 Test Categories

| Category | Tests | Phase | Coverage |
|----------|-------|-------|----------|
| Unit Tests | ~180 | All phases | Each module independently |
| Integration Tests | ~40 | All phases | Module interactions |
| End-to-End Tests | ~15 | After Phase D | Full pipeline |
| Performance Tests | ~10 | After Phase E | Latency budgets |
| Safety Tests | ~20 | All phases | HITL, PIN, rollback, policy |

### 12.2 End-to-End Test Scenarios

```rust
// crates/kria-core/tests/phase8_e2e_intelligence_tests.rs

#[tokio::test]
async fn e2e01_voice_preemption() {
    // Background CuriosityLoop running → voice command arrives
    // Expected: CuriosityLoop cancelled within 500ms, voice task executes
}

#[tokio::test]
async fn e2e02_structured_command_no_injection() {
    // LLM outputs {"binary": "ls", "args": ["; rm -rf /"]}
    // Expected: "; rm -rf /" passed as literal arg to ls, NOT interpreted as shell
}

#[tokio::test]
async fn e2e03_uncertainty_evidence_gathering() {
    // "Make my VM faster" → confidence 0.4 → gather evidence → confidence 0.85 → plan
    // Expected: Evidence gathered before planning, all commands auto-approved (read-only)
}

#[tokio::test]
async fn e2e04_structured_branching_with_self_model() {
    // "Fix slow VM" → 3 paths → SelfModel Beta posterior scoring → winner selected
    // Expected: Unknown tools score 0.5, known tools use posterior mean
}

#[tokio::test]
async fn e2e05_quarantine_auto_promote_green() {
    // Read-only skill succeeds 3 times → auto-promoted to active registry
    // Expected: Skill available to LLM after 3rd success
}

#[tokio::test]
async fn e2e06_quarantine_hitl_yellow() {
    // Write skill succeeds 3 times → HITL approval required
    // Expected: Skill in PendingApproval status until user approves
}

#[tokio::test]
async fn e2e07_circuit_breaker_disables_skill() {
    // Compiled skill fails 3 consecutive times → disabled
    // Expected: Skill removed from active registry, fallback to LLM planning
}

#[tokio::test]
async fn e2e08_world_model_persistence() {
    // Execute task → World Model updated → facts persist across restart
    // Expected: Facts survive process restart with correct Beta posteriors
}

#[tokio::test]
async fn e2e09_goal_verification_deterministic() {
    // "Fix nginx" → execute → verify via "systemctl is-active nginx"
    // Expected: Goal verified by exit code, NOT by LLM
}

#[tokio::test]
async fn e2e10_planner_fallback_chain() {
    // Local 7B unavailable → cloud Gemini → heuristic planner
    // Expected: KRIA never fails to plan, just plans differently
}

#[tokio::test]
async fn e2e11_voice_latency_budget() {
    // Voice input → Executive Controller (P0) → direct tool → TTS
    // Expected: <2s total for simple commands
}

#[tokio::test]
async fn e2e12_full_learning_loop() {
    // Task → Success → Skill Compiler → Quarantine → Promotion → Direct execution
    // Expected: Second task 10x faster than first
}

#[tokio::test]
async fn e2e13_failure_learning_loop() {
    // Execute plan → failure → Failure Analyzer stores pattern → next plan avoids it
    // Expected: Next similar request uses different approach
}

#[tokio::test]
async fn e2e14_curiosity_budget_enforcement() {
    // CuriosityLoop running → voice command arrives → yields within 100ms
    // Expected: Voice task completes normally, Curiosity resumes after
}

#[tokio::test]
async fn e2e15_policy_gate_no_shell_injection() {
    // LLM tries: {"binary": "bash", "args": ["-c", "rm -rf /"]}
    // Expected: "bash" not in allowed_readonly_binaries → Quarantined or Blocked
}
```

### 12.3 Test Execution Commands

```bash
# Phase A tests (Executive Controller)
cargo test -p kria-core --test phase8_executive_tests

# Phase B tests (Policy Gate + Subprocess)
cargo test -p kria-core --test phase8_policy_gate_tests
cargo test -p kria-core --test phase8_subprocess_tests
cargo test -p kria-core --test phase8_code_interpreter_tests

# Phase C tests (Uncertainty + Planner + SelfModel)
cargo test -p kria-core --test phase8_uncertainty_tests
cargo test -p kria-core --test phase8_working_set_tests
cargo test -p kria-core --test phase8_planner_tests
cargo test -p kria-core --test phase8_self_model_tests

# Phase D tests (Skill Compiler + Quarantine)
cargo test -p kria-core --test phase8_skill_compiler_tests
cargo test -p kria-core --test phase8_quarantine_tests
cargo test -p kria-core --test phase8_failure_analyzer_tests
cargo test -p kria-core --test phase8_world_model_tests

# Phase E tests (Perception + Curiosity)
cargo test -p kria-core --test phase8_perception_tests
cargo test -p kria-core --test phase8_curiosity_tests

# End-to-end tests
cargo test -p kria-core --test phase8_e2e_intelligence_tests

# All Phase 8 tests
cargo test -p kria-core phase8

# Frontend tests
cd ui && npm test -- --testPathPattern=intelligence|executive|quarantine
```

---

## 13. Cascading Changes Matrix

### Phase A Cascading Changes (Executive Controller)

| Source File | Change | Affected Files | Risk |
|-------------|--------|----------------|------|
| `agent/executive/mod.rs` (NEW) | Central event loop | `agent/mod.rs`, `agent/loop_engine/mod.rs` | **High** |
| `agent/executive/controller.rs` (NEW) | Task scheduling | `agent/loop_engine/mod.rs` | High |
| `agent/executive/preemption.rs` (NEW) | GPU preemption | `resource/gpu_lease.rs` | Medium |
| `agent/executive/budget_guard.rs` (NEW) | Resource limits | `agent/curiosity/` | Low |
| `voice/v2/pipeline.rs` | Submit tasks to Executive | `agent/executive/` | Medium |

### Phase B Cascading Changes (Command Execution)

| Source File | Change | Affected Files | Risk |
|-------------|--------|----------------|------|
| `tools/subprocess_executor.rs` (NEW) | Replaces raw shell | `tools/mod.rs`, `tools/registry.rs` | **High** |
| `safety/policy_gate.rs` (NEW) | Deterministic safety | `safety/mod.rs` | Medium |
| `tools/code_interpreter.rs` (NEW) | Sandboxed scripts | `tools/subprocess_executor.rs` | Low |
| `tools/registry.rs` | Register new tools | None (internal) | Low |

### Phase C Cascading Changes (Uncertainty + Planner)

| Source File | Change | Affected Files | Risk |
|-------------|--------|----------------|------|
| `agent/uncertainty/mod.rs` (NEW) | Confidence scoring | `agent/loop_engine/mod.rs` | Medium |
| `agent/working_set/mod.rs` (NEW) | Structured context | `agent/planner_v2/` | Low |
| `agent/planner_v2/mod.rs` (NEW) | Replace linear planner | `agent/loop_engine/mod.rs` | **High** |
| `agent/self_model/mod.rs` (NEW) | Beta posterior stats | `agent/planner_v2/` | Low |
| `agent/goal_verifier.rs` (NEW) | Deterministic verification | `agent/planner_v2/executor.rs` | Low |
| `agent/planner_fallback.rs` (NEW) | Cloud/heuristic fallback | `agent/planner_v2/` | Low |
| `agent/loop_engine/mod.rs` | Switch to new planner | All tool execution | **High** |

### Phase D Cascading Changes (Memory + Skills)

| Source File | Change | Affected Files | Risk |
|-------------|--------|----------------|------|
| `agent/world_model/mod.rs` (NEW) | Persistent facts | `memory/store.rs` | Low |
| `agent/failure_analyzer/mod.rs` (NEW) | Failure patterns | `agent/planner_v2/` | Low |
| `agent/skill_compiler/mod.rs` (NEW) | Compile skills | `tools/quarantine.rs` | Medium |
| `tools/quarantine.rs` (NEW) | Safety gate | `tools/registry.rs` | Medium |
| `tools/registry.rs` | Accept promoted skills | `agent/loop_engine/mod.rs` | Medium |

### Phase E Cascading Changes (Perception + Curiosity)

| Source File | Change | Affected Files | Risk |
|-------------|--------|----------------|------|
| `agent/perception/mod.rs` (NEW) | Event hooks | `automation/proactive.rs` | Low |
| `agent/curiosity/mod.rs` (NEW) | Background tasks | `agent/executive/` | Medium |

### Phase F Cascading Changes (Browser + Dynamic)

| Source File | Change | Affected Files | Risk |
|-------------|--------|----------------|------|
| `tools/browser_agent.rs` (NEW) | Browser automation | `tools/registry.rs` | Low |
| `tools/dynamic_gen.rs` (NEW) | Dynamic tool creation | `tools/quarantine.rs` | Medium |
| `agent/prompt_optimizer/mod.rs` (NEW) | Prompt evolution | `agent/prompts.rs` | Low |

---

## 14. Rollout & Rollback

### Feature Flags

```toml
# kria_config.toml

[executive]
# Phase A: Executive Controller
enabled = false
max_background_tasks = 3
preemption_grace_period_ms = 500
voice_priority = 0
background_priority = 3

[policy_gate]
# Phase B: Command Execution
enabled = false
# Custom rules (overrides defaults)
# custom_rules = []

[uncertainty]
# Phase C: Uncertainty Engine
enabled = false
plan_threshold = 0.8
gather_threshold = 0.6
ask_threshold = 0.3
belief_decay_rate_per_hour = 0.05

[planner]
# Phase C: Structured Branching
enabled = false
max_steps = 20
max_replans = 3
working_set_max_tokens = 2048
fallback_to_cloud = true
cloud_api_key = ""  # Gemini Flash free tier

[self_model]
# Phase C: SelfModel
enabled = false
# Beta prior parameters
prior_alpha = 1.0
prior_beta = 1.0

[skill_compiler]
# Phase D
enabled = false
min_successes = 3
quarantine_enabled = true
circuit_breaker_threshold = 3

[world_model]
# Phase D
enabled = false
staleness_hours = 168  # 7 days

[curiosity]
# Phase E
enabled = false
max_cpu_percent = 10
cooldown_secs = 60
max_commands_per_cycle = 10

[browser_agent]
# Phase F
enabled = false

[dynamic_tool_gen]
# Phase F
enabled = false
schema_validation_enabled = true
sandbox_dry_run_required = true

[prompt_optimizer]
# Phase F
enabled = false
```

### Rollout Order

| Week | Feature | Flag | Default |
|------|---------|------|---------|
| 1-2 | Executive Controller | `executive.enabled` | `false` |
| 3-4 | Voice Preemption | (part of executive) | — |
| 5-6 | SubprocessExecutor + PolicyGate | `policy_gate.enabled` | `false` |
| 7 | Code Interpreter | (uses subprocess) | — |
| 8 | PolicyGate log viewer | (frontend) | — |
| 9-10 | Uncertainty Engine | `uncertainty.enabled` | `false` |
| 10 | WorkingSet (Structured) | (part of planner) | — |
| 11-12 | Structured Branching Planner | `planner.enabled` | `false` |
| 12 | SelfModel | `self_model.enabled` | `false` |
| 13 | GoalVerifier + Fallback Chain | (part of planner) | — |
| 14-15 | World Model | `world_model.enabled` | `false` |
| 16 | Failure Analyzer | (part of skill compiler) | — |
| 17-18 | Skill Compiler + Quarantine | `skill_compiler.enabled` | `false` |
| 19-20 | Event-Driven Perception | (part of curiosity) | — |
| 21-22 | CuriosityLoop | `curiosity.enabled` | `false` |
| 23 | Browser Agent | `browser_agent.enabled` | `false` |
| 24 | Dynamic Tool Gen + Prompt Optimizer | `dynamic_tool_gen.enabled` | `false` |

### Rollback Procedure

1. Set feature flag to `false` in `kria_config.toml`
2. Restart KRIA
3. System falls back to legacy behavior
4. No data loss (World Model, SelfModel, Skill Compiler data preserved in SQLite)
5. Quarantined tools remain in quarantine (can re-enable later)

---

## 15. Critical Audit Responses

### Audit Point 1: Execution Sandbox Vulnerability

**v1 Problem:** Raw `execute_shell` string passthrough — bash injection possible.

**v2 Solution:** AST-based `SubprocessExecutor` + deterministic `PolicyGate`.

**Why This Works:**
- LLM outputs `{"binary": "ls", "args": ["-la", "/tmp"]}` — NOT `ls -la /tmp; rm -rf /`
- Binary and args are separate fields — no shell parsing, no injection
- `PolicyGate` evaluates binary name + first-level args against structured rules
- Unknown binaries → Quarantined (HITL approval)
- `bash` and `sh` are NOT in the allowed_readonly_binaries — LLM cannot invoke shell interpreters

**Intelligence Preservation:**
- ~30 common read-only binaries auto-approved (ls, cat, top, ps, df, free, systemctl status, git status/log/diff, docker ps, etc.)
- Common write operations auto-approved with audit logging (mkdir, cp, mv, chmod, systemctl restart)
- Only destructive/unknown operations require HITL
- Result: KRIA can diagnose and fix most issues without nagging the user

### Audit Point 2: Statistical Bias in SelfModel

**v1 Problem:** Raw percentage (`success/total`) — 1 success = 100%.

**v2 Solution:** Beta(α, β) posterior estimation.

**Why Beta Distribution (Not Just Laplace Smoothing):**

Laplace smoothing ($P = \frac{S+1}{N+2}$) is the posterior mean of a Beta(1,1) prior — it's a special case. But Beta gives us more:

1. **Adjustable priors:** Beta(2,1) encodes "probably good" for tools we have reason to trust (e.g., built-in tools)
2. **Confidence intervals:** $\text{CI}_{95\%} \approx p \pm 1.96\sqrt{\frac{p(1-p)}{n}}$ — we can report uncertainty, not just point estimates
3. **Conjugate prior:** Beta is the conjugate prior for Bernoulli — updates are exact, not approximate
4. **Natural regularization:** With few observations, the prior dominates. With many, the data dominates.

**Examples:**
| Scenario | α | β | Posterior Mean | Interpretation |
|----------|---|---|----------------|----------------|
| New tool (no data) | 1 | 1 | 0.50 | Neutral — don't trust or distrust |
| 1 success | 2 | 1 | 0.67 | Slightly positive, but uncertain |
| 10 successes | 11 | 1 | 0.92 | High confidence |
| 5 successes, 5 failures | 6 | 6 | 0.50 | Truly neutral |
| 1 success, 1 failure | 2 | 2 | 0.50 | Insufficient data |

**Is There a Better 2026 Approach?**

For this specific use case (tool selection with infrequent updates), Beta posterior is optimal because:
- **Thompson Sampling** requires real-time exploration/exploitation — overkill for a planning-time decision
- **Elo/Glicko** systems are designed for head-to-head comparisons, not binary outcomes
- **Neural bandits** require training data we don't have yet

Beta posterior is the right choice. If we later add real-time tool A/B testing, we can layer Thompson Sampling on top.

### Audit Point 3: Context Destruction in WorkingSet

**v1 Problem:** LLM summarization destroys error codes, paths, exact data.

**v2 Solution:** `StructuredExtractor` — deterministic field extraction.

**Why This Works:**
- Regex-based extraction preserves exact values (error codes, IPs, file paths, numeric values)
- Priority-based truncation: error codes > exit codes > structured fields > raw prose
- No LLM call during extraction — deterministic, fast, reliable
- Raw output preserved as "snippet" (truncated by line count, not semantic meaning)

**Rust-Native Implementation:**
- `regex` crate (already in workspace dependencies) for field extraction
- `HashMap<String, String>` for key-value pairs
- Line-based truncation (not token-based — simpler and more predictable)
- Total WorkingSet budget: 2048 tokens (fits in 7B context with room for reasoning)

### Audit Point 4: Quarantine Requirement

**v1 Problem:** Auto-promoting compiled skills after N=3 is risky.

**v2 Solution:** `QuarantineRegistry` with tiered promotion.

**How It Works:**
1. Compiled skill → QuarantineRegistry (NOT active ToolRegistry)
2. Skill is tested via the normal tool execution path (PolicyGate still applies)
3. After N=3 successes:
   - **Green risk** (read-only): auto-promote to active registry
   - **Yellow risk** (write): HITL approval required
   - **Red risk** (destructive): HITL approval + PIN required
   - **Black risk**: never promoted
4. Circuit breaker: 3 consecutive failures → skill disabled, reverts to LLM planning

**Non-Blocking Workflow:**
- HITL approval requests are async — they don't block the current task
- While pending, the skill can still be used in quarantine (with HITL approval for each use)
- User can batch-approve skills from the QuarantineQueue UI

### Audit Point 5: Missing Executive Controller

**v1 Problem:** No central brain — parallel modules can OOM VRAM.

**v2 Solution:** `ExecutiveController` with Tokio MPSC + priority queue.

**Why Tokio MPSC (Not Custom Priority Queue):**
- `tokio::sync::mpsc::unbounded_channel` — zero-copy, lock-free send
- `BinaryHeap<Reverse<QueuedTask>>` for priority ordering (standard library, no deps)
- `tokio::select!` for multiplexing task completion, new submissions, and shutdown
- Each task gets its own `CancellationToken` for fine-grained preemption

**VRAM Safety:**
- Only ONE foreground task can hold the GPU at a time
- Background tasks NEVER request GPU lease
- Voice (P0) can preempt any background task (P3/P4) within 500ms
- GPU lease is acquired per-task, not per-controller — no central bottleneck

**Concurrency Model:**
```
Voice Pipeline ──→ mpsc::send(VoiceTask) ──→ ExecutiveController
Text Chat    ──→ mpsc::send(InteractiveTask) ──→    │
HITL         ──→ mpsc::send(HitlResponse) ──→      │
Curiosity    ──→ mpsc::send(BackgroundTask) ──→     │
                                                     ↓
                                              BinaryHeap (priority queue)
                                                     ↓
                                              tokio::select! {
                                                task = recv() => schedule(task),
                                                result = foreground => handle(result),
                                                result = background => handle(result),
                                              }
```

### Audit Point 6: Timeline Fallacy

**v1 Problem:** 12 weeks for solo dev — fantasy.

**v2 Solution:** 24 weeks (6 months), reordered phases.

**Why 6 Months:**
- Phase A (4 weeks): Executive Controller is complex — it touches the core execution path
- Phase B (4 weeks): PolicyGate needs careful rule tuning — ship with conservative defaults
- Phase C (5 weeks): Uncertainty + Planner is the intellectual core — needs iteration
- Phase D (5 weeks): Skill Compiler + Quarantine is safety-critical — needs extensive testing
- Phase E (4 weeks): Event-driven perception is system-level — needs OS-specific testing
- Phase F (2 weeks): Browser + Dynamic tools are isolated — lower risk

**Key Reordering:**
- Executive Controller FIRST (was Phase D in v1) — without it, everything else is fragile
- PolicyGate SECOND (was Phase A in v1) — safe execution must exist before planning
- Uncertainty + Planner THIRD (was Phase A in v1) — needs safe execution to gather evidence

### Additional Hidden Flaws Fixed

| # | Flaw | Fix |
|---|------|-----|
| 7 | No retry on execution path | Added max 3 retries with exponential backoff in SubprocessExecutor |
| 8 | LLM-only reflection | Added `GoalVerifier` with deterministic post-conditions |
| 9 | CuriosityLoop no resource budget | Added `BudgetGuard` — 10% CPU max, yields on foreground |
| 10 | No skill rollback | Added `CircuitBreaker` — 3 failures auto-disables |
| 11 | Fragile dynamic tool generation | Added `SchemaValidator` + sandbox dry-run |
| 12 | Sequential-only plan execution | Added `DependencyGraph` for parallel independent steps |
| 13 | No planner fallback | Added `PlannerFallbackChain` (local → cloud → heuristic) |
| 14 | WorkingSet compressor was stub | Implemented `StructuredExtractor` (deterministic, no LLM) |

---

## Appendix A: File Count Summary

| Category | New Files | Modified Files | Total |
|----------|-----------|----------------|-------|
| Phase A | 5 | 2 | 7 |
| Phase B | 4 | 2 | 6 |
| Phase C | 12 | 2 | 14 |
| Phase D | 8 | 2 | 10 |
| Phase E | 3 | 1 | 4 |
| Phase F | 4 | 1 | 5 |
| Frontend | 7 | 1 | 8 |
| Server | 0 | 1 | 1 |
| Tests | 18 | 0 | 18 |
| **Total** | **61** | **12** | **73** |

## Appendix B: Dependency Additions

```toml
# New dependencies needed:
regex = "1"          # Already in workspace — for StructuredExtractor
uuid = { version = "1", features = ["v4", "v7"] }  # Already in workspace
chrono = { version = "0.4", features = ["serde"] }  # Already in workspace

# Phase D: rusqlite (already available)
# Phase E: inotify (Linux kernel, no crate needed — use tokio::fs)
# Phase F: reqwest (already available for HTTP)
```

## Appendix C: Memory File Updates

After each phase completion, update `/memories/repo/` with:
- New module structure
- Integration points discovered
- Test patterns established
- Performance benchmarks achieved
- PolicyGate rules added/modified
- Quarantine promotion decisions

## Appendix D: Risk Matrix

| Phase | Risk | Mitigation |
|-------|------|------------|
| A (Executive) | High — touches core path | Feature flag, parallel run with legacy |
| B (PolicyGate) | Medium — replaces execution path | Conservative default rules, extensive testing |
| C (Planner) | Medium — replaces planning loop | Fallback chain (local → cloud → heuristic) |
| D (Skills) | Low — additive, async | QuarantineRegistry, circuit breaker |
| E (Perception) | Low — background only | BudgetGuard, yields on foreground |
| F (Browser) | Low — ephemeral, isolated | Docker sandbox, no host access |
