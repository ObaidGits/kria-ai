# KRIA Project Context

## Tech Stack
- Rust workspace: `kria-core`, `kria-desktop`, `kria-server`, `kria-eval`, and `kria-connection-control`.
- UI: SolidJS + TypeScript + Vite + Tauri v2 invoke/events.
- Desktop runtime: Tauri backend hosting the full assistant runtime and frontend contract.
- Server runtime: Axum HTTP/WebSocket APIs plus fleet routes; desktop also starts a local API bridge.
- Python sidecar: JSON-RPC over stdio for heavy processors (audio, code, document, embeddings, google, image, news, web) and Telegram MCP server.
- Storage: SQLite via `rusqlite`, local vector indexes, local config/model/cache/data directories.
- AI/runtime: OpenAI-compatible chat abstraction, local llama-server orchestration, optional cloud/external routing, ONNX/fastembed support, grammar-constrained decoding support via `llguidance`.
- Execution/fleet: remote QEMU abstractions, target pools, connection-control leases/signing, QoS scheduler, snapshots, provisioning, and supervisor infrastructure.
- OpenClaw: Docker-based skill substrate with container pool, capability resolver, audit ledger.

## Architecture Style
- Modular monorepo with `kria-core` as the authoritative domain/runtime library.
- Desktop app is the primary product runtime; the standalone server is a secondary API/fleet surface.
- Rust is the control, safety, orchestration, and policy boundary; Python sidecar is subordinate and should degrade gracefully.
- UI state is event-driven: frontend invokes Tauri commands and listens for backend events.
- Tool execution is safety-gated and can target local or remote/fleet contexts through orchestration layers.
- Desktop commands are organized in a modular `commands/` directory split by feature area.

## Core Subsystems
- `desktop`: primary runtime host; wires config, memory, tools, safety, voice, sidecar, MCP, image, orchestrator, fleet control, provisioning, health, OpenClaw substrate, and UI events. Commands are modular (app_commands, app_state, automation, chat, colab, colab_dispatch, command_helpers, constants, fleet_enrollment, fleet_tools, google_workspace, history_helpers, image_chat, ironclad, local_api, mcp, media, openclaw, orchestrator_helpers, provisioning, runtime, sessions, telegram, tests, tool_result_helpers, voice, voice_diagnostics, voice_runtime_helpers, analytics, test_runner).
- `agent`: user-turn loop (modular loop_engine directory with helpers, intent extractors, intent fallback, response helpers, and tests), prompt construction, routing, parsing, streaming, cancellation, prompt lab profiles, ONNX classification helpers, executive controller, perception, curiosity, failure analyzer, planner v2, prompt optimizer, self model, skill compiler, uncertainty, working set, world model.
- `tools`: built-in handlers for apps, files, internet, docs, packages, power, processes, system info/config, shell/exec, scheduler, knowledge/RAG, communication, Google Workspace (+ contract types), image generation, vision, proactive/precognitive, MCP-backed tools, browser agent.
- `safety`: risk policy, HITL approval, audit logging, rollback snapshots, blacklist checks, and PIN guards.
- `llm`: model router, local/cloud clients, model manager, llama-server orchestration (with telemetry), GPU watchdog, tier/vision/VRAM strategies, server binary handling, and token helpers.
- `image`: image generation orchestration, ComfyUI backend, cloud fallback, WebSocket progress bridge, prompt enhancement, style/mode/capability handling, and swap utilities.
- `voice`: v1 voice pipeline plus v2 streaming architecture with wake, AEC, STT/TTS traits, sentence splitting, post-edit, playback, and metrics.
- `memory`: facts, decay, manager/runtime, retrieval, semantic parser, embeddings, RAG, SQLite store, and vector index.
- `routing`: semantic/lexical domain routing, OOD checks, verb/domain segmentation, trace, embed, decision, and cache modules.
- `infra`: health, observability, pipeline traces, provisioning, sandbox/isolation, downloads, hardware profiling, circuit breakers, pools, QoS, remote QEMU environment (with tests and windows_spawn), snapshots, and supervisor.
- `platform`: OS/app detection, app registry, contacts, paths, sandboxing, VRAM, Telegram, inbox (adapter, approval, egress, media, policy, queue), and typed OS intent dispatch (capability, dispatcher, grammar, linux/macos/windows resolution, scheme).
- `mcp`: MCP client/protocol, server lifecycle, capability discovery, payload shaping, and tool bridge.
- `openclaw`: container pool, skill registry, capability resolver, audit ledger, event stream, ClawHub client.
- `automation`: event bus, workflows, scheduler, macro recorder, and proactive automation.
- `resource`: GPU lease and telemetry coordination for competing workloads.
- `sidecar`: bootstrap, bridge, health, and protocol modules for Python sidecar lifecycle.
- `eval`: E2E/evaluation harness with suites, runner, sandbox, fixtures, LLM fixture, judge, and reports.
- `connection-control`: signed lease/connection management used by fleet/remote target orchestration.
- `preprocessing`: code, document, image, web, and token-budget preprocessing.
- `plugin`: runtime support for native plugin loading.

## Current UI Surfaces
- Chat view with sessions, message rendering, tool/media details, HITL approval modal, voice overlay, image progress chip, settings, and setup wizard.
- Prompt Lab view for experimentation with profiles/tool-selection strategies.
- Fleet Matrix and Add Target modal for Ironclad/fleet target visibility and management.
- Remote Skill Card for OpenClaw skill marketplace browsing and installation.
- Export dropdown for export actions.
- Session sidebar for session list/navigation.
- Settings include MCP, Google Workspace, OpenClaw, voice/configuration, and model/runtime controls.
- Localized UI resources exist for English, Arabic, German, Spanish, French, Hindi, and Chinese.

## Key Constraints
- Keep prompt/context size small; routing and tool mounting reduce schema bloat.
- Hardware tier affects context window, threads, GPU layers, model selection, STT model, orchestration limits, and fallback behavior.
- External/cloud paths must be explicit/configured; local-first/privacy remains the default orientation.
- Dangerous actions must flow through policy, HITL, audit, and rollback where applicable.
- Desktop commands are now modular; prefer surgical edits within the relevant command submodule over broad rewrites.
- Sidecar and local model services are allowed to be unavailable; dependent tools should report degraded capability rather than crash startup.
- Tauri command names and event payload shapes are frontend/backend contracts.

## Important Runtime Events
- Common emitted families include `agent:*`, `prompt_lab:*`, `voice:*`, `image:*`, `orchestrator:*`, `colab:*`, `ironclad:*`, `openclaw:*`, `tray:*`, provisioning/download progress, and health/status updates.

## Documentation
- See `docs/` for current documentation structure.
- Key docs: `ARCHITECTURE.md`, `TOOLS.md`, `OPENCLAW.md`, `SAFETY.md`, `DEVELOPMENT.md`.
- ADRs in `docs/ADR/` for major architectural decisions.
