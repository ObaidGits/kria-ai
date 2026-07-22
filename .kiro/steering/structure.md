---
inclusion: manual
---

# Project Structure & Organization

> Summon with `#structure` when navigating the repo layout or module ownership. Core essentials are always-on in `core.md`.

## Top-Level Directory Layout

```
KRIA/
├── crates/                          # Rust workspace crates
│   ├── kria-core/                   # Core domain library (authoritative)
│   ├── kria-desktop/                # Desktop app runtime (primary product)
│   ├── kria-server/                 # Standalone server + fleet APIs
│   ├── kria-eval/                   # E2E evaluation harness
│   ├── kria-connection-control/     # Signed lease management
│   └── kria-uinput-daemon/          # Input device daemon
├── ui/                              # Frontend (SolidJS + TypeScript)
├── sidecars/                        # Python sidecar services
├── openclaw-substrate/              # OpenClaw skill substrate
├── kria-modules/                    # Modular extensions
├── docs/                            # Documentation
├── tests/                           # E2E tests (Playwright)
├── scripts/                         # Build/utility scripts
├── config/                          # Configuration files
├── docs/llm-context/                # AI/LLM development context
├── models/                          # Downloaded/cached models
├── vendor/                          # Vendored dependencies (piper-rs)
├── Cargo.toml                       # Workspace root
├── Cargo.lock                       # Dependency lock file
├── kria_config.toml                 # Main configuration
├── .env                             # Environment variables
├── Dockerfile                       # GPU image
├── Dockerfile.cpu                   # CPU image
├── docker-compose.yml               # Docker Compose (GPU)
├── docker-compose.cpu.yml           # Docker Compose (CPU)
├── justfile                         # Just recipes
├── rust-toolchain.toml              # Rust version pinning
└── README.md                        # Project overview
```

## Rust Crates Structure

### kria-core/
**Authoritative domain library; all business logic lives here.**

```
crates/kria-core/src/
├── lib.rs                           # Crate root
├── agent/                           # Agent loop & reasoning
│   ├── loop_engine/                 # ReAct loop implementation
│   ├── intent_extractor.rs          # Intent classification
│   ├── prompt_construction.rs       # Prompt building
│   ├── response_parser.rs           # Response parsing
│   ├── routing.rs                   # Tool routing
│   └── ...
├── tools/                           # Tool implementations
│   ├── registry.rs                  # Tool registry
│   ├── apps.rs                      # App launcher tools
│   ├── files.rs                     # File operations
│   ├── internet.rs                  # Web search, fetch
│   ├── documents.rs                 # PDF, DOCX parsing
│   ├── packages.rs                  # Package management
│   ├── shell.rs                     # Shell execution
│   ├── system.rs                    # System info
│   ├── google_workspace.rs          # Google integration
│   ├── image.rs                     # Image generation
│   ├── mcp.rs                       # MCP tools
│   └── ...
├── safety/                          # Safety & policy
│   ├── policy.rs                    # Risk classification
│   ├── hitl.rs                      # Human-in-the-loop
│   ├── audit.rs                     # Audit logging
│   ├── rollback.rs                  # Rollback snapshots
│   └── blacklist.rs                 # Blacklist checks
├── llm/                             # LLM orchestration
│   ├── router.rs                    # Model routing
│   ├── local_client.rs              # Local llama-server
│   ├── cloud_client.rs              # Cloud API client
│   ├── model_manager.rs             # Model lifecycle
│   ├── llama_server.rs              # Server orchestration
│   └── ...
├── voice/                           # Voice pipeline
│   ├── v1/                          # V1 pipeline
│   ├── v2/                          # V2 streaming
│   ├── stt.rs                       # Speech-to-text
│   ├── tts.rs                       # Text-to-speech
│   ├── vad.rs                       # Voice activity detection
│   └── ...
├── image/                           # Image generation
│   ├── orchestrator.rs              # Generation orchestration
│   ├── comfyui.rs                   # ComfyUI backend
│   ├── cloud_fallback.rs            # Cloud fallback
│   └── ...
├── memory/                          # Memory & knowledge
│   ├── facts.rs                     # Fact storage
│   ├── rag.rs                       # RAG system
│   ├── embeddings.rs                # Semantic embeddings
│   ├── store.rs                     # SQLite store
│   └── ...
├── routing/                         # Domain routing
│   ├── semantic.rs                  # Semantic routing
│   ├── lexical.rs                   # Lexical routing
│   └── ...
├── mcp/                             # Model Context Protocol
│   ├── client.rs                    # MCP client
│   ├── protocol.rs                  # Protocol handling
│   ├── server_lifecycle.rs          # Server management
│   └── ...
├── openclaw/                        # OpenClaw skills
│   ├── container_pool.rs            # Container management
│   ├── skill_registry.rs            # Skill registry
│   ├── audit_ledger.rs              # Audit logging
│   └── ...
├── automation/                      # Automation & workflows
│   ├── event_bus.rs                 # Event system
│   ├── workflows.rs                 # Workflow engine
│   ├── scheduler.rs                 # Task scheduling
│   └── ...
├── infra/                           # Infrastructure
│   ├── health.rs                    # Health checks
│   ├── provisioning.rs              # Provisioning
│   ├── remote_qemu.rs               # Remote execution
│   ├── snapshots.rs                 # VM snapshots
│   └── ...
├── platform/                        # Platform abstraction
│   ├── os_detection.rs              # OS detection
│   ├── app_registry.rs              # App registry
│   ├── sandbox.rs                   # Sandboxing
│   └── ...
├── preprocessing/                   # Data preprocessing
│   ├── code.rs                      # Code preprocessing
│   ├── document.rs                  # Document preprocessing
│   └── ...
├── types.rs                         # Shared types
├── config.rs                        # Configuration
└── error.rs                         # Error types
```

### kria-desktop/
**Desktop app runtime; Tauri integration and UI commands.**

```
crates/kria-desktop/src/
├── main.rs                          # App entry point
├── lib.rs                           # Crate root
├── commands/                        # Tauri commands (modular)
│   ├── app_commands.rs              # App lifecycle
│   ├── app_state.rs                 # State management
│   ├── automation.rs                # Automation commands
│   ├── chat.rs                      # Chat commands
│   ├── colab.rs                     # Collaboration
│   ├── fleet_enrollment.rs          # Fleet enrollment
│   ├── fleet_tools.rs               # Fleet operations
│   ├── google_workspace.rs          # Google integration
│   ├── image_chat.rs                # Image generation
│   ├── mcp.rs                       # MCP commands
│   ├── openclaw.rs                  # OpenClaw commands
│   ├── voice.rs                     # Voice commands
│   ├── sessions.rs                  # Session management
│   └── ...
├── events.rs                        # Event emission
├── sidecar.rs                       # Sidecar bridge
├── local_api.rs                     # Local API server
├── config.rs                        # Desktop config
└── error.rs                         # Error handling
```

### kria-server/
**Standalone server; HTTP/WebSocket APIs and fleet routes.**

```
crates/kria-server/src/
├── main.rs                          # Server entry point
├── lib.rs                           # Crate root
├── routes/                          # API routes
│   ├── chat.rs                      # Chat endpoints
│   ├── tools.rs                     # Tool endpoints
│   ├── fleet.rs                     # Fleet endpoints
│   ├── provisioning.rs              # Provisioning endpoints
│   └── ...
├── handlers.rs                      # Request handlers
├── websocket.rs                     # WebSocket support
├── config.rs                        # Server config
└── error.rs                         # Error handling
```

### kria-eval/
**E2E evaluation harness.**

```
crates/kria-eval/src/
├── lib.rs                           # Crate root
├── suites/                          # Test suites
├── runner.rs                        # Test runner
├── fixtures.rs                      # Test fixtures
├── judge.rs                         # Result judge
└── reports.rs                       # Report generation
```

## Frontend Structure (ui/)

```
ui/
├── src/
│   ├── App.tsx                      # Root component
│   ├── main.tsx                     # Entry point
│   ├── components/                  # Reusable components
│   │   ├── Chat.tsx                 # Chat view
│   │   ├── VoiceOverlay.tsx         # Voice UI
│   │   ├── ImageProgress.tsx        # Image generation progress
│   │   ├── HITLModal.tsx            # Human-in-the-loop approval
│   │   ├── ToolResult.tsx           # Tool result display
│   │   └── ...
│   ├── stores/                      # State management
│   │   ├── app.ts                   # Main app store
│   │   ├── chat.ts                  # Chat state
│   │   ├── voice.ts                 # Voice state
│   │   └── ...
│   ├── views/                       # Page-level components
│   │   ├── ChatView.tsx             # Chat page
│   │   ├── PromptLabView.tsx        # Prompt Lab page
│   │   ├── FleetMatrixView.tsx      # Fleet management
│   │   ├── SettingsView.tsx         # Settings page
│   │   └── ...
│   ├── utils/                       # Utilities
│   │   ├── api.ts                   # API client
│   │   ├── formatting.ts            # Text formatting
│   │   └── ...
│   ├── locales/                     # i18n translations
│   │   ├── en.json                  # English
│   │   ├── ar.json                  # Arabic
│   │   ├── de.json                  # German
│   │   ├── es.json                  # Spanish
│   │   ├── fr.json                  # French
│   │   ├── hi.json                  # Hindi
│   │   └── zh.json                  # Chinese
│   └── styles/                      # Global styles
├── public/                          # Static assets
├── package.json                     # Dependencies
├── vite.config.ts                   # Vite configuration
├── tsconfig.json                    # TypeScript config
└── tailwind.config.js               # TailwindCSS config
```

## Python Sidecar Structure (sidecars/)

```
sidecars/
├── kria_sidecar/                    # Main module
│   ├── __init__.py                  # Package init
│   ├── main.py                      # Entry point
│   ├── bridge.py                    # JSON-RPC bridge
│   ├── audio/                       # Audio processing
│   │   ├── __init__.py
│   │   ├── processor.py             # Audio processing
│   │   └── ...
│   ├── code/                        # Code execution
│   │   ├── __init__.py
│   │   ├── executor.py              # Code executor
│   │   └── ...
│   ├── document/                    # Document parsing
│   │   ├── __init__.py
│   │   ├── parser.py                # Document parser
│   │   └── ...
│   ├── embeddings/                  # Semantic embeddings
│   │   ├── __init__.py
│   │   ├── generator.py             # Embedding generator
│   │   └── ...
│   ├── google/                      # Google Workspace
│   │   ├── __init__.py
│   │   ├── client.py                # Google client
│   │   └── ...
│   ├── image/                       # Image processing
│   │   ├── __init__.py
│   │   ├── processor.py             # Image processor
│   │   └── ...
│   ├── news/                        # News aggregation
│   │   ├── __init__.py
│   │   ├── aggregator.py            # News aggregator
│   │   └── ...
│   ├── web/                         # Web processing
│   │   ├── __init__.py
│   │   ├── processor.py             # Web processor
│   │   └── ...
│   └── telegram/                    # Telegram MCP server
│       ├── __init__.py
│       ├── server.py                # MCP server
│       └── ...
├── requirements.txt                 # Python dependencies
├── setup.py                         # Package setup
└── README.md                        # Sidecar documentation
```

## Documentation Structure (docs/)

```
docs/
├── ARCHITECTURE.md                  # System architecture (detailed)
├── TOOLS.md                         # Tool system guide
├── OPENCLAW.md                      # OpenClaw integration
├── SAFETY.md                        # Safety model
├── DEVELOPMENT.md                   # Development workflow
├── SYSTEM_DESIGN.md                 # System design reference
├── VOICE.md                         # Voice pipeline
├── MEMORY.md                        # Memory & RAG system
├── HARDWARE.md                      # Hardware orchestration
├── DEPLOYMENT.md                    # Packaging & distribution
├── FAQ.md                           # Frequently asked questions
├── ADR/                             # Architecture Decision Records
│   ├── ADR-001-*.md
│   ├── ADR-002-*.md
│   └── ...
└── images/                          # Documentation images
```

## Configuration Files

```
config/
├── default.toml                     # Default configuration
├── mcp_servers.json                 # MCP server definitions
└── seccomp/
    └── kria-seccomp.json            # Seccomp profile
```

## Testing Structure

```
tests/
├── e2e/                             # End-to-end tests (Playwright)
│   ├── tests/
│   │   ├── chat.spec.ts             # Chat tests
│   │   ├── voice.spec.ts            # Voice tests
│   │   └── ...
│   ├── playwright.config.ts
│   └── package.json
└── fixtures/                        # Test fixtures
```

## Key Files to Know

| File | Purpose |
|------|---------|
| `Cargo.toml` | Workspace root; defines all crates and shared dependencies |
| `Cargo.lock` | Dependency lock file (commit to repo) |
| `kria_config.toml` | Main runtime configuration |
| `.env` | Environment variables (API keys, secrets) |
| `rust-toolchain.toml` | Rust version pinning |
| `justfile` | Just recipes for common tasks |
| `ui/package.json` | Frontend dependencies |
| `sidecars/requirements.txt` | Python dependencies |

## Important Patterns

### Modular Commands
Desktop commands are organized in `crates/kria-desktop/src/commands/` by feature area. Each module handles a specific domain (chat, voice, image, fleet, etc.). This allows surgical edits without broad rewrites.

### Workspace Dependencies
All Rust dependencies are defined in `[workspace.dependencies]` in the root `Cargo.toml`. This ensures version consistency across crates.

### Feature Flags
Use Cargo features for optional functionality (GPU support, cloud integrations, etc.). Define in individual crate `Cargo.toml` files.

### Error Handling
Use `thiserror` for custom error types and `anyhow` for flexible error handling. Errors should be user-actionable.

### Logging
Use `tracing` macros (`info!`, `warn!`, `error!`, `debug!`) for structured logging. Configure via `tracing-subscriber`.

### Testing
- **Unit tests**: Inline in source files with `#[cfg(test)]` modules
- **Integration tests**: In `tests/` directory
- **E2E tests**: Playwright tests in `tests/e2e/`
- **Frontend tests**: Vitest in `ui/`

## Avoiding Common Mistakes

1. **Don't modify kria-core directly for UI concerns** — Use kria-desktop commands instead
2. **Don't hardcode configuration** — Use `kria_config.toml` or environment variables
3. **Don't bypass safety policy** — All dangerous operations must flow through the safety layer
4. **Don't make optional services mandatory** — Sidecar, MCP, ComfyUI, etc. can be unavailable
5. **Don't change Tauri command/event names** — These are frontend/backend contracts
6. **Don't commit secrets** — Use `.env` and `.gitignore`
7. **Don't use open version ranges** — Pin exact versions for stability
