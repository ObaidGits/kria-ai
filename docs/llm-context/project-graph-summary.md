# KRIA Project Graph Summary

**Last updated:** 2026-05-27

## Core Modules

- `kria-core` is the central Rust library for agent runtime, tools, safety, memory, routing, GUI cognition, HITL decisions, model routing, sidecars, image, voice, N8N, OpenClaw, and infrastructure.
- `kria-desktop` is the primary product runtime: Tauri commands, state wiring, tray integration, local API bridge, device controls, and UI event emission.
- `ui` is the SolidJS frontend coupled to desktop through Tauri command names and emitted event payloads.
- `kria-eval` owns non-product eval harnesses, including GUI, workflow, and integration eval suites.
- `kria-server` is a secondary Axum API/WebSocket/provider/inventory surface, not a replacement for desktop runtime.
- `kria-connection-control` provides signed connection/lease primitives for device and remote orchestration.
- `kria-uinput-daemon` is a focused input daemon for lower-level GUI automation.
- `kria-test-app` is a small test harness app.
- `kria-modules` and `sidecars/` provide Python-side processing and optional sidecar services.

## Most Connected Components

- `crates/kria-desktop/src/commands/` connects UI, config, memory, tools, safety, voice, MCP, sidecar, LLM router, image generation, provisioning, health, local API, N8N/OpenClaw, provider settings, device state, and eval/test controls.
- `crates/kria-core/src/agent/loop_engine/mod.rs` connects prompts, model routing, routing/mounting, tool calls, safety, memory, streaming, cancellation, and result synthesis.
- `crates/kria-core/src/tools/registry.rs` connects built-in tools, MCP tools, schemas, handlers, execution context, and resume capability metadata.
- `crates/kria-core/src/agent/execution_gate.rs` connects readiness, preflight, execution authority, policy, durable decisions, and resource requirement declaration.
- `crates/kria-core/src/agent/collaborative_decision.rs` is the durable HITL decision/event envelope used by action-center resume and continuation.
- `ui/src/stores/app.ts` connects most UI components to backend commands/events.
- `crates/kria-core/src/llm/orchestrator/` connects hardware profiling, local server lifecycle, GPU/VRAM decisions, vision strategy, telemetry, and runtime health.

## Critical System Flows

- Chat: `ChatView` -> `appStore` -> Tauri command -> desktop runtime -> `AgentLoop` -> `ModelRouter` -> tools/safety/memory -> streamed UI events.
- Tool call: model output -> response parser -> registry lookup -> execution gate/policy/HITL/audit/resource leases -> handler -> result synthesis.
- Durable HITL resume: `InteractionDecision` -> user decision -> `ResumeExecutor` -> resume gate -> resource leases -> one deterministic local tool action -> execution event log.
- GUI cognition: prompt -> semantic workflow frame/fidelity/mode/contract metadata -> substrate planner/workflow execution -> verifier authority/observable completion/hybrid sync.
- Voice: UI/tray toggle -> desktop voice state -> v1 or v2 pipeline -> STT -> agent turn -> TTS/playback -> `voice:*` events.
- Image generation: tool/UI request -> image orchestrator -> ComfyUI or cloud fallback -> WebSocket progress -> `image:*` events -> media persistence/rendering.
- MCP: config/settings -> server manager -> capability discovery -> payload shaper/tool bridge -> registry -> agent tools.
- Provisioning: setup wizard -> provisioning store -> backend provisioning steps -> progress/status events -> persisted completion.
- Device/remote/fleet: device UI/settings -> desktop/server handlers -> connection-control signing/leases -> remote target/inventory/orchestration.
- Eval: `kria-eval` suite -> sandbox/fixture/judge/report -> eval reports and regression signals.

## High-Risk Areas

- Safety bypasses around shell/exec, system config, power, package management, file mutation, GUI automation, remote/fleet execution, and external integrations.
- Runtime-authority drift where tools bypass `ExecutionGate`, policy, HITL, audit, leases, or preflight.
- Frontend/backend contract drift in command names, payload shapes, or event names.
- Durable decision drift: stale action/target hashes, expired decisions, unsupported resume tools, or JSONL replay mismatch.
- GUI cognition overclaiming: structural execution must not claim visible workflow success without visible/surfaced evidence.
- Local model/image/voice startup paths that assume optional services are installed.
- GPU-heavy concurrent workloads without lease/orchestration coordination.
- Remote QEMU/VM/snapshot/pool/QoS changes that violate RFC assumptions.
- Sidecar protocol changes made only on Rust or Python side.
- Eval report/log artifacts accidentally treated as source truth.

## Confidence Notes

- This context pack reflects observed workspace structure and module declarations as of 2026-05-27.
- This folder is AI-facing orientation, not execution authority.
- Prefer current code and canonical docs over historical planning docs.
- Root `README.md` may contain broader product claims; verify behavior against code, tests, and active docs before relying on it.
