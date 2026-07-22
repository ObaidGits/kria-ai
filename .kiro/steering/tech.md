---
inclusion: manual
---

# Technology Stack & Build System

> Summon with `#tech` when working on build, dependencies, or stack details. Core essentials are always-on in `core.md`.

## Core Tech Stack

### Language & Runtime
- **Rust** (1.75+): Primary language for core, desktop, server, and infrastructure
- **TypeScript**: Frontend UI code
- **Python** (3.10+): Sidecar for heavy processors (audio, code, document, embeddings, Google Workspace, image, news, web) and Telegram MCP server

### Desktop Framework
- **Tauri v2**: Desktop application framework (smaller, faster, secure apps)
- **SolidJS**: Reactive frontend framework
- **TailwindCSS**: Utility-first CSS framework
- **Vite**: Frontend build tool

### Backend & Services
- **Axum**: HTTP/WebSocket server framework (used in kria-server)
- **Tokio**: Async runtime (full feature set)
- **SQLite**: Primary database (via `rusqlite` with bundled support)

### AI & ML
- **llama.cpp**: Local LLM inference server
- **OpenAI-compatible API**: Chat abstraction layer
- **ONNX Runtime**: For VAD, TTS, and intent classification
- **FastEmbed**: Semantic embeddings (ONNX-based, no Python required)
- **llguidance**: Grammar-constrained LLM decoding
- **Whisper**: Speech-to-text (via faster-whisper, distil-whisper)
- **Piper**: Text-to-speech
- **ComfyUI**: Local GPU-accelerated image generation
- **tree-sitter**: AST parsing (Python code analysis)

### Voice & Audio
- **cpal**: Cross-platform audio I/O
- **rodio**: Audio playback
- **WebRTC VAD**: Voice activity detection
- **Silero VAD**: Alternative VAD
- **Porcupine**: Wake word detection

### System & Infrastructure
- **sysinfo**: System information (CPU, memory, disk, GPU)
- **zbus**: D-Bus async interaction (Linux perception layer)
- **libc**: POSIX bindings (prctl, setsid, kill — Unix only)
- **nvml-wrapper**: NVIDIA GPU telemetry
- **Docker**: OpenClaw skill substrate (sandboxed execution)
- **SSH**: Remote target execution

### Data & Storage
- **fjall**: Durable embedded queue (inbox pipeline)
- **dashmap**: Concurrent hash map
- **arc-swap**: Atomic reference swapping
- **memmap2**: Memory-mapped files
- **zip**: ZIP archive reading (DOCX/XLSX/PPTX extraction)

### Utilities
- **serde/serde_json**: Serialization/deserialization
- **toml**: TOML config parsing
- **chrono**: Date/time handling
- **uuid**: Unique ID generation (v4, v7)
- **regex**: Pattern matching
- **walkdir**: Filesystem traversal
- **notify**: File system event watching
- **reqwest**: HTTP client (with rustls-tls, multipart support)
- **scraper**: Web scraping
- **base64**: Encoding/decoding
- **blake3/sha2/hmac**: Cryptographic hashing
- **rand**: Random number generation
- **arboard**: Clipboard access
- **notify-rust**: Desktop notifications
- **open**: Open URLs/files in default app

### Error Handling & Logging
- **thiserror**: Error type derivation
- **anyhow**: Flexible error handling
- **tracing**: Structured logging
- **tracing-subscriber**: Log formatting and filtering
- **tracing-appender**: Log file appending

## Build System

### Workspace Structure
```
Cargo.toml (workspace root)
├── crates/kria-core/           # Core domain library (authoritative)
├── crates/kria-desktop/        # Desktop app runtime (primary product)
├── crates/kria-server/         # Standalone server + fleet APIs
├── crates/kria-eval/           # E2E evaluation harness
├── crates/kria-connection-control/  # Signed lease management
└── crates/kria-uinput-daemon/  # Input daemon
```

### Build Profiles

**Dev Profile** (optimized for low compile RAM):
```toml
opt-level = 0
debug = 1              # Line tables only
codegen-units = 16     # Bound LLVM codegen partitions
incremental = true
split-debuginfo = "unpacked"
```

**Release Profile** (optimized for performance):
```toml
opt-level = 3
lto = "thin"
codegen-units = 16
strip = "debuginfo"
panic = "abort"
```

## Common Commands

These commands are references, not a default sequence to run in full. Start with
focused diagnostics, affected tests, or package checks; escalate to workspace,
release, E2E, Docker, or packaging validation only when scope and risk justify it.
Reuse existing development services and build caches. Keep one managed instance of
long-running servers/watchers, stop temporary instances after use, and avoid running
heavy builds concurrently. Control Cargo job concurrency separately from
`codegen-units`, based on current RAM, CPU load, and task size.

### Building
```bash
# Build all crates (debug)
cargo build

# Build release binary
cargo build --release

# Build specific crate
cargo build -p kria-desktop --release

# Build with specific features
cargo build --features "feature1,feature2"
```

### Testing
```bash
# Run all tests
cargo test

# Run tests for specific crate
cargo test -p kria-core

# Run tests with output
cargo test -- --nocapture

# Run focused test
cargo test test_name

# Run E2E tests (Playwright)
cd tests/e2e && npm test

# Run frontend tests
cd ui && npm test
```

### Development
```bash
# Start Tauri dev server (hot reload)
cargo tauri dev

# Format code
cargo fmt

# Lint code
cargo clippy

# Check without building
cargo check

# Generate documentation
cargo doc --open
```

### Desktop App
```bash
# Run desktop app (debug)
cargo run -p kria-desktop

# Run desktop app (release)
cargo run -p kria-desktop --release

# Build installer/bundle
cargo tauri build
```

### Server
```bash
# Run standalone server
cargo run -p kria-server --release

# Run with specific config
KRIA_CONFIG=config/default.toml cargo run -p kria-server
```

### Frontend
```bash
# Install dependencies
cd ui && npm install

# Start dev server
npm run dev

# Build for production
npm run build

# Run tests
npm test

# Format/lint
npm run format
npm run lint
```

### Docker
```bash
# Build CPU image
docker build -f Dockerfile.cpu -t kria:cpu .

# Build GPU image
docker build -f Dockerfile -t kria:gpu .

# Run with docker-compose
docker-compose up

# Run CPU variant
docker-compose -f docker-compose.cpu.yml up
```

## Code Organization

### Rust Crates
- **kria-core**: Domain logic, tools, safety, agent loop, memory, routing, LLM, voice, image, OpenClaw, MCP, automation, infrastructure
- **kria-desktop**: Tauri commands, event handlers, UI integration, sidecar bridge, local API
- **kria-server**: Axum HTTP/WebSocket APIs, fleet routes, provisioning
- **kria-eval**: E2E testing harness with suites, runner, fixtures, judge
- **kria-connection-control**: Signed lease/connection management for fleet
- **kria-uinput-daemon**: Input device daemon

### Frontend Structure
```
ui/
├── src/
│   ├── components/      # Reusable UI components
│   ├── stores/          # State management (app.ts, etc.)
│   ├── views/           # Page-level components
│   ├── utils/           # Utilities and helpers
│   ├── locales/         # i18n translations (en, ar, de, es, fr, hi, zh)
│   └── App.tsx          # Root component
├── package.json
└── vite.config.ts
```

### Python Sidecar
```
sidecars/
├── kria_sidecar/        # Main sidecar module
│   ├── audio/           # Audio processing
│   ├── code/            # Code execution
│   ├── document/        # Document parsing
│   ├── embeddings/      # Semantic embeddings
│   ├── google/          # Google Workspace integration
│   ├── image/           # Image processing
│   ├── news/            # News aggregation
│   ├── web/             # Web processing
│   └── telegram/        # Telegram MCP server
└── requirements.txt
```

## Configuration

### Main Config File
- **kria_config.toml**: Primary configuration (model paths, API keys, hardware tier, voice settings, etc.)
- **.env**: Environment variables (API keys, secrets)
- **config/default.toml**: Default configuration template

### MCP Configuration
- **config/mcp_servers.json**: MCP server definitions and settings

## Dependencies Management

- **Workspace dependencies**: Defined in `[workspace.dependencies]` in root `Cargo.toml`
- **Pinned versions**: Use exact versions for stability; avoid open ranges
- **Vendored dependencies**: `vendor/piper-rs` is patched locally via `[patch.crates-io]`
- **Python dependencies**: Managed via `sidecars/requirements.txt`

## Performance Considerations

- **Dev profile**: Optimized for low compile RAM (16 codegen units) to prevent OOM on low-RAM machines
- **Release profile**: Thin LTO, debug info stripping, abort on panic
- **Incremental compilation**: Enabled in dev profile for faster iteration
- **Codegen partitioning**: 16 units balance compile speed and memory; control Cargo job concurrency separately based on current system load
