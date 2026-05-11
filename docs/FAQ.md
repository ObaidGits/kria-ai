# KRIA — Frequently Asked Questions & Design Queries

> **Last Updated:** 2026-05-11
> **Status:** Reference

---

## Table of Contents

1. [Is KRIA fully open source and free?](#q1--is-kria-fully-open-source-and-free)
2. [What features does KRIA have?](#q2--what-features-does-kria-have)
3. [Does KRIA support Windows and Linux?](#q3--does-kria-support-windows-and-linux)
4. [Is the platform multilingual?](#q4--is-the-platform-multilingual)
5. [Is there a memory or learning system?](#q5--is-there-a-memory-or-learning-system)
6. [What data is stored?](#q6--what-data-is-stored)
7. [How does GPU orchestration work?](#q7--how-does-gpu-orchestration-work)
8. [Can multiple LLM models be used?](#q8--can-multiple-llm-models-be-used)
9. [How can the project be enhanced?](#q9--how-can-the-project-be-enhanced)

---

## Q1 — Is KRIA fully open source and free?

**Answer: Yes — 100%. Every component is open source and free.**

### Core AI Models

| Component | License | Cost |
|---|---|---|
| Qwen2.5-VL-7B-Instruct (LLM) | Apache 2.0 | Free |
| Whisper.cpp (speech-to-text) | MIT | Free |
| Piper TTS (text-to-speech) | MIT | Free |
| Silero VAD (voice activity detection) | MIT | Free |
| FastEmbed (embeddings) | MIT | Free |

### Inference Engines

| Component | License | Cost |
|---|---|---|
| llama.cpp (LLM server) | MIT | Free |
| whisper.cpp (STT server) | MIT | Free |

### Infrastructure

| Component | License | Cost |
|---|---|---|
| Rust | MIT/Apache 2.0 | Free |
| Tokio | MIT | Free |
| SQLite | Public Domain | Free |
| Tauri v2 | MIT/Apache 2.0 | Free |

### Internet / Web Tools — No API Keys Required

| Data Source | Method | Cost |
|---|---|---|
| Web Search | DuckDuckGo HTML scraping | Free, no API key |
| Weather | wttr.in JSON API | Free, no API key |
| News | RSS feeds | Free, no API key |
| Content Extraction | trafilatura (Python sidecar) | Free |

**Bottom line: Zero paid APIs. Zero subscriptions. Zero cloud dependency.**

---

## Q2 — What features does KRIA have?

**Answer: KRIA is a complete AI Assistant with 60+ tools across 12 domains.**

### 🧠 Intelligent Conversational AI
- Natural language chat with local LLM (Qwen2.5-VL-7B)
- Multi-step reasoning via ReAct loop (Think → Act → Observe → Repeat)
- Conversation memory across sessions (SQLite)

### 🎙️ Voice Control
- "Hey KRIA" wake word — hands-free activation
- Real-time speech-to-text (Whisper)
- Natural-sounding voice responses (Piper TTS)
- Sub-500ms latency for simple commands

### 🌐 Internet Access
- Web search (DuckDuckGo — no API key)
- Fetch and extract content from web pages
- Download files with progress tracking
- Real-time weather, news headlines
- RSS/Atom feed reading

### 📄 Document Intelligence
- Read: PDF, DOCX, XLSX, CSV, Markdown, JSON
- Summarize via LLM
- Convert between formats
- OCR via Python sidecar
- RAG: Ingest documents into knowledge base

### 📁 File Management
- Read, write, copy, move, delete files
- Search files by name/pattern
- Directory monitoring
- Find large files, detect duplicates

### 💻 OS-Level System Control
- System info: CPU, RAM, disk, network, battery, GPU
- Services: List, start, stop system services
- Power: Shutdown, reboot, lock screen
- System config: Volume, brightness

### 📦 Application Management
- Search package repositories (apt, dnf, winget, brew)
- Install/uninstall applications
- Open, close, focus running applications

### ⚙️ Automation Engine
- YAML workflows — multi-step automated routines
- Cron-like scheduling
- Event triggers

### 🛡️ Safety System
- 4-tier risk classification: GREEN → YELLOW → RED → BLACK
- Human-in-the-loop: dangerous actions require approval
- Rollback: automatic backups before destructive actions
- Audit log: every action logged

### 🔌 Plugin System
- MCP servers for external tools
- OpenClaw skills (sandboxed Docker containers)
- Community-contributed capabilities

---

## Q3 — Does KRIA support Windows and Linux?

**Answer: Yes — both are first-class platforms.**

### Cross-Platform Strategy

| Operation | Linux | Windows |
|---|---|---|
| Open app | `xdg-open` | `start` / COM API |
| List/kill processes | `psutil` (cross-platform) | `psutil` (cross-platform) |
| Service management | `systemctl` | `Get-Service` |
| Volume/brightness | `pactl` / `brightnessctl` | WMI |
| Package install | `apt` / `dnf` | `winget` |
| Notifications | `notify-send` | Toast API |

### Identical on Both Platforms

- LLM reasoning → llama.cpp (cross-platform)
- Speech-to-text → whisper.cpp (cross-platform)
- TTS → Piper (cross-platform)
- File operations → Rust `std::fs`
- System info → `sysinfo` crate
- Database → SQLite (cross-platform)

---

## Q4 — Is the platform multilingual?

**Answer: Partially supported by the tech stack, English configured by default.**

### Current State by Component

| Component | Multilingual Capability | Configured |
|---|---|---|
| **STT (Whisper)** | 99 languages | English only |
| **LLM (Qwen2.5-VL)** | English, Chinese, Hindi, 20+ more | English only |
| **TTS (Piper)** | 30+ languages | `en_US-lessac` only |
| **Wake Word** | Language-independent | ✅ Works for all |

### How to Enable Multilingual

- **STT:** Change Whisper language config
- **LLM:** No changes needed — Qwen handles multiple languages
- **TTS:** Download additional Piper voice models (~65MB each)

---

## Q5 — Is there a memory or learning system?

**Answer: Yes — SQLite-backed memory with RAG integration.**

### Memory Architecture

| Memory Type | Technology | Purpose |
|---|---|---|
| **Episodic** | SQLite + FTS5 | Conversation history |
| **Semantic** | FastEmbed vectors | Similar conversation retrieval |
| **Document** | RAG engine | Document Q&A |
| **Working** | In-memory | Current session context |

### Fact Store

| Tool | Purpose |
|---|---|
| `remember_fact` | Store user facts |
| `recall_fact` | Retrieve stored facts |
| `search_knowledge` | Semantic search |

---

## Q6 — What data is stored?

**Answer: All data stays local.**

### SQLite Database (`~/.kria/kria.db`)

| Table | Data Stored |
|---|---|
| `audit_log` | Every tool call — timestamp, action, risk level, result |
| `conversations` | Chat history |
| `facts` | User-stored facts |
| `preferences` | User preferences |

### Filesystem

| Location | Data |
|---|---|
| `~/.kria/logs/` | JSON logs (daily rotation) |
| `~/.kria/config.toml` | User configuration |
| `~/.kria/rollback/` | File backups (72-hour retention) |

### What Is NOT Stored

- ❌ No telemetry sent externally
- ❌ No cloud backup
- ❌ No file contents sent over network (unless user requests)
- ❌ No credentials stored

---

## Q7 — How does GPU orchestration work?

**Answer: Dynamic VRAM management via Hardware Orchestrator.**

### GPU Backends

| Platform | Backend | Dynamic Offloading |
|----------|---------|-------------------|
| Linux/Windows + NVIDIA | Cuda | Full VRAM-based |
| macOS (Apple Silicon) | Metal | Static (all layers) |
| No discrete GPU | CpuOnly | N/A |

### VRAM Thresholds

| Threshold | Default | Action |
|-----------|---------|--------|
| `yield_threshold_mb` | 512 | Start offloading layers |
| `emergency_threshold_mb` | 128 | Immediate CPU fallback |
| `recover_threshold_mb` | 2048 | Add layers back |

### Degradation Levels

| Level | GPU Layers | Context |
|-------|------------|---------|
| Full | All | Max |
| ReducedContext | All | Reduced |
| PartialOffload | Partial | Reduced |
| HeavyOffload | Minimal | Minimal |
| CPU | 0 | Minimal |

---

## Q8 — Can multiple LLM models be used?

**Answer: Yes — via model routing.**

### Model Routing

```
User Command
    │
    ▼
Intent Router
    │
    ├── Simple ("open Chrome") → Direct tool call (no LLM)
    ├── Medium ("search web") → Fast local model
    └── Complex ("analyze PDF") → Full reasoning model
```

### Available Models

| Model | VRAM | Purpose |
|---|---|---|
| Qwen2.5-VL-7B Q4_K_M | ~4.7 GB | Primary reasoning, vision |
| FastEmbed | ~270 MB | Embeddings |

---

## Q9 — How can the project be enhanced?

### Priority 1 — Multi-Language Voice

- **Effort:** Low — change Whisper config, download Piper voice
- **Impact:** High for international users

### Priority 2 — Vision Enhancements

- Screenshot analysis
- Screen content understanding
- Image Q&A

### Priority 3 — Telegram Bot Interface

- Control KRIA from phone
- ~100 lines of code

### Priority 4 — Natural Language Workflow Creation

- LLM generates YAML workflows from plain English
- Self-programming capability

### Other Enhancements

| Enhancement | Effort | Impact |
|---|---|---|
| Git integration tools | Low | Developer-friendly |
| Universal search | Medium | Power feature |
| Dashboard analytics | Medium | Visual appeal |
| Performance benchmarking | Medium | Evaluation gold |

---

## Related Documentation

- **ARCHITECTURE.md** — System architecture
- **DEVELOPMENT.md** — Development guide
- **VOICE.md** — Voice pipeline details
- **MEMORY.md** — Memory system
