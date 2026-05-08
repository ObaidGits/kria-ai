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

### Tree-of-Thoughts Planning (Layer 3)

**Linear planning is abolished.** The 7B model generates and scores multiple paths before executing.

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
│  TREE-OF-THOUGHTS (3 parallel paths)                    │
│                                                         │
│  Path A: Reduce nginx workers (64→4)                    │
│    Score: 0.92 | Risk: Low | Impact: CPU 80%→12%       │
│                                                         │
│  Path B: Kill nginx, use caddy                          │
│    Score: 0.65 | Risk: High | Impact: Unknown           │
│                                                         │
│  Path C: Add more RAM via swap file                     │
│    Score: 0.71 | Risk: Medium | Impact: RAM +2GB        │
│                                                         │
│  Winner: Path A (highest score, lowest risk)            │
└─────────────────────────────────────────────────────────┘
    ↓
Execute Path A → Observe → CPU 12% → Goal achieved ✅
```

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

### The Skill Compiler Solution

When the ToT planner successfully resolves a novel task, the execution graph is abstracted and compiled into a **reusable tool schema**.

```
First time: "Make my VM faster"
    ↓
ToT Planner generates: [ssh top → ssh systemctl status nginx → ssh sed → ssh systemctl restart]
    ↓
Execution succeeds → CPU 80%→12%
    ↓
SKILL COMPILER:
    - Abstract: hardcoded IP → variable {target_host}
    - Abstract: "nginx" → variable {service_name}
    - Abstract: "64" → variable {config_key}
    ↓
Compiled Skill: optimize_service_workers(target_host, service_name, config_key, new_value)
    ↓
Registered as new tool schema in ToolRegistry
```

```
Next time: "Make my VM faster"
    ↓
0.5B Router matches intent → finds compiled skill "optimize_service_workers"
    ↓
Direct execution (50ms) → No 7B model needed → No VRAM swap → No LLM cost
```

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

### VRAM Swap Strategy

The existing `GpuLeaseManager` + `LlmEvictionController` already handle model swapping. The Planner LLM integrates as another GPU owner:

```rust
pub enum GpuOwner {
    LlmServer,           // Existing: main chat LLM
    VisionModel,         // Existing: Qwen2.5-VL
    TtsModel,            // Existing: Piper
    PlannerModel,        // NEW: Qwen2.5-7B planner
    ImageGenerator,      // Existing: ComfyUI
}
```

**Swap priority (highest wins VRAM):**
1. Emergency HITL (instant)
2. Active LLM inference
3. Planner (when complex task pending)
4. Vision (when image attached)
5. TTS (voice response)
6. ImageGenerator (background)

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

## Appendix B: System Prompt Template for ToT Planning

```
You are KRIA's planning engine. You have access to the following system state:

{belief_graph}

Available tools: {tool_descriptions}

User goal: {goal}

Generate exactly 3 distinct plans to achieve this goal. For each plan:
1. List the specific commands/steps
2. Predict the outcome
3. Assess the risk level (low/medium/high)
4. Assign a confidence score (0.0-1.0)

Format:
PLAN A: [description]
  Steps: [step1, step2, ...]
  Predicted outcome: [what happens]
  Risk: [low/medium/high]
  Confidence: [0.0-1.0]

PLAN B: ...
PLAN C: ...

SELECT: [A/B/C] because [reasoning]
```
