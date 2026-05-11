# KRIA Knowledge Graph Summary

## Core Modules
- `kria-core` is the central dependency hub for assistant behavior.
- `kria-desktop` is the primary integration/runtime hub for the product UI.
- `ui` is coupled to desktop through Tauri command names and emitted event payloads.
- `kria-server` is a secondary API/fleet surface, not a replacement for desktop runtime.
- `kria-eval` provides validation and scenario execution.
- `kria-connection-control` supports signed target/lease control for fleet orchestration.

## Most Connected Components
- `crates/kria-desktop/src/commands/` connects UI, config, memory, tools, safety, voice, MCP, sidecar, LLM router, image generation, provisioning, health, local API, and fleet state.
- `crates/kria-core/src/agent/loop_engine/mod.rs` connects prompts, model routing, routing/mounting, tool calls, safety, memory, streaming, and cancellation.
- `crates/kria-core/src/tools/registry.rs` connects built-in tools, MCP tools, schemas, handlers, and safety-aware execution.
- `ui/src/stores/app.ts` connects most UI components to backend commands/events.
- `crates/kria-core/src/llm/orchestrator/` connects hardware profiling, local server lifecycle, GPU/VRAM decisions, vision strategy, telemetry, and runtime health.

## Critical System Flows
- Chat: `ChatView` -> `appStore` -> Tauri command -> desktop runtime -> `AgentLoop` -> `ModelRouter` -> tools/safety/memory -> streamed UI events.
- Tool call: model output -> response parser -> registry lookup -> safety policy/HITL/audit/rollback -> handler -> result back to agent.
- Voice: UI/tray toggle -> desktop voice state -> v1 or v2 pipeline -> STT -> agent turn -> TTS/playback -> `voice:*` events.
- Image generation: tool/UI request -> image orchestrator -> ComfyUI or cloud fallback -> WebSocket progress -> `image:*` events -> media persistence/rendering.
- MCP: config/settings -> server manager -> capability discovery -> payload shaper/tool bridge -> registry -> agent tools.
- Provisioning: SetupWizard -> provisioning store -> backend provisioning steps -> progress/status events -> persisted completion.
- Fleet/Ironclad: settings/status -> heartbeat/lease state -> Fleet Matrix/Add Target UI -> desktop/server fleet handlers -> connection-control signing/leases -> remote target orchestration.

## High-Risk Areas
- Safety bypasses around shell/exec, system config, power, package management, file mutation, remote/fleet execution, and automation.
- Frontend/backend contract drift in Tauri command names or event payload shapes.
- Local model/image/voice startup paths that assume optional services are always installed.
- GPU-heavy concurrent workloads without lease/orchestration coordination.
- Remote QEMU/VM/snapshot/pool/QoS changes that violate RFC assumptions.
- Sidecar protocol changes made only on Rust or Python side.
- Desktop command submodule changes that unintentionally affect unrelated flows.

## Confidence Notes
- Current docs reflect observed workspace structure and module declarations as of the latest scan.
- Some product claims in root `README.md` appear broader/older than current implemented Rust/Tauri architecture; prefer code and active `docs/`/RFCs over the old marketing-style feature list.
- `docs-old/` should be treated as historical unless explicitly requested.
- `ai-context` is gitignored assistant context; verify against source before making invasive implementation decisions.
