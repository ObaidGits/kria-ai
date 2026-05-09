# KRIA Intelligence Master Plan
## From Tool Router → True AI Agent (JARVIS-Class)

**Status:** Final Architecture Proposal  
**Date:** 2026-05-08  
**Author:** Systems Architecture  
**Target:** Voice-first AI assistant with true autonomous intelligence  

---

## Executive Summary

KRIA today is an excellent **tool router** — it classifies user intent and dispatches to pre-coded tools. This plan transforms it into a **true intelligent agent** that can:

- **Reason** about problems it's never seen before
- **Plan** multi-step solutions autonomously
- **Execute** arbitrary code on the user's machine and VMs
- **Observe** results and **replan** when things fail
- **Remember** what it learned for future tasks
- **Control** the entire laptop through voice

The transformation requires adding **4 new capability layers** on top of the existing infrastructure — all using free/open-source 2026 technologies.

---

## Current State vs Target State

```
CURRENT (Tool Router):
User: "Make my VM faster"
  → Router → Domain: SystemInfo → No tool match → LLM generates generic advice
  → Result: Useless text response ❌

TARGET (True Agent):
User: "Make my VM faster"
  → Router → Domain: Developer → No exact tool
  → Planner decomposes: [diagnose → identify bottleneck → fix → verify]
  → Step 1: SSH "top -bn1 | head -20" → observes nginx 80% CPU
  → Step 2: SSH "systemctl status nginx" → observes 64 workers
  → Step 3: SSH "sed 's/64/4/' /etc/nginx/nginx.conf && systemctl restart nginx"
  → Step 4: SSH "top -bn1 | head -5" → CPU now 12%
  → Result: "Fixed! Nginx had 64 workers, reduced to 4. CPU 80%→12%." ✅
```

---



---

## Architectural Flaw Analysis: Why Previous Plan Was Insufficient

The initial plan described *architecture* but not *intelligence*. Six critical flaws were identified:

| Flaw | Impact | Solution |
|------|--------|----------|
| **Python Orchestration Bloat** | IPC latency, memory duplication, 6GB VRAM cannot support Python frameworks alongside 7B model | **Pure Rust orchestration** for core cognitive loop. Python confined to ephemeral tool sandboxes only. |
| **Linear Planning Trap** | 7B models hallucinate with single-path planning. Executing the first generated plan leads to destructive OS actions. | **Tree-of-Thoughts (ToT) planning** — force 7B to simulate and score 3 distinct paths before executing the highest-confidence route. |
| **Reactive Polling** | Cron-based system checks (every 5 min) create massive blind spots. AI is deaf to real-time anomalies. | **Event-driven perception** via `inotify`/`tokio::fs` for filesystem, `dbus` signals for system events, kernel `netlink` for network. Sub-millisecond real-time. |
| **Logging vs Learning** | Saving past actions to a database is mere logging. KRIA endlessly re-solves similar issues from scratch. | **Skill Compiler** — when a plan succeeds, abstract variables and compile the execution graph into a reusable, zero-shot tool schema. |
| **No Uncertainty Awareness** | Binary execution (Plan -> Act) forces the AI to guess when lacking information, leading to catastrophic failure. | **Uncertainty Engine** — if confidence < 0.6, mandate "Gather Evidence" or "Ask User" before planning. Never wake the 7B GPU model to guess. |
| **Sensory Disconnect** | Relying entirely on CLI tools limits control. Parsing STT strips emotional and contextual audio cues. | **Vision-Language-Action (VLA)** for UI bounding-box control via `xdotool`. Full-duplex voice transport for interruptible conversation. |

### Key Design Principle

**The 0.5B router should NEVER guess.** If it's uncertain, it gathers evidence (read-only commands) or asks the user. The 7B planner is only woken when confidence exceeds the threshold AND the task requires reasoning.


## Architecture: 6 Intelligence Layers (Cognitive)

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    KRIA COGNITIVE BRAIN (6 Layers)                       │
│                                                                         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  LAYER 0: Perceive (Enhanced)                                   │   │
│  │  Voice (STT/TTS/VAD) · Vision · Screen · Files                 │   │
│  │  + Event-Driven: inotify · dbus · netlink (no polling)          │   │
│  └──────────────────────────┬──────────────────────────────────────┘   │
│                              ↓                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  LAYER 1: Route & Classify (Partially Built + Enhanced)         │   │
│  │  Phase 1-5 Routing System · Intent Classification               │   │
│  │  Tool Semantic Index · Context-Aware · Feedback Learning        │   │
│  └──────────────────────────┬──────────────────────────────────────┘   │
│                              ↓                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  LAYER 2: Uncertainty Engine (NEW)                              │   │
│  │  Belief Graph · Confidence Scoring · Evidence Gathering         │   │
│  │  → Low confidence: Gather Evidence (read-only) or Ask User      │   │
│  │  → High confidence: Proceed to Planning                         │   │
│  └──────────────────────────┬──────────────────────────────────────┘   │
│                              ↓                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  LAYER 3: Reason & Plan (Enhanced — ToT + MCTS)                 │   │
│  │  Tree-of-Thoughts Planning · Monte Carlo Simulation             │   │
│  │  Multi-Path Evaluation · Reflection Loop                        │   │
│  │  Pure Rust orchestration (no Python sidecar for core loop)      │   │
│  └──────────────────────────┬──────────────────────────────────────┘   │
│                              ↓                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  LAYER 4: Act & Execute (Enhanced)                              │   │
│  │  Tool Execution · Open Shell · Code Interpreter                 │   │
│  │  Browser Agent · VM Control · File Operations                   │   │
│  └──────────────────────────┬──────────────────────────────────────┘   │
│                              ↓                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  LAYER 5: Skill Compiler (NEW — True Self-Improvement)          │   │
│  │  Pattern Extraction · Variable Abstraction · Tool Compilation   │   │
│  │  → Successful plan → Compile into reusable tool schema          │   │
│  │  → Next time: 0.5B router matches directly, skips 7B entirely   │   │
│  └──────────────────────────┬──────────────────────────────────────┘   │
│                              ↓                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  SAFETY & CONTROL (Already Built + Enhanced)                    │   │
│  │  HITL · PIN Guard · Risk Levels · Audit Log · Rollback          │   │
│  │  + Uncertainty Gate · Step Approval · Dry Run                    │   │
│  └─────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Layer 2: Uncertainty Engine + Layer 3: Reason & Plan — The Brain

### Critical Design Decision: Pure Rust Only

**LangGraph, CrewAI, and all Python orchestration frameworks are REJECTED for the core cognitive loop.** Reasons:

| Concern | Impact on KRIA |
|---------|---------------|
| IPC latency (Rust ↔ Python) | +50-200ms per round-trip, destroys voice latency target |
| Memory duplication | Python process + Rust process = double RAM usage on 16GB system |
| VRAM contention | Python frameworks load their own model copies, stealing from 6GB budget |
| State fragility | Python sidecar crashes = silent intelligence failure |
| Deployment complexity | Two runtimes to maintain, debug, and version |

**Python is used ONLY for:** Ephemeral tool execution sandboxes (Browser-Use, code interpreter scripts). Not for orchestration.

### Uncertainty Engine (Layer 2)

Before planning, KRIA must score its own confidence. **The 0.5B router should NEVER guess.**

```rust
pub struct BeliefState {
    pub proposition: String,       // "Nginx is OOM crashing"
    pub confidence: f32,           // 0.0-1.0
    pub evidence: Vec<String>,     // ["syslog: OOM killer invoked", "top: nginx 80%"]
    pub gather_plan: Option<Vec<String>>, // Read-only commands to gather more evidence
}

pub enum UncertaintyAction {
    /// Confidence high enough — proceed to planning
    Plan { belief: BeliefState },
    /// Confidence medium — gather more evidence first
    GatherEvidence { commands: Vec<String> },
    /// Confidence low — ask the user for clarification
    AskUser { question: String },
    /// Confidence very low — refuse and explain why
    Refuse { reason: String },
}

impl UncertaintyEngine {
    pub fn evaluate(&self, goal: &str, context: &SystemContext) -> UncertaintyAction {
        let confidence = self.score_confidence(goal, context);

        match confidence {
            c if c >= 0.8 => UncertaintyAction::Plan { belief: self.current_belief() },
            c if c >= 0.6 => UncertaintyAction::GatherEvidence {
                commands: self.diagnostic_commands(goal)
            },
            c if c >= 0.3 => UncertaintyAction::AskUser {
                question: self.clarification_question(goal)
            },
            _ => UncertaintyAction::Refuse {
                reason: "I'm not confident enough to act on this. Can you provide more details?".into()
            },
        }
    }
}
```

**Evidence Gathering (read-only, no risk):**
- "Make my VM faster" → confidence 0.4 → gather: `top -bn1`, `free -h`, `df -h`, `systemctl list-units --state=running`
- After gathering → confidence 0.85 → proceed to planning

### Structured Branching (Layer 3)

**Open-ended Tree-of-Thoughts is rejected.** 7B models fail at unguided ToT — they produce verbose, hallucinated branches. Instead, the Planner is **forced** to generate exactly 3 structured paths with specific templates:

```
User: "Make my VM faster"
    ↓
Uncertainty Engine: confidence 0.4 → Gather Evidence
    ↓
Evidence: nginx 80% CPU, 64 workers, 2GB RAM free
    ↓
Confidence: 0.85 → Proceed to Planning
    ↓
┌─────────────────────────────────────────────────────────┐
│  STRUCTURED BRANCHING (3 forced templates)              │
│                                                         │
│  PATH A — DIAGNOSE-FIRST (read-only, safe)              │
│  Steps: [top -bn1, systemctl status nginx, free -h]     │
│  Risk: None (read-only)                                 │
│  Confidence: 0.95                                       │
│                                                         │
│  PATH B — MINIMAL-RISK FIX                              │
│  Steps: [reduce nginx workers 64→4, restart nginx]      │
│  Risk: Low (config change, reversible)                  │
│  Confidence: 0.88                                       │
│                                                         │
│  PATH C — AGGRESSIVE FIX                                │
│  Steps: [kill nginx, install caddy, migrate config]     │
│  Risk: High (service replacement, hard to rollback)     │
│  Confidence: 0.60                                       │
│                                                         │
│  SELF-MODEL EVALUATION:                                 │
│  Path A: nginx_status tool success rate = 98%           │
│  Path B: config_edit tool success rate = 85%            │
│  Path C: service_replace tool success rate = 45%        │
│                                                         │
│  Winner: Path B (best risk/reward ratio)                │
└─────────────────────────────────────────────────────────┘
    ↓
Execute Path B → Observe → CPU 12% → Goal achieved ✅
```

**Why structured branching beats open-ended ToT:**
- **Deterministic output format** — 7B model always produces exactly 3 paths
- **Risk-graduated** — Each path has a clear risk level (safe/medium/high)
- **Self-model integration** — Paths are scored against historical success rates
- **Lower latency** — Structured prompts produce faster, shorter responses than open-ended ToT

### Architecture: Plan-Execute-Reflect Loop (Rust-Native)

```
User Goal
    ↓
┌──────────────┐
│   PLANNER    │ ← Dedicated LLM (Qwen2.5-7B Q4_K_M, ~4.5GB VRAM)
│              │    Evicts TTS/Vision when loading, swaps back when done
│              │
│  Input: goal + context + available tools + system state
│  Output: [Step 1, Step 2, ..., Step N]
└──────┬───────┘
       ↓
┌──────────────┐     ┌──────────────┐
│   EXECUTOR   │ ──→ │   OBSERVER   │
│              │     │              │
│  Runs step   │     │  Reads result│
│  on target   │     │  Checks goal │
└──────────────┘     └──────┬───────┘
                            ↓
                    ┌──────────────┐
                    │   REFLECTOR  │
                    │              │
                    │  Goal met?   │──→ Yes → Done ✅
                    │  Failed?     │──→ Replan with error context
                    │  Partial?    │──→ Continue with adjusted plan
                    └──────────────┘
```

### Implementation Strategy

**Option A: Embed LangGraph State Machine in Rust (Recommended)**

LangGraph (31.5k stars, MIT) provides the Plan-Execute-Reflect pattern as a graph:

```python
# Python sidecar — LangGraph Plan-Execute agent
from langgraph.graph import StateGraph
from langchain_core.messages import HumanMessage

class PlanState(TypedDict):
    input: str
    plan: list[str]
    past_steps: list[tuple]
    response: str

# Planner node: decomposes goal into steps
# Executor node: runs each step (SSH, shell, API call)
# Re-planner node: observes results, adjusts plan
# Should-end node: checks if goal is satisfied
```

**How it integrates with KRIA:**

```
KRIA Rust AgentLoop
    ↓ (when no tool match and complexity > threshold)
    ↓
Python Sidecar (LangGraph Plan-Execute)
    ↓
Step execution via KRIA's existing:
    - QemuSshEnvironment (for VM tasks)
    - CommandExecutor (for local tasks)
    - ToolRegistry (for existing tools)
    ↓
Results stream back to Rust AgentLoop
    ↓
Response to user via TTS/text
```

**Option B: Pure Rust Plan-Execute (Simpler, No Python)**

Implement the Plan-Execute-Reflect loop directly in your existing `loop_engine`:

```rust
pub struct PlanExecuteLoop {
    planner: LlmBackend,      // 7B Q4_K_M on GPU (swaps with Vision/TTS)
    max_steps: usize,          // Safety: max 20 steps
    reflection_threshold: f32, // When to replan
}

impl PlanExecuteLoop {
    pub async fn execute_goal(&self, goal: &str, context: &PlanContext) -> PlanResult {
        let mut plan = self.planner.decompose(goal, context).await;
        let mut results = Vec::new();
        
        for step in &plan.steps {
            let result = self.execute_step(step).await;
            results.push(result);
            
            // Reflect after each step
            let reflection = self.planner.reflect(goal, &results).await;
            match reflection.outcome {
                GoalOutcome::Achieved => return PlanResult::Success(results),
                GoalOutcome::Failed { reason } => {
                    plan = self.planner.replan(goal, &results, &reason).await;
                }
                GoalOutcome::Continue => {},
            }
        }
        
        PlanResult::Partial(results)
    }
}
```

### Tech Stack (Free/Open Source 2026)

| Component | Technology | Why |
|-----------|-----------|-----|
| Planner LLM | **Qwen2.5-7B-Instruct Q4_K_M** (local, ~4.5GB VRAM) or **Gemini Flash** (free API fallback) | Best reasoning per token, free |
| Plan-Execute Pattern | **LangGraph** (Python sidecar) or **Pure Rust** (no dependency) | Battle-tested, 31.5k stars |
| Multi-Agent | **CrewAI** (Python sidecar) | 50.9k stars, MIT, supports local models via Ollama |
| Reflection | LLM self-evaluation | Built into Plan-Execute loop |

### What Gets Added to KRIA

```
NEW: crates/kria-core/src/agent/planner/
├── mod.rs           — Planner orchestrator
├── decomposer.rs    — Goal → Step decomposition
├── executor.rs      — Step execution dispatcher
├── reflector.rs     — Result evaluation + replanning
└── plan_executor.rs — Full Plan-Execute-Reflect loop
```

---

## Layer 3: Act & Execute — The Hands

### 3A: Open Shell Execution

**What:** Let the LLM generate and execute ANY shell command, not just pre-coded tools.

**How:** Add a generic `execute_shell` tool that the LLM can call with arbitrary commands:

```rust
// The LLM generates:
ToolCall {
    name: "execute_shell",
    arguments: {
        "command": "ping -c 3 8.8.8.8",
        "target": "vm1",  // or "local"
        "timeout": 30
    }
}

// KRIA executes on:
// - Local machine (CommandExecutor)
// - VM via SSH (QemuSshEnvironment)
// - Docker container (DockerEnvironment)
```

**Safety:** Your existing HITL + PIN guard + risk levels already handle this. Green commands (read-only) auto-execute. Yellow/Red require approval.

### 3B: Code Interpreter

**What:** LLM writes Python/shell scripts → executes in sandbox → returns result.

**How:** Use your existing QEMU VMs as the sandbox:

```
LLM writes Python script
    ↓
Write to temp file on VM via SSH
    ↓
Execute: ssh vm1 "python3 /tmp/kria_code_XXXX.py"
    ↓
Capture stdout/stderr
    ↓
Feed result back to LLM for reflection
```

**Free tools:**
- **Pyodide** (WebAssembly Python) — for lightweight in-browser execution
- **Your QEMU VMs** — for full Python/Node/Shell execution (already built)
- **Docker containers** — for isolated execution (already integrated)

### 3C: Browser Agent

**What:** LLM controls a real web browser to perform web tasks.

**How:** Integrate [Browser-Use](https://github.com/browser-use/browser-use) (92.8k stars, MIT) as a Python sidecar:

```
User: "Find me the cheapest 32GB DDR5 RAM on Amazon"
    ↓
Browser-Use agent:
    - Navigates to Amazon
    - Searches for "32GB DDR5 RAM"
    - Compares prices
    - Returns top 3 options with prices
```

**Integration with KRIA:**

```rust
// New tool: browser_agent
ToolCall {
    name: "browser_agent",
    arguments: {
        "task": "Find cheapest 32GB DDR5 RAM on Amazon",
        "max_steps": 20
    }
}
```

### 3D: Full Laptop Control

**What:** The LLM can control any aspect of the laptop through existing tools + open shell.

**What KRIA already has:**

| Capability | Status | How |
|-----------|--------|-----|
| File operations | ✅ Built | `read_file`, `write_file`, `delete_file` |
| Process management | ✅ Built | `list_running_apps`, `kill_process` |
| System config | ✅ Built | `set_volume`, `set_brightness`, `connect_wifi` |
| Application control | ✅ Built | `open_application`, `close_application` |
| Package management | ✅ Built | `install_package`, `uninstall_package` |
| VM control | ✅ Built | `execute_fleet_command` |
| Git operations | ✅ Built | `git_status`, `git_commit`, `git_push` |
| Google Workspace | ✅ Built | `gw_gmail_send`, `gw_calendar_create` |
| Screenshot/vision | ✅ Built | `screenshot`, `analyze_image` |

**What's missing:** The LLM's ability to **chain these creatively** for tasks it's never seen. That's Layer 2 (Planner).

---

## Layer 4: Remember & Learn — The Memory

### Current Memory (Basic)

KRIA has:
- ✅ Conversation turns (SQLite)
- ✅ User preferences (key-value)
- ✅ Fact extraction (basic)
- ✅ RAG knowledge base

### Enhanced Memory (True Intelligence)

| Memory Type | What It Stores | Example |
|-------------|---------------|---------|
| **Episodic** | What happened in each task | "Fixed nginx on VM1 by reducing workers from 64 to 4" |
| **Procedural** | How to do things | "To check VM health: SSH → top → systemctl → report" |
| **Semantic** | Facts and knowledge | "User's VM1 IP is 192.168.122.240, runs Ubuntu 24.04" |
| **Preference** | User likes/dislikes | "User prefers Hinglish, voice-first, 2-second response" |
| **Pattern** | Recurring tasks | "Every Monday: check system health, check email, check calendar" |

### Implementation: Task Pattern Memory

```rust
pub struct TaskPattern {
    pub trigger: String,           // "make VM faster"
    pub successful_plan: Vec<Step>, // [diagnose → fix → verify]
    pub success_count: usize,       // How many times this worked
    pub avg_duration: Duration,     // How long it takes
    pub learned_from: Vec<String>,  // Session IDs where this was learned
}

// When user says "make VM faster" again:
// 1. Check task pattern memory → found previous successful plan
// 2. Skip Planner LLM → reuse the known plan
// 3. Execute directly → much faster
```

### Tech Stack

| Component | Technology | Why |
|-----------|-----------|-----|
| Vector memory | **ChromaDB** (free, local) or existing `VectorIndex` | Semantic search over memories |
| Graph memory | **Graphify** (already integrated) | Knowledge graph of relationships |
| Procedural memory | SQLite task patterns | Structured storage of successful plans |
| Memory extraction | LLM-based fact extraction | Already partially built |

---



---

## Layer 5: Skill Compiler — True Self-Improvement

### The Problem with "Memory"

Saving past actions to a database is **logging**, not learning. KRIA will endlessly re-solve similar issues from scratch because it doesn't generalize.

### The Skill Compiler Solution (Calibrated Compilation)

When the planner successfully resolves a novel task, the execution graph is abstracted and compiled into a **reusable tool schema** — but ONLY after it has succeeded in **3 slightly varied contexts**. This prevents over-generalization from a single lucky execution.

```
Attempt 1: "Make my VM faster" → reduce nginx workers → success ✅
    → Stored as uncompiled "playbook" (requires Planner oversight)

Attempt 2: "Optimize my web server" → reduce nginx workers → success ✅
    → Same pattern, different phrasing → playbook confidence: 0.7

Attempt 3: "Fix slow VM1" → reduce nginx workers → success ✅
    → 3 varied successes → READY FOR COMPILATION
    ↓
SKILL COMPILER:
    - Abstract: hardcoded IP → variable {target_host}
    - Abstract: "nginx" → variable {service_name}
    - Abstract: "64" → variable {config_key}
    ↓
Compiled Skill: optimize_service_workers(target_host, service_name, config_key, new_value)
    ↓
Registered as new tool schema in ToolRegistry
    → Success count: 3, Confidence: 0.85
```

```
Next time: "Make my VM faster"
    ↓
0.5B Router matches intent → finds compiled skill "optimize_service_workers"
    ↓
Direct execution (50ms) → No 7B model needed → No LLM cost
```

**Calibration Rules:**
- **N=3 minimum** — A pattern must succeed 3 times in varied contexts before compilation
- **Variation required** — The 3 successes must use different phrasings or targets
- **Failure resets counter** — If the pattern fails once, the counter resets to 0
- **Confidence decay** — Compiled skills lose 0.01 confidence per week without use

### Implementation

```rust
pub struct SkillCompiler {
    /// Extracted patterns from successful plans
    compiled_skills: Vec<CompiledSkill>,
}

pub struct CompiledSkill {
    pub name: String,
    pub description: String,
    pub trigger_patterns: Vec<String>,  // "make VM faster", "optimize service", etc.
    pub parameters: Vec<ParamDef>,      // Abstracted variables
    pub execution_graph: ExecutionGraph, // Parameterized step sequence
    pub success_count: usize,
    pub avg_duration: Duration,
    pub confidence: f32,
}

impl SkillCompiler {
    /// After a successful plan, extract and compile into a reusable skill.
    pub fn compile_from_success(&mut self, plan: &SuccessfulPlan) -> CompiledSkill {
        let variables = self.extract_variables(plan);
        let graph = self.parameterize_graph(plan, &variables);
        let trigger_patterns = self.generate_trigger_patterns(plan);

        CompiledSkill {
            name: self.generate_skill_name(plan),
            description: plan.goal.clone(),
            trigger_patterns,
            parameters: variables,
            execution_graph: graph,
            success_count: 1,
            avg_duration: plan.duration,
            confidence: 0.8,
        }
    }

    /// Check if a compiled skill matches the user's intent.
    pub fn match_skill(&self, text: &str) -> Option<&CompiledSkill> {
        self.compiled_skills.iter()
            .filter(|s| s.confidence > 0.7)
            .find(|s| s.trigger_patterns.iter().any(|p| similarity(text, p) > 0.8))
    }
}
```

### What Gets Added

```
NEW: crates/kria-core/src/agent/skill_compiler/
├── mod.rs              — Skill compiler orchestrator
├── pattern_extractor.rs — Extract variables from execution graphs
├── graph_parameterizer.rs — Abstract hardcoded values into parameters
└── trigger_generator.rs — Generate trigger patterns from goal descriptions
```




---

## Critical Gap Analysis: What's Still Missing for True Intelligence

The plan above describes **architecture** (how to build the system) but not **cognition** (how the system thinks). After deep analysis, five critical gaps remain that separate "orchestrated tool router" from "true intelligent agent."

### Gap 1: No Failure Learning

**Problem:** The Skill Compiler only learns from **success**. But intelligence comes equally from failure. If KRIA tries "restart nginx" and it makes things worse, that knowledge is lost.

**Solution:** Failure Analyzer

```rust
pub struct FailureAnalyzer {
    failure_patterns: Vec<FailurePattern>,
}

pub struct FailurePattern {
    pub trigger: String,              // "make VM faster"
    pub failed_plan: Vec<Step>,       // [restart nginx]
    pub failure_reason: String,       // "nginx config was wrong, restart crashed site"
    pub what_would_have_worked: Option<Vec<Step>>, // [fix config first, then restart]
    pub confidence: f32,
}

impl FailureAnalyzer {
    /// After a failed plan, extract what went wrong.
    pub fn analyze_failure(&mut self, plan: &FailedPlan) -> FailurePattern {
        FailurePattern {
            trigger: plan.goal.clone(),
            failed_plan: plan.steps.clone(),
            failure_reason: self.extract_root_cause(plan),
            what_would_have_worked: self.suggest_alternative(plan),
            confidence: 0.7,
        }
    }

    /// Before executing a plan, check if it matches known failure patterns.
    pub fn check_plan_against_failures(&self, plan: &PlannedSteps) -> Option<&FailurePattern> {
        self.failure_patterns.iter().find(|f| {
            f.failed_plan.iter().any(|step| {
                plan.steps.iter().any(|p| similarity(&p.command, &step.command) > 0.8)
            })
        })
    }
}
```

**How it works in practice:**

```
First time: "Make my VM faster"
  → Plan: restart nginx
  → Result: site crashed ❌
  → Failure Analyzer: "restart nginx without checking config → crash"
  → Stored: FailurePattern { trigger: "make VM faster", failed: [restart nginx], reason: "config wrong" }

Next time: "Make my VM faster"
  → Plan: restart nginx
  → Failure Analyzer: MATCH! This plan matches a known failure pattern
  → Modified plan: [check nginx config → fix if wrong → then restart]
  → Result: site stays up ✅
```

### Gap 2: No Reasoning Scaffolding

**Problem:** The plan assumes the 7B model can "just think." But small models need **explicit reasoning scaffolding** — structured prompts that force step-by-step thinking. Without this, the 7B model will jump to conclusions.

**Solution:** Chain-of-Thought System Prompt (built into the planner)

```
You are KRIA's planning engine. You MUST follow this reasoning process:

STEP 1 — UNDERSTAND:
  What is the user actually asking? (paraphrase in your own words)
  What domain is this? (system admin, file ops, communication, etc.)
  What do I already know about this system? (check belief graph)

STEP 2 — HYPOTHESIZE:
  What are the possible causes/solutions? (list 3)
  What evidence would confirm each hypothesis?
  What's the risk of each approach?

STEP 3 — PLAN:
  Select the highest-confidence, lowest-risk path
  List exact commands/steps
  Predict what each step will produce
  Identify rollback steps if something goes wrong

STEP 4 — VERIFY:
  How will I know the goal is achieved?
  What metrics should I check after execution?
  What's the "undo" command if this fails?

Current system state: {belief_graph}
Available tools: {tool_descriptions}
User goal: {goal}

Think step by step. Do not skip steps.
```

### Gap 3: No World Model Persistence

**Problem:** The system starts fresh every conversation. It doesn't maintain a persistent understanding of the user's world (machines, services, preferences, history).

**Solution:** Persistent World Model (SQLite + vector embeddings)

```rust
pub struct WorldModel {
    /// Facts about the user's systems
    system_facts: Vec<SystemFact>,
    /// Facts about the user's preferences
    user_facts: Vec<UserFact>,
    /// Facts about the user's current projects
    project_facts: Vec<ProjectFact>,
}

pub struct SystemFact {
    pub subject: String,     // "VM1"
    pub predicate: String,   // "runs"
    pub object: String,      // "Ubuntu 24.04"
    pub confidence: f32,
    pub last_verified: DateTime,
    pub source: String,      // "detected via SSH" or "user told me"
}

// Examples of what the World Model stores:
// "VM1" → "runs" → "Ubuntu 24.04" (confidence: 0.95, verified: today)
// "VM1" → "has IP" → "192.168.122.240" (confidence: 0.99, verified: today)
// "VM1" → "runs service" → "nginx" (confidence: 0.9, verified: yesterday)
// "User" → "prefers" → "Hinglish" (confidence: 0.99, source: explicit)
// "User" → "works on" → "KRIA project" (confidence: 0.95, source: observed)
// "User" → "has GPU" → "RTX 4050 6GB" (confidence: 0.99, source: detected)
```

**How it integrates:**

```
User: "Make my VM faster"
    ↓
Planner reads World Model:
  - VM1: Ubuntu 24.04, IP 192.168.122.240, runs nginx, 8GB RAM
  - User: prefers voice, Hinglish, 2-second response
  - Last task: fixed nginx workers 2 days ago
    ↓
Planner uses this context to generate BETTER plans
  (not generic "check system" but specific "check nginx on VM1 at 192.168.122.240")
```

### Gap 4: No Prompt Self-Improvement

**Problem:** The system prompt never changes. But the best prompts are learned from experience.

**Solution:** DSPy-style prompt optimization (simplified, no Python)

```rust
pub struct PromptOptimizer {
    /// Current system prompt template
    template: String,
    /// Performance history: (prompt_variant, task_type, success_rate)
    history: Vec<(String, String, f32)>,
}

impl PromptOptimizer {
    /// After each task, record whether the prompt variant worked.
    pub fn record_outcome(&mut self, variant: &str, task_type: &str, success: bool) {
        // Track which prompt variants work best for which task types
    }

    /// Periodically, try small variations and keep what works.
    pub fn optimize(&mut self) {
        // For each task type, find the variant with highest success rate
        // Gradually shift the template toward the best variants
    }
}
```

**Example:**

```
Week 1: System prompt says "Generate 3 plans"
  → Success rate: 70%

Week 2: Try "Generate 3 plans, each with rollback steps"
  → Success rate: 85%

Week 3: Try "Generate 3 plans, each with rollback steps and verification commands"
  → Success rate: 92%

Week 4: Prompt automatically adopts the best variant
```

### Gap 5: No Confidence Calibration

**Problem:** The Uncertainty Engine uses arbitrary thresholds (0.6, 0.8). But these need to be calibrated against actual outcomes.

**Solution:** Adaptive Thresholds

```rust
pub struct ConfidenceCalibrator {
    /// Historical outcomes: (predicted_confidence, actual_success)
    outcomes: Vec<(f32, bool)>,
    /// Calibrated thresholds
    plan_threshold: f32,      // Start at 0.8, adjust based on outcomes
    gather_threshold: f32,    // Start at 0.6, adjust based on outcomes
    ask_threshold: f32,       // Start at 0.3, adjust based on outcomes
}

impl ConfidenceCalibrator {
    /// After each task, update thresholds based on actual outcomes.
    pub fn calibrate(&mut self) {
        // If the system planned at 0.7 confidence and succeeded → threshold is fine
        // If it planned at 0.7 and failed → threshold should be higher
        // If it gathered evidence at 0.5 and the evidence was useful → threshold is fine
        // If it asked user at 0.3 but the answer was obvious → threshold should be lower
    }
}
```

### Gap 6: Working Memory Bloat

**Problem:** The 7B model's context window gets filled with irrelevant conversation history, causing reasoning fragmentation. The planner loses focus.

**Solution:** WorkingSet — A cognitive scratchpad

```rust
pub struct WorkingSet {
    /// The active goal stack (what we're trying to achieve)
    pub goal_stack: Vec<Goal>,
    /// Unresolved questions from the current task
    pub open_questions: Vec<String>,
    /// Immediate constraints (e.g., "don't restart nginx during business hours")
    pub constraints: Vec<Constraint>,
    /// Key evidence gathered so far (compressed summaries, not raw output)
    pub evidence_summary: Vec<String>,
    /// Max tokens for the WorkingSet (prevents context bloat)
    pub max_tokens: usize,
}

impl WorkingSet {
    /// Build a WorkingSet from the current task state.
    /// The Planner ONLY reads this, not the entire conversation history.
    pub fn build(goal: &str, world_model: &WorldModel, evidence: &[String]) -> Self {
        Self {
            goal_stack: vec![Goal::new(goal)],
            open_questions: Vec::new(),
            constraints: world_model.active_constraints(),
            evidence_summary: evidence.iter().map(|e| summarize(e)).collect(),
            max_tokens: 2048, // Fits comfortably in 7B context window
        }
    }

    /// Serialize to a compact string for the Planner's system prompt.
    pub fn to_prompt_context(&self) -> String {
        // Only includes: goal, constraints, evidence summary
        // Excludes: full conversation history, raw tool output, system prompt boilerplate
    }
}
```

**Why this matters:**
- 7B model with 8K context → if 6K is conversation history, only 2K for reasoning
- WorkingSet compresses to ~2K tokens → leaves 6K for actual reasoning
- Result: **dramatically better plan quality** from the same model

### Gap 7: No Capability Awareness (SelfModel)

**Problem:** The planner doesn't know its own strengths and weaknesses. It might try to use a tool that has a 30% success rate when a better alternative exists.

**Solution:** SelfModel — A capability graph with historical success rates

```rust
pub struct SelfModel {
    /// Per-tool success rates
    tool_stats: HashMap<String, ToolStats>,
    /// Per-domain routing accuracy
    domain_accuracy: HashMap<Domain, f32>,
    /// Known failure modes
    failure_modes: Vec<FailureMode>,
}

pub struct ToolStats {
    pub tool_name: String,
    pub total_calls: usize,
    pub successful_calls: usize,
    pub success_rate: f32,
    pub avg_latency: Duration,
    pub last_used: DateTime,
    pub known_failure_modes: Vec<String>,
}

impl SelfModel {
    /// When the planner generates paths, score them against the SelfModel.
    pub fn score_path(&self, path: &PlannedPath) -> f32 {
        let tool_scores: Vec<f32> = path.steps.iter()
            .map(|step| self.tool_stats.get(&step.tool_name)
                .map(|s| s.success_rate)
                .unwrap_or(0.5)) // Unknown tools get neutral score
            .collect();
        // Geometric mean (path fails if any step fails)
        tool_scores.iter().product::<f32>().powf(1.0 / tool_scores.len() as f32)
    }

    /// After execution, update success rates.
    pub fn record_outcome(&mut self, tool_name: &str, success: bool, latency: Duration) {
        let stats = self.tool_stats.entry(tool_name.to_string())
            .or_insert_with(|| ToolStats::new(tool_name));
        stats.record(success, latency);
    }
}
```

**How it integrates with Structured Branching:**
```
Planner generates 3 paths
    ↓
SelfModel scores each path:
  Path A (diagnose): nginx_status(98%) × free_h(95%) = 0.93
  Path B (fix):      config_edit(85%) × restart(90%) = 0.87
  Path C (aggressive): service_replace(45%) × install(80%) = 0.60
    ↓
Winner: Path A (highest SelfModel score, though Path B has better impact)
    ↓
Decision: Execute Path A first (gather more evidence), then Path B
```

### Gap 8: Passive Reactivity (CuriosityLoop)

**Problem:** The system only acts when spoken to. It never proactively investigates anomalies or learns about the environment.

**Solution:** CuriosityLoop — Background diagnostic engine

```rust
pub struct CuriosityLoop {
    /// Novelty detector: flags unusual system events
    novelty_detector: NoveltyDetector,
    /// Inquiry scheduler: runs low-priority diagnostics
    inquiry_scheduler: InquiryScheduler,
    /// World model to update with findings
    world_model: Arc<RwLock<WorldModel>>,
}

impl CuriosityLoop {
    /// Run in background when GPU is idle (Planner loaded but not busy).
    pub async fn run_background(&self) {
        loop {
            // Only run when Planner is idle (no active tasks)
            if !self.planner_is_idle() {
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }

            // Check for novelties
            if let Some(novelty) = self.novelty_detector.check() {
                // Run read-only diagnostics on the novelty
                let diagnosis = self.investigate(novelty).await;
                // Update World Model with findings
                self.world_model.write().await.update(diagnosis);
            }

            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }
}
```

**What triggers curiosity:**
- New service started on VM → "What is this? Is it expected?"
- Disk usage spike → "What's consuming space?"
- New device connected → "What device is this?"
- Unusual network traffic → "What's connecting outbound?"

**Safety:** CuriosityLoop only runs **read-only** diagnostics. It NEVER modifies system state without explicit user approval.

---

## Updated Cognitive Architecture (Complete)

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    KRIA COGNITIVE BRAIN (Complete)                       │
│                                                                         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  LAYER 0: Perceive (Event-Driven)                               │   │
│  │  Voice · Vision · Screen · Files                                │   │
│  │  inotify · dbus · netlink (sub-ms, no polling)                  │   │
│  │  + CuriosityLoop (background novelty detection)                 │   │
│  └──────────────────────────┬──────────────────────────────────────┘   │
│                              ↓                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  LAYER 1: Route & Classify (Phases 1-5, Already Built)          │   │
│  │  Intent Classification · Tool Semantic Index · Context-Aware     │   │
│  └──────────────────────────┬──────────────────────────────────────┘   │
│                              ↓                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  LAYER 2: World Model + Uncertainty Engine + WorkingSet          │   │
│  │  Persistent facts · Confidence scoring · Cognitive scratchpad    │   │
│  │  → Low: Gather Evidence or Ask User · High: Proceed to Plan     │   │
│  │  → WorkingSet compresses context for 7B model                   │   │
│  └──────────────────────────┬──────────────────────────────────────┘   │
│                              ↓                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  LAYER 3: Structured Branching Planner                          │   │
│  │  3 forced templates (Diagnose/Minimal-Risk/Aggressive)          │   │
│  │  SelfModel scoring (historical success rates per tool)          │   │
│  │  Failure pattern checking · Chain-of-Thought scaffolding        │   │
│  │  Pure Rust orchestration (no Python sidecar)                    │   │
│  └──────────────────────────┬──────────────────────────────────────┘   │
│                              ↓                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  LAYER 4: Act & Execute                                         │   │
│  │  Tool Execution · Open Shell · Code Interpreter                 │   │
│  │  Browser Agent · VM Control · File Operations                   │   │
│  └──────────────────────────┬──────────────────────────────────────┘   │
│                              ↓                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  LAYER 5: Learn & Improve (Calibrated)                          │   │
│  │  Skill Compiler (N=3 gating, success → reusable tool)           │   │
│  │  Failure Analyzer (failure → avoid pattern)                     │   │
│  │  SelfModel (per-tool success rates, updated after each task)    │   │
│  │  Confidence Calibrator (adaptive thresholds)                    │   │
│  └──────────────────────────┬──────────────────────────────────────┘   │
│                              ↓                                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  SAFETY & CONTROL                                               │   │
│  │  HITL · PIN Guard · Risk Levels · Audit Log · Rollback          │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                                                         │
│  HARDWARE ISOLATION:                                                    │
│  CPU: Router(0.5B) + TTS + VAD + Embeddings — always resident          │
│  GPU: Planner(7B) — permanently hot, evicted only for Vision/Image      │
└─────────────────────────────────────────────────────────────────────────┘
```

### What Gets Added (Final)

```
NEW: crates/kria-core/src/agent/
├── planner/
│   ├── mod.rs              — Structured Branching planner orchestrator
│   ├── decomposer.rs       — Goal → 3 forced-template decomposition
│   ├── executor.rs         — Step execution dispatcher
│   ├── reflector.rs        — Result evaluation + replanning
│   └── scaffolding.rs      — Chain-of-Thought prompt templates
├── skill_compiler/
│   ├── mod.rs              — Calibrated skill compiler (N=3 gating)
│   ├── pattern_extractor.rs — Extract variables from execution graphs
│   ├── graph_parameterizer.rs — Abstract hardcoded values
│   └── trigger_generator.rs — Generate trigger patterns
├── failure_analyzer/
│   ├── mod.rs              — Failure pattern extraction
│   ├── root_cause.rs       — Root cause analysis
│   └── alternative_suggester.rs — "What would have worked"
├── world_model/
│   ├── mod.rs              — Persistent fact store (SQLite)
│   ├── system_facts.rs     — Facts about user's machines/services
│   ├── user_facts.rs       — Facts about user preferences
│   └── project_facts.rs    — Facts about current projects
├── working_set/
│   ├── mod.rs              — Cognitive scratchpad for Planner
│   └── compressor.rs       — Context compression for 7B model
├── self_model/
│   ├── mod.rs              — Capability graph with success rates
│   └── tool_stats.rs       — Per-tool historical performance
├── uncertainty_engine/
│   ├── mod.rs              — Belief graph + confidence scoring
│   ├── evidence_gatherer.rs — Read-only diagnostic commands
│   └── calibrator.rs       — Adaptive threshold adjustment
├── curiosity_loop/
│   ├── mod.rs              — Background novelty detection
│   ├── novelty_detector.rs — Flags unusual system events
│   └── inquiry_scheduler.rs — Low-priority diagnostic scheduling
└── prompt_optimizer/
    ├── mod.rs              — Prompt variant tracking
    └── template_evolver.rs — Gradual template improvement
```

### The Complete Learning Loop

```
Task completed
    ↓
Was it successful?
    ├─ Yes → Skill Compiler: abstract into reusable tool
    └─ No  → Failure Analyzer: extract what went wrong
              ↓
Both outcomes update:
    - World Model (new facts learned)
    - Confidence Calibrator (adjust thresholds)
    - Prompt Optimizer (track which prompts worked)
    ↓
Next task benefits from ALL previous experience
```




## Dual-Model Architecture (6GB VRAM Optimized)

### Hardware Constraint

```
RTX 4050 — 6GB VRAM, 16GB System RAM
├── VRAM Budget: 6GB total
│   ├── Planner LLM (Q4_K_M): ~4.5GB
│   ├── Vision Model (when active): ~3GB (evicts Planner)
│   └── TTS (Piper): ~200MB (evicts with Vision)
└── CPU Budget: 16GB RAM
    ├── Router LLM (0.5B Q4): ~400MB (always resident)
    ├── OS + KRIA: ~4GB
    └── Headroom: ~11GB
```

**Critical Rule:** Only ONE large model lives in VRAM at a time. The `GpuLeaseManager` (already built) orchestrates swaps.

### Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                    DUAL LLM BRAIN (6GB VRAM)                        │
│                                                                     │
│  ┌─────────────────────────┐  ┌─────────────────────────────────┐  │
│  │  ROUTER LLM (Always On) │  │  PLANNER LLM (On-Demand)        │  │
│  │                         │  │                                 │  │
│  │  Qwen2.5-0.5B Q4_K_M   │  │  Qwen2.5-7B-Instruct Q4_K_M    │  │
│  │  Running on: CPU threads │  │  Running on: GPU (6GB VRAM)     │  │
│  │  Via: ort / llama.cpp    │  │  Via: llama.cpp server           │  │
│  │  Memory: ~400MB RAM      │  │  Memory: ~4.5GB VRAM             │  │
│  │  Latency: <25ms          │  │  Latency: 200-500ms              │  │
│  │                         │  │                                 │  │
│  │  Tasks:                  │  │  Tasks:                          │  │
│  │  - Intent classification │  │  - Goal decomposition            │  │
│  │  - Tool selection        │  │  - Multi-step planning           │  │
│  │  - Simple Q&A            │  │  - Code generation               │  │
│  │  - Chitchat              │  │  - Reflection & replanning       │  │
│  │  - Hinglish routing      │  │  - Complex reasoning             │  │
│  └─────────────────────────┘  └─────────────────────────────────┘  │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  VRAM SWAP CONTROLLER (GpuLeaseManager — Already Built)     │   │
│  │                                                             │   │
│  │  Idle → Planner loads (4.5GB)                               │   │
│  │  Voice request → Planner evicts, TTS loads (200MB)          │   │
│  │  Image request → Planner evicts, Vision loads (3GB)         │   │
│  │  Task complete → Planner reloads (if pending tasks)         │   │
│  │                                                             │   │
│  │  Swap time: ~800ms (model already in RAM hot cache)         │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  Decision Flow:                                                     │
│  1. Simple task → Router LLM (CPU, <25ms, no VRAM used)            │
│  2. Complex task → Evict idle models → Load Planner (800ms)        │
│  3. Planner executes → Evict Planner → Reload previous model       │
└─────────────────────────────────────────────────────────────────────┘
```

### Strict Hardware Isolation Protocol (No Dynamic Swapping)

**Dynamic VRAM swapping is rejected.** It introduces catastrophic latency spikes and unpredictable behavior. Instead, models are **permanently locked** to specific hardware:

```text
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

**Critical Rule:** The Planner LLM is **permanently resident in VRAM**. It is NEVER evicted for TTS, routing, or background tasks. The cognitive loop stays hot for instantaneous multi-step reasoning.

```rust
pub enum VramResident {
    /// Always in VRAM unless explicitly evicted by user action
    Planner,          // Qwen2.5-7B Q4_K_M — ~4.5GB
    /// Only loaded when user explicitly requests vision/image
    VisionModel,      // Qwen2.5-VL — evicts Planner temporarily
    ImageGenerator,   // ComfyUI — evicts Planner temporarily
}

pub enum CpuResident {
    /// Always in System RAM — never evicted
    Router,           // Qwen2.5-0.5B — ~400MB
    Tts,              // Piper — ~200MB
    Vad,              // Silero — ~50MB
    Embeddings,       // FastEmbed — ~100MB
}
```

**Why this is better than dynamic swapping:**
- **Zero swap latency** — Planner is always ready (0ms cold start)
- **Predictable memory** — No surprise OOM from swap contention
- **Simpler code** — No eviction logic, no swap coordination
- **Voice latency** — TTS runs on CPU, never competes with Planner

**When Planner must be evicted (rare, user-invoked only):**
1. User attaches an image → Planner evicts, Vision loads (~800ms)
2. User requests image generation → Planner evicts, ComfyUI loads (~1200ms)
3. Task complete → Planner reloads (~800ms)
4. During eviction: cloud fallback (Gemini Flash) handles planning if needed

### Cloud Fallback (Zero VRAM Cost)

When VRAM is occupied by Vision/Image models:

| Fallback | Cost | Latency | Quality |
|----------|------|---------|---------|
| **Local Qwen2.5-7B Q4** | Free | 200-500ms | Excellent |
| **Google Gemini Flash** | Free tier (15 req/min) | ~500ms | Excellent |
| **Mistral Small API** | Free tier | ~400ms | Excellent |
| **Wait for VRAM swap** | Free | +800ms | Same as local |

### When to Use Which LLM

| Task Type | LLM | VRAM | Latency |
|-----------|-----|------|---------|
| "Check system health" | Router (0.5B, CPU) | 0ms | <25ms |
| "Send email to boss" | Router (0.5B, CPU) | 0ms | <25ms |
| "Make my VM faster" | Planner (7B, GPU) | 800ms swap | 200-500ms |
| "Debug this Python error" | Planner (7B, GPU) | 0ms (if loaded) | 200-500ms |
| "Set up a dev environment" | Planner (7B, GPU) | 800ms swap | 200-500ms |
| "What's the weather?" | Router (0.5B, CPU) | 0ms | <25ms |
| Complex + Vision active | Cloud fallback | 0ms | ~500ms |

### Free LLM Options (2026) — 6GB VRAM Compatible

| Model | Size | VRAM | Reasoning | Best For |
|-------|------|------|-----------|---------|
| **Qwen2.5-0.5B Q4** | 0.5B | ~400MB RAM (CPU) | Basic | Routing, tool selection |
| **Qwen2.5-7B Q4_K_M** | 7B | ~4.5GB VRAM | Excellent | Planning, code gen, reflection |
| **Phi-4-mini Q4** | 3.8B | ~2.5GB VRAM | Good | Fallback planner if 7B too large |
| **Google Gemini Flash** | Cloud | 0 VRAM | Excellent | Cloud fallback, free tier |
| **Mistral Small** | Cloud | 0 VRAM | Excellent | Cloud fallback, free tier |

## Future-Ready: Plugin & Extension Architecture

### Self-Extending Capability

KRIA should be able to **learn new capabilities** without code changes:

```
User: "I need to control my smart lights"
    ↓
KRIA: "I don't have a smart lights tool. Let me check what's available."
    ↓
Discovers: Philips Hue API (via MCP or REST)
    ↓
Generates: Tool definition from API docs
    ↓
Registers: New tool "control_lights"
    ↓
Executes: "Turn on living room lights" → ✅
```

### How: MCP + Dynamic Tool Generation

```
┌─────────────────────────────────────────────────────────┐
│                EXTENSIBILITY LAYER                        │
│                                                         │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │  MCP Servers │  │  REST APIs   │  │  CLI Tools   │  │
│  │  (discovered │  │  (auto-      │  │  (detected   │  │
│  │   at startup)│  │   documented)│  │   via PATH)  │  │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  │
│         └──────────────────┼──────────────────┘         │
│                            ↓                             │
│                   ┌──────────────┐                       │
│                   │ Tool Schema  │                       │
│                   │ Generator    │                       │
│                   │              │                       │
│                   │ API docs →   │                       │
│                   │ ToolDef      │                       │
│                   └──────────────┘                       │
└─────────────────────────────────────────────────────────┘
```

### Tech Stack for Extensibility

| Component | Technology | Status |
|-----------|-----------|--------|
| MCP Protocol | **MCP SDK** (already integrated) | ✅ Built |
| API Discovery | **OpenAPI/Swagger** auto-parsing | 🆕 Add |
| CLI Discovery | `which` + `--help` parsing | 🆕 Add |
| Tool Schema Gen | LLM generates ToolDef from docs | 🆕 Add |
| Plugin System | **Skill Manifest** (already designed) | ✅ Designed |

---

## Voice-First Intelligence

### Current Voice Pipeline (Already Built)

```
Mic → VAD → STT → Agent Loop → TTS → Speaker
```

### Enhanced Voice Intelligence

| Feature | Current | Enhancement |
|---------|---------|-------------|
| Wake word | ✅ "Hey Ria" | Keep |
| Barge-in | ✅ Built | Keep |
| Partial transcripts | ✅ Built | Use for speculative routing (Phase 4) |
| Multi-turn context | ❌ Missing | Phase 1 routing context |
| Voice memory | ❌ Missing | "Remember this" → stores in episodic memory |
| Proactive voice | ❌ Missing | "You have a meeting in 15 minutes" |

### Proactive Intelligence

```
Background thread (every 5 minutes):
    ↓
Check: Calendar → Upcoming meetings?
Check: System → CPU/RAM/Disk alerts?
Check: Email → Important unread?
Check: Tasks → Overdue reminders?
    ↓
If urgent: Voice nudge ("You have a meeting in 15 minutes")
If informational: Silent notification badge
```

---

## Implementation Roadmap

### Phase A: Uncertainty + Open Execution (Weeks 1-3)

| Week | Deliverable | Effort |
|------|-------------|--------|
| 1 | **Uncertainty Engine** — Belief graph, confidence scoring, evidence gathering | 4 days |
| 2 | **Open Shell tool** — LLM can run any command on local/VM | 3 days |
| 3 | **Code Interpreter** — LLM writes Python → VM executes → returns result | 4 days |

### Phase B: Tree-of-Thoughts Planning (Weeks 4-5)

| Week | Deliverable | Effort |
|------|-------------|--------|
| 4 | **ToT Planner** — 7B generates 3 paths, Rust scores and selects best | 5 days |
| 5 | **Reflection + Replanning** — Observe results, adjust plan, handle failures | 4 days |

### Phase C: Skill Compiler + Memory (Weeks 6-8)

| Week | Deliverable | Effort |
|------|-------------|--------|
| 6 | **Skill Compiler** — Extract patterns from successful plans, compile into tools | 5 days |
| 7 | **Task Pattern Memory** — Store successful plans, reuse them | 4 days |
| 8 | **Procedural Memory** — "How to do X" learned from experience | 4 days |

### Phase D: Event-Driven Perception (Weeks 9-10)

| Week | Deliverable | Effort |
|------|-------------|--------|
| 9 | **Event-driven system monitoring** — inotify + dbus + netlink (no polling) | 4 days |
| 10 | **Proactive nudges** — Calendar, system, email monitoring via events | 3 days |

### Phase E: Browser & Autonomous (Weeks 11-12)

| Week | Deliverable | Effort |
|------|-------------|--------|
| 11 | **Browser-Use integration** — Python sandbox for web tasks | 3 days |
| 12 | **Self-extending tools** — Auto-discover and register new capabilities | 5 days |

---

## Complete Technology Stack (All Free/Open Source)

### Core Infrastructure (Already Built in KRIA)

| Component | Technology | Stars | License |
|-----------|-----------|-------|---------|
| Backend | **Rust** + Tokio | - | - |
| Desktop | **Tauri** | 89k | MIT |
| Frontend | **SolidJS** + Vite | - | MIT |
| Voice STT | **Whisper.cpp** | 37k | MIT |
| Voice TTS | **Piper** | 5k | MIT |
| VAD | **Silero VAD** | 4k | MIT |
| Embeddings | **FastEmbed** (multilingual-e5-small) | 2k | Apache-2 |
| Database | **SQLite** | - | Public Domain |
| Vector DB | **ChromaDB** or built-in | 16k | Apache-2 |
| VM/QEMU | **QEMU** + SSH | - | GPL |
| MCP | **MCP SDK** | - | MIT |

### New Components to Add

| Component | Technology | Stars | License | Cost |
|-----------|-----------|-------|---------|------|
| Planner LLM | **Qwen2.5-7B-Instruct Q4_K_M** (~4.5GB VRAM, GGUF) | - | Apache-2 | Free |
| Plan-Execute | **Pure Rust** (in `loop_engine`, no Python) | - | - | Free |
| Browser Agent | **Browser-Use** (Python, ephemeral sandbox only) | 92.8k | MIT | Free |
| Code Execution | **QEMU VMs** (already have) | - | GPL | Free |
| Knowledge Graph | **Graphify** (already integrated) | - | MIT | Free |
| Skill Compiler | **Pure Rust** (pattern extraction + tool schema gen) | - | - | Free |
| Uncertainty Engine | **Pure Rust** (belief graph + confidence scoring) | - | - | Free |
| System Events | **inotify** + **dbus** + **netlink** (kernel-level, no polling) | - | - | Free |

### Optional Enhancements

| Component | Technology | Stars | License | Cost |
|-----------|-----------|-------|---------|------|
| Cloud LLM Fallback | **Google Gemini Flash** (free tier) | - | - | Free |
| Vision Model | **Qwen2.5-VL-7B** (local, VRAM-shared) | - | Apache-2 | Free |
| Fine-tuning | **Unsloth** (LoRA training) | 25k | Apache-2 | Free |
| UI Automation | **xdotool** + **xdg-open** (CLI, no VLA needed) | - | - | Free |

### Rejected Technologies (And Why)

| Technology | Why Rejected |
|-----------|-------------|
| **LangGraph** | Python sidecar adds IPC latency, memory duplication. KRIA is Rust-native. |
| **CrewAI** | Python framework, 6GB VRAM cannot support alongside 7B model. |
| **AutoGen/MAF** | Maintenance mode. Microsoft recommends MAF but it's Python-heavy. |
| **eBPF** | Overkill for local assistant. `inotify` + `dbus` + `netlink` cover 95% of use cases with zero kernel module risk. |
| **VLA (Vision-Language-Action)** | Too heavy for 6GB VRAM. `xdotool` + `xdg-open` handle UI automation at 0 VRAM cost. |

---

## Safety Architecture (Already Built + Enhanced)

### Existing Safety (Keep)

| Layer | What | Status |
|-------|------|--------|
| Risk Classification | Green/Yellow/Red/Black | ✅ Built |
| HITL Approval | Human approval for destructive actions | ✅ Built |
| PIN Guard | Typed PIN for sensitive operations | ✅ Built |
| Audit Log | 30-day retention, all actions logged | ✅ Built |
| Rollback | Snapshot-based rollback for file operations | ✅ Built |
| Blacklist | Blocked commands/paths | ✅ Built |
| Emergency Stop | "KRIA stop now" voice command | ✅ Built |

### Enhanced Safety for Autonomous Mode

| Enhancement | What | Priority |
|-------------|------|----------|
| **Step Approval** | Show plan to user before executing | P0 |
| **Dry Run** | "What would you do?" without executing | P1 |
| **Cost Estimation** | "This will take ~3 minutes and modify 2 files" | P1 |
| **Undo Plan** | Generate rollback plan before executing | P2 |
| **Sandbox Mode** | Execute in VM first, apply to host only on approval | P2 |

---

## Summary: What Makes This "True Intelligence"

| Current KRIA | After This Plan |
|-------------|-----------------|
| Routes to pre-coded tools | **Reasons about unknown problems with uncertainty awareness** |
| Single-path execution | **Tree-of-Thoughts: simulates 3 paths, picks best** |
| No learning from experience | **Skill Compiler: successful plans become reusable tools** |
| Can only do what's coded | **Can generate and execute arbitrary code** |
| Reactive polling (5-min cron) | **Event-driven perception (sub-ms kernel hooks)** |
| Fixed capability set | **Self-extending via MCP + API discovery + compiled skills** |
| Chatbot behavior | **Autonomous agent with human oversight** |
| Python orchestration | **Pure Rust cognitive loop (zero IPC latency)** |
| Binary execution (guess or fail) | **Uncertainty Engine (gather evidence or ask before acting)** |

### The Key Insights

1. **Intelligence = Observation + Reasoning + Self-Improvement.** The Skill Compiler is what makes KRIA genuinely smarter over time — not just storing logs, but compiling successful patterns into reusable tools.

2. **The 0.5B router should NEVER guess.** If uncertain, gather evidence (read-only commands) or ask the user. The 7B planner is only woken when confidence exceeds the threshold.

3. **Pure Rust for the brain, Python only for the hands.** The cognitive loop (route → assess uncertainty → plan → execute → compile skill) runs entirely in Rust. Python is confined to ephemeral tool sandboxes (Browser-Use, code interpreter scripts).

4. **Tree-of-Thoughts over linear planning.** A 7B model generating a single plan will hallucinate. Forcing it to generate and score 3 paths produces dramatically better outcomes with minimal extra latency.

5. **Event-driven over polling.** `inotify` + `dbus` + `netlink` give sub-millisecond system awareness without wasting CPU on cron jobs.

---

## Appendix A: Quick Reference Commands

```bash
# Install Planner LLM (Q4_K_M quantized for 6GB VRAM)
ollama pull qwen2.5:7b-instruct-q4_K_M

# Install Browser-Use (Python, ephemeral sandbox only)
pip install browser-use

# Install Graphify (knowledge graph)
pip install graphifyy

# All free, all open-source, all local-first
# Python frameworks (LangGraph, CrewAI, AutoGen) are NOT used
```

## Appendix B: System Prompt Template for Structured Branching

```
You are KRIA's planning engine. You MUST generate exactly 3 plans using these templates:

SYSTEM STATE:
{working_set}

WORLD MODEL FACTS:
{world_model_facts}

SELF-MODEL (tool success rates):
{self_model_stats}

USER GOAL: {goal}

Generate exactly 3 plans:

PATH A — DIAGNOSE-FIRST (read-only, gather information):
  Steps: [list exact commands]
  Predicted outcome: [what you'll learn]
  Risk: None (read-only)
  Confidence: [0.0-1.0]
  SelfModel score: [geometric mean of tool success rates]

PATH B — MINIMAL-RISK FIX (reversible changes):
  Steps: [list exact commands]
  Predicted outcome: [what will change]
  Risk: Low (reversible)
  Confidence: [0.0-1.0]
  SelfModel score: [geometric mean of tool success rates]

PATH C — AGGRESSIVE FIX (may be hard to reverse):
  Steps: [list exact commands]
  Predicted outcome: [what will change]
  Risk: High (potentially irreversible)
  Confidence: [0.0-1.0]
  SelfModel score: [geometric mean of tool success rates]

FAILURE CHECK: Do any of these paths match known failure patterns?
{failure_patterns}

SELECT: [A/B/C] because [reasoning based on risk, confidence, and SelfModel score]
```
