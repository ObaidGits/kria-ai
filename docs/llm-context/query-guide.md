# KRIA Graph Query Guide

## What Depends On X?
- For a Rust module, start with `grep_search` for `use crate::module`, `use kria_core::module`, function/type names, and command/event names.
- For Tauri commands, check `crates/kria-desktop/src/main.rs`, `commands/`, `ui/src/stores/app.ts`, and calling components.
- For UI events, search both backend `emit(...)` calls and frontend `listen(...)` registrations.
- For tools, search the tool name in `tools/registry.rs`, the domain module, prompt/tool mounting logic, tests, and UI rendering paths.

## What Breaks If I Change Y?
- Changing `ToolRegistry` or tool schema can break agent prompts, parser expectations, safety classification, MCP bridging, prompt lab, and UI tool rendering.
- Changing `commands/` payloads can break `ui/src/stores/app.ts`, components, tests, and event listeners.
- Changing safety policy can alter tool execution, HITL modal behavior, audit logs, rollback expectations, automation, and remote/fleet operations.
- Changing model orchestration can affect chat, vision, image generation, GPU leases, hardware tiering, health status, and startup/idle behavior.
- Changing sidecar protocol requires synchronized Rust and Python changes.
- Changing fleet/remote execution requires checking RFCs 001-006, connection-control leases, fleet UI, server fleet API, QoS, pool, snapshot, and supervisor behavior.

## Trace Flow From A To B
- UI to backend: component -> `ui/src/stores/app.ts` or provisioning store -> Tauri `invoke` -> desktop `commands/` function -> core module.
- Backend to UI: core/desktop action -> `app.emit(...)` -> `ui/src/stores/app.ts` listener or component listener -> component state/render.
- Agent to tool: `AgentLoop` -> prompt/tool selection -> `ModelRouter` -> response parser -> `ToolRegistry` -> safety/HITL/audit/rollback -> handler.
- Image: tool/command -> `ImageOrchestrator` -> backend/cloud/ComfyUI -> `ws_bridge` progress -> desktop event -> `ImageProgressChip`/message media.
- Voice: UI/tray -> desktop command/event -> voice pipeline -> agent turn -> TTS/playback -> `VoiceOverlay`.
- MCP: settings/config -> `McpServerManager` -> discovery -> `tool_bridge` -> `ToolRegistry` -> agent loop.
- Fleet: UI settings/status -> `useFleetHeartbeat`/Fleet Matrix -> desktop/server fleet code -> connection-control -> remote target/lease.

## Where Is This Feature Implemented?
- Chat/session/history: `commands/`, `agent/loop_engine/mod.rs`, `memory/store.rs`, `ui/src/stores/app.ts`, `ChatView.tsx`, `SessionSidebar.tsx`.
- Prompt Lab: `commands/`, agent execution profile types, `PromptLabView.tsx`, `prompt_lab:*` events.
- HITL: `safety/hitl.rs`, `safety/policy.rs`, `commands/`, `HitlModal.tsx`.
- Voice: `voice/`, `voice/v2/`, `commands/voice.rs`, `VoiceOverlay.tsx`, settings UI.
- Image generation: `image/`, `tools/image_generation.rs`, `commands/`, `ImageProgressChip.tsx`, message media rendering.
- Vision/OCR: `tools/vision.rs`, `preprocessing/image.rs`, sidecar bridge/protocol/Python handlers.
- MCP: `mcp/`, `tools/registry.rs`, `commands/mcp.rs`, `SettingsModal.tsx`.
- Google Workspace: `tools/google_workspace.rs`, `tools/google_workspace_contract.rs`, settings UI/tests.
- Provisioning: `infra/provisioning.rs`, desktop commands, `SetupWizard.tsx`, `stores/provisioning.ts`.
- Fleet/Ironclad: `fleet_control.rs`, `kria-server/src/fleet.rs`, `kria-connection-control`, `FleetMatrix.tsx`, `AddTargetModal.tsx`, `useFleetHeartbeat.ts`, RFCs.
- Remote QEMU/VM/QoS/snapshots: `infra/environment/remote_qemu/mod.rs`, `infra/qos/`, `infra/snapshot/`, `infra/pool/`, `infra/supervisor.rs`, RFCs 001-006.
- Evaluation: `crates/kria-eval`, `tests/e2e`, `ui/src/**/*.test.*`, `crates/kria-core/tests`.
- Export: `components/ExportDropdown.tsx`, `commands/`.

## Low-Token Workflow
- First read `../index.md`, `../architecture/overview.md`, `../architecture/core-runtime.md`, and this guide.
- For structure questions, read `entry-points.md`, `project-graph-summary.md`, and `project-graph.json`.
- For implementation work, inspect source files named in the relevant feature section before editing.
- For GUI work, read `../architecture/gui-cognition-runtime.md` and `../orchestration/gui-execution.md`.
- For safety or HITL work, read `../architecture/safety-hitl-runtime.md` and `../contracts/hitl-mvp/01-boundary.md`.
- For remote/fleet work, read the relevant decision records under `../decisions/rfc/` before changing code.
- After edits, run focused tests for the touched crate/UI area where practical.
