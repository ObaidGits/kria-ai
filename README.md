<div align="center">

# 🤖 K.R.I.A.

### **Kernel Responsive Intelligent Agent**

**A Local-First AI Desktop Assistant with Voice Control, Memory, Safety & Extensible Skills**

[![Rust](https://img.shields.io/badge/Built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
[![Tauri](https://img.shields.io/badge/Framework-Tauri-blue?logo=tauri)](https://tauri.app/)
[![License](https://img.shields.io/badge/License-Apache%202.0-green.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey)]()

---

*An AI-driven desktop productivity agent that automates workflows through contextual reasoning, natural interaction, and adaptive decision-making — all running locally on your machine.*

[Features](#-features) • [Architecture](#-architecture) • [Installation](#-installation) • [Documentation](#-documentation) • [Contributing](#-contributing)

</div>

---

## 📋 Overview

K.R.I.A. is an intelligent desktop assistant that transforms how you interact with your computer. It combines **conversational AI**, **workflow automation**, and **context-aware execution** into a unified platform — prioritizing **privacy**, **local-first operation**, and **extensibility**.

### Why K.R.I.A?

| 🏠 **Local-First** | 🔒 **Privacy-Focused** | 🧩 **Extensible** | 🎙️ **Voice-Native** |
|:---:|:---:|:---:|:---:|
| Runs entirely on your machine | Your data never leaves your device | Install skills from ClawHub marketplace | Hands-free voice interaction |

---

## ✨ Features

### 🧠 Intelligent Agent Core

| Feature | Description |
|---------|-------------|
| **ReAct Loop Engine** | Multi-step reasoning with tool orchestration |
| **Intent Classification** | ONNX-powered fast intent routing |
| **Semantic Tool Injection** | Context-aware tool selection based on relevance |
| **Uncertainty Quantification** | Confidence-based decision making |
| **Self Model** | Tracks tool success rates and adapts strategies |
| **World Model** | Persistent system facts across sessions |
| **Failure Analyzer** | Pattern matching for error recovery |
| **Curiosity Loop** | Autonomous investigation of anomalies |

### 🛠️ 60+ Native Tools

| Category | Tools |
|----------|-------|
| **System** | CPU, memory, disk, GPU, network, battery, uptime, health checks |
| **Files** | Read, write, copy, move, delete, search, organize |
| **Documents** | PDF parsing, DOCX conversion, chunking, RAG queries |
| **Internet** | Web search, URL fetch, downloads, API calls |
| **Applications** | Launch, install, uninstall, switch, manage |
| **Power** | Lock, sleep, shutdown, reboot, power plans |
| **Packages** | Install, update, remove (apt, dnf, pacman, brew) |
| **Processes** | List, kill, monitor, manage |
| **Knowledge** | Facts, snippets, document search, memory queries |
| **Automation** | Schedules, macros, workflows, proactive triggers |
| **Communication** | Telegram bridge, notifications, clipboard |
| **Developer** | Code execution, shell commands, git operations |

### 🔧 OpenClaw Skill Substrate

Extend K.R.I.A. with sandboxed skills from the ClawHub marketplace:

| Feature | Details |
|---------|---------|
| **Skill Marketplace** | Browse and install community skills |
| **Docker Isolation** | Each skill runs in a sandboxed container |
| **Capability Control** | Skills declare what they can access |
| **Trust Tiers** | Community → Verified → Partner → Internal |
| **Audit Ledger** | Tamper-proof HMAC-signed invocation logs |
| **Container Pool** | Warm containers for <100ms skill latency |

```
┌─────────────────────────────────────────────────────────────┐
│  User Request: "Summarize this PDF document"                │
└──────────────────────┬──────────────────────────────────────┘
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  CapabilityResolver matches → pdf-analyzer skill            │
└──────────────────────┬──────────────────────────────────────┘
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  ContainerPool.acquire() → Warm Docker container            │
└──────────────────────┬──────────────────────────────────────┘
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  Skill executed in isolated container with PDF mounted      │
└──────────────────────┬──────────────────────────────────────┘
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  AuditLedger records invocation with HMAC signature         │
└─────────────────────────────────────────────────────────────┘
```

### 🛡️ Safety & Security

| Layer | Protection |
|-------|------------|
| **Risk Classification** | GREEN / YELLOW / RED / BLACK tiers |
| **HITL Gateway** | Human-in-the-loop approval for risky actions |
| **Audit Logging** | Every action recorded with parameters hash |
| **Rollback Manager** | Snapshots before destructive operations |
| **PIN Guard** | Sensitive operations require PIN |
| **Blacklist** | Hard-blocked dangerous actions |
| **Quarantine** | Dynamic tool isolation pending approval |

### 🎙️ Voice Pipeline

| Component | Technology |
|-----------|-------------|
| **STT** | Whisper (faster-whisper, distil-whisper) |
| **TTS** | Piper, Coqui TTS |
| **VAD** | WebRTC VAD, Silero VAD |
| **Wake Word** | Porcupine, custom models |
| **AEC** | Acoustic Echo Cancellation (v2) |
| **Streaming** | Real-time transcription with sentence splitting |

### 🖼️ Image Generation

| Backend | Features |
|---------|----------|
| **ComfyUI** | Local GPU-accelerated generation |
| **Cloud Fallback** | OpenAI DALL-E, Stability AI |
| **Progress Streaming** | Real-time generation progress |
| **Prompt Enhancement** | Automatic prompt optimization |
| **Style Library** | Predefined artistic styles |

### 🌐 Fleet & Remote Execution

| Capability | Description |
|------------|-------------|
| **Target Enrollment** | Register remote VMs/servers |
| **SSH Execution** | Run commands on enrolled targets |
| **Inventory Pooling** | Reusable target connections |
| **QoS Scheduling** | Adaptive transport quality |
| **Snapshot Orchestration** | VM state management |
| **Connection Control** | Signed lease management |

### 📚 Memory & Knowledge

| Store | Purpose |
|-------|---------|
| **Conversations** | Chat history with context |
| **Facts** | User facts with decay scoring |
| **Document Chunks** | RAG-indexed documents |
| **Vector Index** | Semantic search embeddings |
| **Audit Log** | Action history for compliance |
| **World Model** | Persistent system state |

### 🔌 MCP (Model Context Protocol)

| Feature | Details |
|---------|---------|
| **Server Management** | Start/stop MCP servers |
| **Tool Discovery** | Auto-discover MCP tools |
| **Payload Shaping** | Bridge MCP tools to KRIA tool registry |
| **Settings UI** | Configure MCP servers in settings |

---

## 🏗️ Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           KRIA ARCHITECTURE                             │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐                 │
│  │  SolidJS UI │◄──►│   Tauri     │◄──►│  kria-core  │                 │
│  │  (Frontend) │    │  Desktop    │    │  (Sovereign)│                 │
│  └─────────────┘    └─────────────┘    └──────┬──────┘                 │
│                                                │                         │
│         ┌──────────────────────────────────────┼──────────────────┐     │
│         │                                      │                  │     │
│         ▼              ▼              ▼         ▼         ▼        ▼     │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐ ┌────────┐ ┌────────┐ ...   │
│  │  Agent   │  │  Tools   │  │  Safety  │ │  LLM   │ │ Memory │       │
│  │  Loop    │  │ Registry │  │  Policy  │ │ Router │ │ Store  │       │
│  └──────────┘  └──────────┘  └──────────┘ └────────┘ └────────┘       │
│         │              │              │          │          │           │
│         └──────────────┴──────────────┴──────────┴──────────┘           │
│                                   │                                     │
│         ┌─────────────────────────┼─────────────────────────┐          │
│         ▼            ▼            ▼            ▼            ▼          │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌─────────┐  │
│  │ OpenClaw │  │   Voice  │  │  Image   │  │   MCP    │  │  Fleet  │  │
│  │ Substrate│  │ Pipeline │  │ Orchestr.│  │  Client  │  │ Control │  │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘  └─────────┘  │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Tech Stack

| Layer | Technology |
|-------|------------|
| **Core Runtime** | Rust (kria-core, kria-desktop, kria-server) |
| **Desktop Framework** | Tauri v2 |
| **Frontend** | SolidJS + TypeScript + TailwindCSS |
| **Database** | SQLite (conversations, facts, audit) |
| **Vector Search** | FastEmbed, local vector index |
| **LLM** | llama.cpp server, OpenAI-compatible API |
| **Voice** | Whisper, Piper, WebRTC VAD |
| **Image** | ComfyUI, Stable Diffusion |
| **Skills** | Docker, OpenClaw substrate |
| **Remote** | SSH, QEMU, connection-control |

---

## 🚀 Installation

### Prerequisites

- **Rust** 1.75+ 
- **Node.js** 18+
- **Python** 3.10+ (for sidecar)
- **Docker** (optional, for OpenClaw skills)
- **CUDA** (optional, for GPU acceleration)

### Quick Start

```bash
# Clone the repository
git clone https://github.com/ObaidGits/kria-ai.git
cd kria-ai

# Install dependencies
npm install

# Build the application
cargo build --release

# Run KRIA
cargo run --release
```

### First Run Setup

1. Launch KRIA
2. Complete the **Setup Wizard**:
   - Hardware detection
   - Model download (or use existing)
   - Sidecar setup
   - Voice configuration
3. Start chatting or enable voice mode

---

## 📖 Documentation

| Document | Purpose |
|----------|---------|
| [Architecture](docs/ARCHITECTURE.md) | Detailed system architecture |
| [Tools Guide](docs/TOOLS.md) | All 60+ tools documentation |
| [OpenClaw](docs/OPENCLAW.md) | Skill substrate integration |
| [Safety Model](docs/SAFETY.md) | Risk classification & HITL |
| [Development](docs/DEVELOPMENT.md) | Build, test, contribute |
| [FAQ](docs/FAQ.md) | Frequently asked questions |
| [Voice](docs/VOICE.md) | Voice pipeline details |
| [Memory](docs/MEMORY.md) | Memory & RAG system |
| [Hardware](docs/HARDWARE.md) | GPU orchestration |
| [Deployment](docs/DEPLOYMENT.md) | Packaging & distribution |

### AI Context Files

For AI assistants working on this codebase:

| File | Purpose |
|------|---------|
| [PROJECT_CONTEXT.md](ai-context/PROJECT_CONTEXT.md) | Tech stack, subsystems, constraints |
| [CODE_GRAPH_INDEX.md](ai-context/CODE_GRAPH_INDEX.md) | Entry points, dependencies, types |
| [CORE_CONTEXT.md](ai-context/CORE_CONTEXT.md) | Authoritative runtime behavior |
| [AI_RULES.md](ai-context/AI_RULES.md) | Rules for AI working on KRIA |

---

## 🧪 Testing

KRIA includes comprehensive test suites:

```bash
# Rust unit tests
cargo test

# Frontend tests
cd ui && npm test

# E2E tests (Playwright)
cd tests/e2e && npm test

# OpenClaw integration tests
cargo test --test openclaw_integration
```

See [TestPrompts.txt](TestPrompts.txt) and [VMTestPrompts.txt](VMTestPrompts.txt) for manual testing scenarios.

---

## 🤝 Contributing

We welcome contributions! Please see:

1. [Development Guide](docs/DEVELOPMENT.md)
2. [Architecture Decision Records](docs/ADR/)
3. [Code of Conduct](CODE_OF_CONDUCT.md)

### Ways to Contribute

- 🐛 **Report bugs** via GitHub Issues
- 💡 **Suggest features** via GitHub Discussions
- 📝 **Improve documentation**
- 🔧 **Submit pull requests**
- 🧩 **Create OpenClaw skills** for the community

---

## 📜 License

KRIA is licensed under the **Apache 2.0 License**.

```
Copyright 2024-2026 ObaidGits

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
```

---

## 🙏 Acknowledgments

- Built with [Tauri](https://tauri.app/) — smaller, faster, secure apps
- Powered by [llama.cpp](https://github.com/ggerganov/llama.cpp) — efficient LLM inference
- Voice by [Whisper](https://github.com/openai/whisper) & [Piper](https://github.com/rhasspy/piper)
- Image generation via [ComfyUI](https://github.com/comfyanonymous/ComfyUI)

---

<div align="center">

**[⬆ Back to Top](#-kria)**

Made with ❤️ by [ObaidGits](https://github.com/ObaidGits)

</div>