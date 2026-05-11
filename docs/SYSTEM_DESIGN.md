# KRIA — System Design Document

> **Last Updated:** 2026-05-11
> **Status:** Reference
> **Developer:** Obaidullah Zeeshan

---

## Executive Summary

KRIA (Kernel-Responsive Intelligent Agent) is a locally-hosted, voice-controlled AI assistant designed as a complete operating system companion. Unlike cloud-dependent assistants, KRIA runs entirely on-device, ensuring zero data exfiltration, zero subscription costs, and sub-500ms voice-loop latency.

### Key Differentiators

- **Fully local** — No cloud dependency, no API keys, no telemetry
- **Rust core** — Sovereign orchestrator with memory-safe execution
- **Agentic** — Multi-step task planning via ReAct loop
- **Safe** — Four-tier risk classification with human-in-the-loop
- **Extensible** — MCP servers, OpenClaw skills, plugin architecture
- **Fast** — Under 500ms for simple voice commands

---

## Design Principles

| Principle | Implementation |
|---|---|
| **Local-First** | All models, data, processing on-device. Internet only for user-requested operations. |
| **Sovereign Core** | Rust owns planning, safety, memory authority, resource allocation, audit boundaries. |
| **Fail-Safe by Default** | Dangerous operations blocked unless approved. Rollback points before destructive actions. |
| **Modular & Pluggable** | Each subsystem behind standard interfaces. Swap components independently. |
| **Resource-Aware** | Dynamic VRAM orchestration prevents OOM. Models loaded/unloaded on demand. |
| **Latency-Obsessed** | Every pipeline stage benchmarked. Streaming STT, LLM tokens, TTS. |

---

## High-Level Architecture

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
│  └──────────────┘  └──────────────┘  │ • Sidecar bridge (→ Python)      │ │
│                                      │ • OpenClaw skill substrate      │ │
│                                      └────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────────────────┘
```

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

---

## Module Overview

### Module 1 — Reasoning Brain

| Component | Technology | Purpose |
|-----------|------------|---------|
| TurnGate | Rust | Intent classification, resource planning |
| AgentLoop | Rust | ReAct execution loop |
| LLM Router | Rust | Local/cloud model routing |
| Context Manager | Rust | Sliding window + RAG |

### Module 2 — Sensory Pipeline

| Component | Technology | Purpose |
|-----------|------------|---------|
| VAD | webrtc-audio-processing | Voice activity detection |
| STT | whisper.cpp | Speech to text |
| TTS | Piper | Text to speech |
| Wake Word | Custom detector | Hands-free activation |

### Module 3 — Execution Layer

| Component | Technology | Purpose |
|-----------|------------|---------|
| ToolRegistry | Rust | 60+ native tools |
| PolicyEngine | Rust | Risk classification, HITL |
| AuditLedger | Rust/SQLite | Append-only audit trail |
| OpenClaw | Docker | Sandboxed skill execution |
| MCP Bridge | Rust | External tool servers |

### Module 4 — Internet Layer

| Component | Technology | Purpose |
|-----------|------------|---------|
| Web Search | DuckDuckGo | No-key web search |
| Content Extractor | trafilatura | Web page extraction |
| Download Manager | httpx | File downloads |
| RSS Engine | feedparser | Feed aggregation |

### Module 5 — File Intelligence

| Component | Technology | Purpose |
|-----------|------------|---------|
| Document Parser | PyMuPDF, python-docx | PDF, DOCX, XLSX parsing |
| Document Converter | pandoc | Format conversion |
| RAG Engine | FastEmbed + SQLite | Document Q&A |

### Module 6 — OS Control

| Component | Technology | Purpose |
|-----------|------------|---------|
| Service Manager | systemctl / sc.exe | Service control |
| Task Scheduler | cron / Task Scheduler | Scheduled tasks |
| Power Manager | systemctl / shutdown | Power management |
| Network Manager | nmcli / netsh | Network config |

### Module 7 — Safety System

| Component | Technology | Purpose |
|-----------|------------|---------|
| Risk Classifier | Rust | GREEN/YELLOW/RED/BLACK tiers |
| HITL Gateway | Rust | Human-in-the-loop approval |
| Rollback Manager | Rust | Backup/restore |
| Audit Logger | SQLite | Tamper-proof log |

---

## Technology Stack

### Rust Core

| Crate | Purpose |
|-------|---------|
| Tokio | Async runtime |
| Serde | Serialization |
| Reqwest | HTTP client |
| Rusqlite | SQLite storage |
| Tracing | Structured logging |
| FastEmbed | Semantic embeddings |
| NVML | GPU telemetry |

### Python Sidecar

| Package | Purpose |
|---------|---------|
| Pillow | Image processing |
| PyMuPDF | PDF parsing |
| python-docx | DOCX parsing |
| sentence-transformers | Embeddings |
| trafilatura | Web extraction |

### Frontend

| Technology | Purpose |
|------------|---------|
| Tauri v2 | Desktop shell |
| SolidJS | UI framework |
| TypeScript | Type layer |
| Vite | Build tool |

---

## Latency Budget

| Stage | Target | Typical |
|-------|--------|---------|
| Wake word | < 100ms | 50ms |
| STT transcription | < 500ms | 200-400ms |
| Intent routing | < 50ms | 10-30ms |
| LLM reasoning | < 2s | 500ms-2s |
| Tool execution | < 500ms | 50-200ms |
| TTS synthesis | < 300ms | 100-200ms |
| **Total (simple)** | **< 500ms** | **300-400ms** |
| **Total (complex)** | **< 3s** | **1-3s** |

---

## Resource Constraints

### Target Hardware

- **GPU:** NVIDIA RTX 4050 Laptop (6GB VRAM)
- **RAM:** 16GB system RAM
- **Storage:** SSD for SQLite and models

### VRAM Budget

| Component | VRAM |
|-----------|------|
| LLM (Qwen2.5-VL-7B Q4_K_M) | 4-5 GB |
| Whisper medium.en | 1.5 GB |
| CUDA overhead | 0.5 GB |
| **Total** | 6-7 GB |

Dynamic offloading manages VRAM pressure via Hardware Orchestrator.

---

## Deployment

| Method | Platforms |
|--------|-----------|
| Tauri desktop | Linux, macOS, Windows |
| Standalone server | Linux (headless) |
| Docker | Linux, WSL2 |

---

## Future Roadmap

| Version | Features |
|---------|----------|
| v1.1 | Vision enhancements, multi-language voice |
| v1.2 | Streaming ASR, Telegram bot |
| v2.0 | Multi-device mesh, cloud sync (optional) |

---

## Related Documentation

- **ARCHITECTURE.md** — Detailed architecture
- **TOOLS.md** — Tool system guide
- **SAFETY.md** — Safety model
- **DEVELOPMENT.md** — Development guide
- **FAQ.md** — Frequently asked questions
