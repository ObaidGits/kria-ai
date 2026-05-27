# KRIA Deployment Operations

Last updated: 2026-05-27

## Purpose

This document defines how KRIA is packaged, configured, started, observed, and recovered in operational environments. It describes deployment behavior as implemented, not aspirational architecture.

KRIA's deployment rule is simple:

```text
The host may run KRIA, but KRIA core remains the authority for tools, safety,
providers, integrations, and completion claims.
```

## Deployable Surfaces

Workspace crates:

- `kria-core`: runtime logic, agent loop, tools, config, providers, memory, safety, integrations.
- `kria-desktop`: Tauri desktop app and command surface.
- `kria-server`: standalone server target.
- `kria-eval`: evaluation harness.
- `kria-connection-control`: remote target connection-control runtime.
- `kria-uinput-daemon`: GUI input daemon.
- `kria-test-app`: test application for GUI/E2E flows.

Packaging surfaces:

- `scripts/build-release.sh`: Linux/macOS Tauri release build.
- `scripts/build-release.ps1`: Windows release build.
- `Dockerfile`, `Dockerfile.cpu`, `docker-compose.yml`, `docker-compose.cpu.yml`: container deployment profiles.
- `Dockerfile.openclaw-substrate`: OpenClaw sandbox image.
- `justfile`: local/CI task entry points.

## Runtime Startup

Desktop startup begins in `crates/kria-desktop/src/main.rs`.

Startup sequence:

1. Install Linux seccomp filter when supported.
2. Build Tauri app with desktop plugins.
3. Register `AppStateCell` immediately so early commands fail cleanly instead of panicking.
4. Create tray.
5. Start `init_runtime` in the background.
6. On exit, run `shutdown_runtime` once.

`init_runtime` in `crates/kria-desktop/src/commands/runtime.rs` performs the operational boot:

1. Resolve paths and initialize logging.
2. Load config.
3. Detect/cache hardware tier and clamp runtime limits.
4. Open SQLite memory store.
5. Boot OpenClaw registry/audit tables and optional Docker pool.
6. Build model router and optional local llama-server orchestrator.
7. Start Python sidecar non-blocking.
8. Build tool registry, semantic router, tool index, and agent loop.
9. Load MCP server config and register MCP-backed tools.
10. Register Google Workspace, n8n, fleet, image, GUI, app lifecycle, and OpenClaw tools when available.
11. Wire safety, HITL, audit, rollback, verifier, PSDG, transparency, and workflow cognition engines.

Missing optional subsystems degrade. They should not block the entire desktop runtime unless the missing dependency is required by the selected workflow.

## Configuration

Config load order:

1. Project default: discovered `config/default.toml`.
2. User override: `~/.kria/config.toml`.
3. Explicit override path when caller provides one.
4. Environment variables.

Important environment overrides:

- `KRIA_MODELS_DIR`
- `KRIA_TIER`
- `KRIA_LLM_MODE`
- `KRIA_ACTIVE_PROVIDER`
- `KRIA_ACTIVE_MODEL`
- `KRIA_PROVIDER_API_KEY`
- `KRIA_OPENAI_API_KEY`
- `KRIA_GEMINI_API_KEY`
- `KRIA_ANTHROPIC_API_KEY`
- `KRIA_OPENROUTER_API_KEY`
- `KRIA_OPENCODE_API_KEY`
- `KRIA_CLOUD_API_KEY`
- `KRIA_AGENT_AUTONOMY_PROFILE`
- `KRIA_AGENT_MAX_TOOL_ROUNDS`
- `KRIA_AGENT_MIN_CONFIDENCE`
- `KRIA_COLAB_ENABLED`
- `KRIA_COLAB_MCP_SERVER`
- `KRIA_ENABLE_ONNX_L0`
- `KRIA_ONNX_L0_MODEL_PATH`

Environment wins over user settings for provider/runtime selection.

## Standard Data Paths

`KriaPaths::resolve()` creates and uses:

| Path | Purpose |
|---|---|
| `~/.kria/config.toml` | User config override. |
| `~/.kria/kria.db` | Memory, audit, PSDG/world-model, and related SQLite state. |
| `~/.kria/skills.db` | OpenClaw skill registry and audit table. |
| `~/.kria/models/llm` | GGUF local LLM files. |
| `~/.kria/models/stt` | Speech-to-text models. |
| `~/.kria/models/tts` | Text-to-speech voices/models. |
| `~/.kria/models/embeddings` | Embedding models. |
| `~/.kria/rollback` | Rollback snapshots. |
| `~/.kria/workflows` | User/runtime workflow state. |
| `~/.kria/plugins` | Plugin data. |
| `~/.kria/logs` | Runtime logs. |
| `~/.kria/n8n/callback_inbox.jsonl` | Durable n8n callback replay inbox. |
| `~/.kria/n8n/governance_audit.jsonl` | n8n governance audit records. |

`KRIA_MODELS_DIR` can relocate model storage.

## Release Build

Primary desktop release command:

```bash
scripts/build-release.sh
```

The script:

1. Verifies `cargo`, `node`, `npm`, and `cargo-tauri`.
2. Builds the frontend in `ui`.
3. Stages bundled resources under `crates/kria-desktop/resources`.
4. Downloads/stages `llama-server` and `uv` if missing.
5. Optionally stages a `kria-modules` wheel.
6. Runs `cargo tauri build`.
7. Reports bundles under `target/release/bundle`.

General workspace release build:

```bash
cargo build --release --workspace
```

Release gate profile:

```bash
scripts/run_release_test_gate.sh
```

That script runs `cargo kria-test --mode RELEASE` with strict release-gate environment defaults.

## Optional Runtime Dependencies

| Dependency | Needed for |
|---|---|
| Docker | OpenClaw sandbox execution. |
| `kria/openclaw-substrate:latest` image | OpenClaw container pool. |
| Python 3 | Sidecar and selected media/vision helpers. |
| GGUF model files | Local llama.cpp runtime. |
| `llama-server` | Managed local LLM runtime. |
| AT-SPI and accessibility settings | Semantic GUI interaction on Linux. |
| uinput daemon | Low-level GUI input where enabled. |
| OCR dependency | Vision fallback and visible GUI verification. |
| n8n endpoint/API key/signing secret | n8n workflow substrate. |
| MCP server commands | MCP tools and integrations. |

OpenClaw image build:

```bash
docker build -f Dockerfile.openclaw-substrate -t kria/openclaw-substrate:latest .
```

## Production Preflight

Before shipping or starting a production profile:

1. Run `cargo fmt --all`.
2. Run `cargo clippy --workspace --all-features -- -D warnings`.
3. Run `cargo test --workspace --lib`.
4. Run targeted integration/eval suites for changed subsystems.
5. Confirm `~/.kria/config.toml` does not contain unintended credentials or stale provider IDs.
6. Confirm model files exist under the expected model path.
7. Confirm OpenClaw Docker image exists if OpenClaw is enabled.
8. Confirm n8n catalog workflows are approved before enabling execution.
9. Confirm GUI prerequisites if GUI automation is part of the deployment.
10. Confirm logs write under `~/.kria/logs`.

## Health And Observability

Operational signals are exposed through:

- Tauri commands such as `get_health`, `get_runtime_diagnostics`, `get_orchestrator_status`, provider commands, n8n status, OpenClaw status, and GUI automation status.
- Logs under `~/.kria/logs`.
- SQLite state in `kria.db`.
- n8n JSONL callback and governance files.
- OpenClaw `skills.db` audit entries.
- UI events such as `orchestrator:selected`, `orchestrator:disabled`, `llm-runtime:apply`, `n8n:callback`, `n8n:governance`, and `n8n:hitl_response`.

Watch these in production:

- runtime boot failures,
- provider connection failures,
- local llama-server startup/swap failures,
- OpenClaw pool unavailable status,
- n8n dead letters,
- HITL timeout/denial rates,
- GUI accessibility readiness,
- memory/audit DB write failures,
- model file lookup failures.

## Failure Handling

| Failure | Expected behavior |
|---|---|
| Missing project default config | Use user config or defaults. |
| Missing user config | Start from project/default config. |
| Missing model files | Disable local orchestrator and emit actionable status. |
| Cloud/external provider active | Skip local llama-server and avoid GPU allocation. |
| OpenClaw disabled | Do not start container pool. |
| Docker/image missing | Mark OpenClaw unavailable, keep runtime alive. |
| n8n invalid config | Do not register `n8n_invoke_workflow`. |
| Python sidecar failure | Mark sidecar/vision degraded, keep runtime alive. |
| AT-SPI unavailable | Mark semantic GUI degraded and surface remediation. |
| DB open failure | Fail startup for required persistent state. |

Recovery rule:

```text
Restart or downgrade only through explicit runtime/config paths.
Do not bypass safety, HITL, verifier, or tool authority to recover faster.
```

## Deployment Invariants

- Runtime config changes must be traceable.
- Credentials must come from config or environment, not source code.
- Optional integrations must degrade cleanly.
- Dangerous actions remain policy/HITL gated.
- External substrates are execution targets, not authority layers.
- Visible GUI workflows must still satisfy verifier/fidelity requirements.
- Production success requires both code tests and relevant evals.
