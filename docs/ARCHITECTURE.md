# KRIA Architecture

> **Last Updated:** 2026-05-11
> **Status:** Production

---

## Executive Summary

KRIA (Knowledgeable Responsive Intelligent Assistant) is a local-first autonomous AI assistant built with Rust at its core. The architecture follows the **Sovereign-Orchestrator Principle**: Rust owns planning, safety, memory authority, resource allocation, and audit boundaries. All external systems (Python sidecar, MCP servers, OpenClaw skills, cloud APIs) are execution engines that receive sanitized input and return structured output.

---

## System Overview

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                            KRIA Workspace                                    │
│                                                                              │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────────────────────────┐ │
│  │ kria-desktop │  │ kria-server  │  │           kria-core                │ │
│  │  (Tauri v2)  │  │  (Axum)      │  │      (Sovereign Core)             │ │
│  │              │  │              │  │                                    │ │
│  │ • Window     │  │ • HTTP API   │  │ • Agent engine (ReAct loop)       │ │
│  │ • Tray icon  │  │ • WebSocket  │  │ • LLM inference (local + cloud)   │ │
│  │ • IPC bridge │  │ • Auth layer │  │ • Tool system (60+ tools)        │ │
│  │ • Auto-start │  │ • Static UI  │  │ • Memory & knowledge (SQLite)    │ │
│  │ • Installer  │  │ • Multi-user │  │ • Safety & HITL                  │ │
│  └──────┬───────┘  └──────┬───────┘  │ • Sidecar bridge (→ Python)      │ │
│         │                  │          │ • Plugin runtime                │ │
│         └──────────────────┴──────────┤ • OpenClaw skill substrate      │ │
│                 depends on            └──────────────┬─────────────────┘ │
│                                                      │                     │
│                                                      │ JSON-RPC / msgpack  │
│                                                      │ over stdio          │
│                                                      ▼                     │
│  ┌───────────────────────────────────────────────────────────────────────┐ │
│  │                     kria-modules (Python Sidecar)                      │ │
│  │                    "Pre-Cognitive Processing Layer"                    │ │
│  │                                                                       │ │
│  │  • Image processing (OpenCV, Pillow, Tesseract)                      │ │
│  │  • Document extraction (PyMuPDF, python-docx, pandas)                │ │
│  │  • Embeddings & RAG (sentence-transformers, chunking)                │ │
│  │  • Code analysis (tree-sitter, ast)                                  │ │
│  │  • Web extraction (readability, trafilatura)                         │ │
│  │  • Audio preprocessing (librosa, webrtcvad)                          │ │
│  │                                                                       │ │
│  │  Managed by: uv (virtual environments, per-plugin isolation)          │ │
│  └───────────────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## Component Map

```text
UI / Voice / Telegram / Server Transport
        |
        v
TurnAdmission
  |-- per-session turn gate
  |-- bounded queue
  |-- stale-turn invalidation
        |
        v
TurnGate  (SOLE TOP-LEVEL PLANNER)
  |-- deterministic guards
  |-- FastEmbed semantic router
  |-- optional ONNX classifier
  |-- validator
  |-- resource planner
        |
        v
Resource Plane
  |-- GpuLeaseManager
  |-- ResourceTelemetry / ResourceSnapshot
  |-- L1 Residency Manager
  |-- ImageBackend registry
        |
        v
Execution Plane
  |-- AgentLoop ReAct executor
  |-- ToolRegistry
  |-- PolicyEngine / HITL / Audit
  |-- Python sidecar
  |-- MCP clients
  |-- OpenClaw skill substrate
        |
        v
MemoryManager
  |-- facts
  |-- turns
  |-- media
  |-- snippets
  |-- document chunks
  |-- preferences
```

---

## Non-Negotiable Invariants

1. **Rust owns planning, safety, memory authority, resource allocation, and audit boundaries.**
2. **`TurnGate` is the only top-level planner.**
3. **`AgentLoop` is not allowed to allocate hardware, decide global route class, or override TurnGate resource plans.**
4. **No GPU-consuming component runs without a `GpuLease`.**
5. **L0 (local classifier) cannot authorize actions, tools, memory writes, or GPU allocation.**
6. **Tool execution always flows through `ToolRegistry`, `PolicyEngine`, HITL where required, and audit.**
7. **Python sidecar, ComfyUI, MCP servers, and OS tools are execution engines only.**
8. **Memory writes go through `MemoryManager`; `MemoryStore` is private persistence.**
9. **Every turn has a root cancellation token and bounded admission.**
10. **Telemetry reconciliation, not logical state alone, decides GPU recovery and degraded mode.**

---

## Crate Structure

### `kria-core` — Sovereign Core

The heart of KRIA. All agent logic, tool execution, memory management, and safety enforcement.

| Module | Responsibility |
|--------|----------------|
| `agent/` | TurnGate, AgentLoop, IntentRouter, Planner |
| `tools/` | 60+ native Rust tools, ToolRegistry, handlers |
| `safety/` | PolicyEngine, RiskLevel classification, HITL gateway |
| `memory/` | MemoryManager, MemoryStore (SQLite), RAG engine |
| `llm/` | ModelRouter, local/cloud LLM orchestration |
| `image/` | ImageBackend trait, ComfyUI integration |
| `voice/` | STT, TTS, wake word detection |
| `openclaw/` | Skill substrate, ContainerPool, ClawHub client |
| `infra/` | Sidecar bridge, isolation, environment providers |

### `kria-desktop` — Tauri Application

Desktop GUI with SolidJS frontend. Exposes kria-core via Tauri commands.

| Module | Responsibility |
|--------|----------------|
| `commands/` | Tauri IPC commands (chat, voice, tools, settings) |
| `tray/` | System tray icon and menu |
| `updater/` | Auto-update mechanism |

### `kria-server` — HTTP/WebSocket Server

Optional headless server mode for remote access.

| Module | Responsibility |
|--------|----------------|
| `routes/` | REST API endpoints |
| `ws/` | WebSocket streaming |
| `auth/` | Authentication layer |

### `kria-eval` — Evaluation Harness

Test framework for measuring agent quality.

| Module | Responsibility |
|--------|----------------|
| `runner/` | Eval case execution |
| `judge/` | LLM-based evaluation |
| `report/` | Report generation |

---

## Control Flow

### Turn Lifecycle

```
1. User input → TurnAdmission
2. TurnGate classifies intent (deterministic → semantic → classifier → validator)
3. ResourcePlan created (ReflexRust | ToolOnly | L1Text | L1Vision | ImageGeneration | MixedPipeline)
4. GPU lease requested if needed
5. AgentLoop executes within ResourcePlan bounds
6. Tool calls → ToolRegistry → PolicyEngine → (HITL if Red) → Execute
7. Results → MemoryManager (if persistent)
8. Response emitted to UI
```

### Tool Execution Flow

```
ToolRegistry.get_def(name)
    → PolicyEngine.evaluate(tool, params)
    → HITL.approve() if Red tier
    → ToolHandler.execute(params)
    → AuditLogger.record()
    → Result to AgentLoop
```

---

## Resource Plan Types

```rust
pub enum ResourcePlan {
    ReflexRust,                    // Deterministic, no LLM
    ToolOnly,                      // Tools only, no reasoning
    SidecarCpu,                    // Python sidecar processing
    L1Text { residency },          // Local LLM text inference
    L1Vision { visual_budget },    // Local LLM vision inference
    ImageGeneration { backend },   // ComfyUI or other backend
    MixedPipeline { stages },      // Multi-stage pipeline
    Clarify,                       // Ask user for clarification
    Refuse,                        // Safety refusal
}
```

---

## GPU Lease State Machine

```rust
pub enum GpuLeaseState {
    Idle,
    Held { owner, token, turn_id, deadline },
    Recovering { owner, reason },
    Degraded { reason },
}

pub enum GpuOwner {
    L1Worker,           // Local LLM inference
    ImageBackend,       // ComfyUI / image generation
    Vision,             // Vision model
    Speech,             // STT/TTS GPU acceleration
    Maintenance,        // System maintenance tasks
}
```

**Lease Rules:**
- L1 inference requires a lease when GPU-resident
- ComfyUI requires a lease for generation
- Vision GPU paths require a lease
- Lease release is not trusted until telemetry reconciliation passes

---

## Cancellation Tree

Every accepted turn receives a root cancellation token with mandatory child tokens:

```rust
pub struct TurnCancellationTree {
    pub root: CancellationToken,
    pub l0: CancellationToken,      // Classifier
    pub l1: CancellationToken,      // LLM inference
    pub tools: CancellationToken,   // Tool execution
    pub sidecar: CancellationToken, // Python sidecar
    pub mcp: CancellationToken,     // MCP servers
    pub image: CancellationToken,   // Image generation
}
```

Canceling the root propagates to every child, ensuring no zombie work.

---

## Trait Boundaries

Core traits for modularity and future extensibility:

| Trait | Purpose | Implementation |
|-------|---------|----------------|
| `ImageBackend` | Image generation abstraction | ComfyUI, future: sd.cpp, cloud |
| `MemoryManager` | Memory write authority | SQLite-backed store |
| `L1Runtime` | Local LLM lifecycle | llama-server orchestrator |
| `ResourceTelemetry` | Hardware monitoring | NVML, system stats |

---

## Hardware Constraints

Current target platform:
- **GPU:** NVIDIA RTX 4050 Laptop (6GB VRAM)
- **RAM:** ~16GB system RAM
- **Storage:** SSD recommended for SQLite and model files

The architecture is designed to scale to:
- Multi-GPU setups
- Cloud fallback providers
- Multi-agent federation

---

## Related Documentation

- **TOOLS.md** — Tool system architecture and development guide
- **OPENCLAW.md** — OpenClaw skill integration
- **SAFETY.md** — Safety model, policy engine, HITL
- **MEMORY.md** — Memory and knowledge systems
- **HARDWARE.md** — GPU/VRAM orchestration
- **DEVELOPMENT.md** — Build, test, and development workflow
