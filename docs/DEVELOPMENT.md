# KRIA Development Guide

> **Last Updated:** 2026-05-11
> **Status:** Production

---

## Quick Start

```bash
# One-time setup
bash scripts/setup.sh

# Install frontend dependencies
cd ui && npm install && cd ..

# Run development mode
cargo tauri dev --features nvidia
```

---

## Prerequisites

### System Dependencies (Ubuntu/Debian)

```bash
sudo apt update && sudo apt install -y \
  build-essential pkg-config \
  libssl-dev \
  libasound2-dev \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  librsvg2-dev \
  patchelf
```

### Rust Toolchain

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

### Node.js

```bash
# Using fnm (recommended)
curl -fsSL https://fnm.vercel.app/install | bash

# Or via apt
sudo apt install nodejs npm
```

### Tauri CLI

```bash
cargo install tauri-cli --version "^2" --locked
```

### llama.cpp

```bash
# Build from source with CUDA support
git clone https://github.com/ggerganov/llama.cpp && cd llama.cpp
cmake -B build -DGGML_CUDA=ON && cmake --build build --target llama-server -j
sudo cp build/bin/llama-server /usr/local/bin/
```

---

## Project Structure

```
KRIA/
├── crates/
│   ├── kria-core/          # Core library (LLM, tools, memory, safety)
│   ├── kria-desktop/       # Tauri v2 desktop app
│   └── kria-server/        # Headless HTTP/WS server
├── ui/                     # SolidJS + Vite frontend
├── config/                 # Default configuration
│   └── default.toml        # Main config file
├── models/                 # Model files (gitignored)
│   └── llm/                # .gguf model files
├── kria-modules/           # Python sidecar
├── scripts/                # Build and setup scripts
└── docs/                   # Documentation
```

---

## Development Workflow

### Running the App

```bash
# With NVIDIA GPU telemetry
cargo tauri dev --features nvidia

# Without NVIDIA feature
cargo tauri dev
```

This single command:
1. Starts Vite dev server on `http://localhost:1420`
2. Compiles Rust backend
3. Opens Tauri window
4. Spawns llama-server automatically (if orchestrator enabled)

### Hot Reload

| What Changed | Reloads? | How |
|--------------|----------|-----|
| Frontend (SolidJS/CSS) | Yes, instantly | Vite HMR |
| Rust backend | Yes, recompiles | Tauri CLI watches |
| Config files | No | Restart app |
| Feature flags | No | Re-run with flag |

### Building

```bash
# Development build
cargo build --workspace

# Release build
cargo build --workspace --release

# Production bundle
bash scripts/build-release.sh --features nvidia
```

---

## Testing

### Rust Tests

```bash
# All workspace tests
cargo test --workspace

# Specific crate
cargo test -p kria-core

# Specific module
cargo test -p kria-core --lib -- llm::orchestrator

# With verbose output
cargo test --workspace -- --nocapture
```

### Frontend Tests

```bash
cd ui
npm run test:run
```

### Python Sidecar Tests

```bash
cd kria-modules
pytest
```

### Regression Tests

```bash
# Chat regression tests (mandatory for tool changes)
cargo test -p kria-core --test test_chat_regression
```

---

## Configuration

### Config File Locations

| File | Purpose | Priority |
|------|---------|----------|
| `config/default.toml` | Project defaults | Lowest |
| `~/.kria/config.toml` | User overrides | Highest |
| Environment variables | Runtime overrides | Override both |

### Key Sections

| Section | Controls |
|---------|----------|
| `[llm]` | Model mode, API URL, context |
| `[[llm.models]]` | Model files, capabilities |
| `[orchestrator]` | VRAM thresholds, llama-server path |
| `[voice]` | STT/TTS models |
| `[memory]` | Max facts, decay |
| `[safety]` | HITL, audit, rollback |
| `[hardware]` | Tier, GPU layers |

---

## LLM Backend Modes

### Mode 1: Local + Orchestrator (Recommended)

```toml
[llm]
routing_mode = "local"

[orchestrator]
enabled = true
```

KRIA auto-spawns and manages llama-server.

### Mode 2: Local + Manual

```toml
[llm]
routing_mode = "local"
local_api_url = "http://127.0.0.1:8080/v1"

[orchestrator]
enabled = false
```

Run llama-server manually.

### Mode 3: Cloud LLM

```toml
[llm]
routing_mode = "gemini"

[orchestrator]
enabled = false
```

Set `KRIA_CLOUD_API_KEY` environment variable.

---

## Debugging

### Logs

**Terminal:** Live logs in the running terminal

**Log files:** `~/.kria/logs/kria.log.YYYY-MM-DD`

```bash
# View latest logs
cat ~/.kria/logs/kria.log.$(date +%F) | jq .

# Filter errors
cat ~/.kria/logs/kria.log.$(date +%F) | jq 'select(.level == "ERROR")'

# Verbose orchestrator logging
RUST_LOG="kria_core::llm::orchestrator=debug" cargo tauri dev --features nvidia
```

**Browser DevTools:** Right-click → Inspect Element → Console

### Checking Status

```js
// In browser console
await window.__TAURI__.core.invoke("get_orchestrator_status")
```

---

## Troubleshooting

| Problem | Cause | Fix |
|---------|-------|-----|
| Blank window | Vite dev server didn't start | `fuser -k 1420/tcp` |
| `no model path configured` | No model in config | Check `[[llm.models]]` |
| `failed to spawn llama-server` | Not on PATH | `which llama-server` |
| No VRAM telemetry | NVML unavailable | Build with `--features nvidia` |
| Excessive swapping | Thresholds too sensitive | Increase `cooldown_secs` |

### Full Reset

```bash
pkill kria-desktop 2>/dev/null
cargo clean
rm -rf ui/dist ui/node_modules/.vite
rm -f ~/.kria/config.toml
cargo tauri dev --features nvidia
```

---

## Tech Stack

### Rust Core

- **Tokio** — Async runtime
- **Serde / serde_json** — Serialization
- **Reqwest** — HTTP client
- **Rusqlite** — SQLite storage
- **Tracing** — Structured logging
- **FastEmbed** — Semantic embeddings
- **NVML** — GPU telemetry

### Python Sidecar

- **Python 3.11+**
- **JSON-RPC 2.0** — Transport
- **Pillow / OpenCV** — Image processing
- **PyMuPDF / python-docx** — Document parsing
- **sentence-transformers** — Embeddings

### Frontend

- **Tauri v2** — Desktop shell
- **SolidJS** — UI framework
- **TypeScript** — Type layer
- **Vite** — Build tool

---

## Related Documentation

- **ARCHITECTURE.md** — System architecture
- **TOOLS.md** — Tool development guide
- **HARDWARE.md** — GPU orchestration
- **HOW_TO_RUN.md** — Detailed run instructions
