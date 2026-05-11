# KRIA Core Context

## Authoritative Runtime
- `kria-core` contains most product logic and should be treated as the source of truth for assistant behavior.
- `kria-desktop` wires `kria-core` into the Tauri app and exposes frontend-facing commands/events through a modular `commands/` directory.
- The standalone `kria-server` is useful for API/fleet paths but is not equivalent to the desktop chat runtime.

## Agent Turn Flow
- UI calls a Tauri command such as `send_message` (in `commands/chat.rs`) or prompt-lab equivalents.
- Desktop prepares runtime state, session/history, attachments, routing context, active tools, safety/HITL hooks, and local-runtime readiness.
- `AgentLoop` (in `agent/loop_engine/mod.rs`, with helpers, intent extractors, intent fallback, and response helpers) coordinates prompt construction, model routing, streaming, tool-call parsing/execution, cancellation, and final message persistence.
- Routing narrows context with semantic/lexical/domain decisions and falls back safely when embeddings are unavailable.
- Tool visibility can be controlled by mount/routing logic to reduce prompt/tool-schema bloat.

## Tool Execution Model
- Tools are defined and registered through `ToolRegistry` and domain modules under `crates/kria-core/src/tools`.
- Handlers receive `ToolContext`, can access memory, sidecar, system environment, image orchestrator, and safety/runtime facilities depending on wiring.
- MCP tools are discovered from configured MCP servers and bridged into KRIA's tool registry.
- OpenClaw skills are discovered from ClawHub or local registry, executed in sandboxed Docker containers.
- Google Workspace (with dedicated contract types in `google_workspace_contract.rs`) and image generation are first-class tool domains with dedicated UI/settings integration.
- Vision and precognitive tools may call the Python sidecar and must degrade gracefully if sidecar capability is missing.
- Local shell/exec/system tools must honor safety policy and sandbox/isolation constraints.

## Tool Priority Order
1. **Native Rust tools** — Direct execution, lowest latency
2. **MCP server tools** — External tool servers via JSON-RPC
3. **OpenClaw skills** — Sandboxed Docker containers with capability isolation

## Safety Model
- `safety/policy.rs` defines risk classifications and approval/blocking behavior.
- `safety/hitl.rs` mediates human approval requests.
- `safety/audit.rs` records actions.
- `safety/rollback.rs` supports snapshots/backups before destructive operations.
- `safety/blacklist.rs` and `pin_guard.rs` add explicit blocking/authorization controls.
- Any new dangerous action should be classified, auditable, and where practical rollback-aware.

## Model and Hardware Orchestration
- `ModelRouter` dispatches across local, cloud, and external backends according to config/routing mode.
- Local llama-server orchestration lives under `llm/orchestrator` and manages child processes, runtime state, hardware-aware strategies, vision/mmproj handling, GPU watchdog, thresholds, VRAM budgets, and telemetry.
- Hardware profiling and tiering influence context windows, threads, GPU layers, model selection, STT choices, and fallback behavior.
- `resource/gpu_lease` coordinates GPU ownership among workloads such as LLM, vision, image generation, and remote/fleet paths.
- `resource/telemetry.rs` provides resource-level telemetry.
- Desktop runtime tracks active turns and idle release behavior to avoid unnecessary local-runtime pressure.

## Image and Vision
- `image/orchestrator.rs` coordinates image generation and progress.
- ComfyUI is the main local backend; cloud fallback support exists through the image cloud module.
- `ws_bridge.rs` handles image-generation WebSocket progress bridging.
- Prompt enhancer, styles, modes, capabilities, and swap utilities are separated into dedicated modules.
- Vision tools normalize attachments, generate thumbnails/preprocessing artifacts, and can call sidecar OCR/analysis with native/CLI fallbacks.

## Voice
- v1 voice pipeline remains the default compatibility path in the desktop runtime.
- v2 voice modules provide a streaming architecture: wake detection, AEC, STT/TTS traits, sentence splitting, playback sink, and post-edit.
- Desktop state can hold an active v1 or v2 voice pipeline, but v2 end-to-end behavior may be gated by available engines/features and configuration.

## Memory and RAG
- Memory includes facts, document/RAG flows, semantic parsing, retrieval, embeddings, vector indexes, and SQLite persistence.
- Conversation/session history and media metadata are persisted locally.
- New memory behavior should preserve local-first storage and avoid blocking the main turn loop unnecessarily.

## OpenClaw Skill Substrate
- `openclaw/container_pool.rs` manages warm Docker containers for skill execution.
- `openclaw/skill_registry.rs` tracks installed skills and their manifests.
- `openclaw/capability_resolver.rs` matches user requests to skills via hybrid BM25 + dense retrieval.
- `openclaw/audit_ledger.rs` records all skill invocations with HMAC signatures.
- Skills declare capabilities (filesystem_read, filesystem_write, subprocess, network, etc.) in manifest.
- Trust tiers (Community/Verified/Partner/Internal) affect auto-approval behavior.
- Container pool must be shut down on app exit via `shutdown_runtime` command.

## Remote/Fleet/Ironclad Execution
- Remote QEMU execution, inventory pooling, snapshots, QoS, and leases follow architecture in `docs/ARCHITECTURE.md`.
- `infra/environment/remote_qemu/mod.rs` (with tests and windows_spawn submodules) and related infra modules support target runtimes, transport, file policies, helper provisioning, host artifact GC, and guest filesystem policy.
- `infra/pool`, `infra/qos`, `infra/snapshot`, and `infra/supervisor` support pooled targets, adaptive scheduling, state snapshots, and resilient process/runtime supervision.
- `kria-connection-control` provides signed lease/connection primitives.
- Desktop fleet control and UI Fleet Matrix expose target/lease/heartbeat visibility.

## Sidecar Boundary
- The Rust sidecar bridge starts and communicates with Python over JSON-RPC.
- Sidecar bootstrap (`sidecar/bootstrap.rs`) and health monitoring (`sidecar/health.rs`) manage lifecycle.
- Startup should not fail the whole app if sidecar is missing.
- Sidecar-backed tools should produce clear degraded errors and avoid panics.
- Protocol changes must be reflected in both Rust `sidecar/protocol.rs` and Python bridge/module handlers.

## Config and Contracts
- `config/default.toml` and Rust config structs must stay in sync.
- Tauri commands are a stable UI/backend contract; update `ui/src/stores/app.ts` and component call sites if command names or payloads change.
- Backend event payload changes must update frontend listeners and relevant UI types.
- MCP config lives in `config/mcp_servers.json` and user settings.
- OpenClaw config includes registry URL, approved capabilities, and trust tier policies.
- Seccomp policy lives in `config/seccomp/kria-seccomp.json`.
