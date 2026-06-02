# KRIA Graph Query Guide

**Last updated:** 2026-05-27

Use `rg` first. This repo is large enough that ad hoc browsing wastes time and misses call sites.

## What Depends On X?

- Rust module or type: `rg -n "TypeName|function_name|mod_name" crates/`.
- Tauri command: search command name in `crates/kria-desktop/src/main.rs`, `crates/kria-desktop/src/commands/`, `ui/src/stores/app.ts`, and UI components.
- Stream/UI event: search both backend emitters and frontend listeners.
- Tool name: search `tools/registry.rs`, the specific `tools/*.rs` module, safety policy, preflight, prompt/tool mounting, tests, and UI tool rendering.
- HITL/decision runtime: search `collaborative_decision.rs`, `execution_gate.rs`, `resume_executor.rs`, `continuation_reentry.rs`, `resource_lease.rs`, `DecisionActionCenter.tsx`, and `HitlModal.tsx`.
- GUI cognition behavior: search `semantic_workflow.rs`, `execution_mode_reasoner.rs`, `workflow_intent_contract.rs`, `gui_wiring.rs`, `gui_substrate_planner.rs`, `verifier_authority.rs`, and `hybrid_synchronization.rs`.

## What Breaks If I Change Y?

- `ToolRegistry` or tool schema changes can break model tool calls, response parsing, safety classification, MCP bridging, prompt lab, resume capability, and UI tool display.
- Desktop command payload changes can break `ui/src/stores/app.ts`, components, tests, event listeners, and local API bridge assumptions.
- Safety policy changes can alter tool execution, HITL modal behavior, decision action center behavior, audit logs, leases, rollback expectations, GUI automation, and fleet/remote operations.
- Execution gate or collaborative decision changes can break stale-decision rejection, action-center resume, JSONL replay, resource leases, and HITL tests.
- GUI cognition changes can break structural vs visible workflow fidelity, browser/IDE workflow handling, observable completion, and GUI eval reports.
- Model orchestration changes can affect chat, vision, image generation, GPU leases, hardware tiering, provider health, startup, and idle behavior.
- Sidecar protocol changes require synchronized Rust and Python updates.
- Remote/fleet changes require checking connection-control, server routes, desktop device UI, leases, inventory, QoS, pool, snapshot, and supervisor behavior.

## Trace Common Flows

- UI to backend: component -> `ui/src/stores/app.ts` or provisioning store -> Tauri `invoke` -> desktop command -> `kria-core`.
- Backend to UI: core/desktop event -> Tauri `emit` -> `app.ts` listener or component listener -> component state/render.
- Chat: `ChatView` -> `appStore` -> desktop `chat.rs`/commands -> `AgentLoop` -> `ModelRouter` -> parser/tools/safety/memory -> streamed events.
- Tool execution: model output -> `response_parser` -> `ToolRegistry` -> `ExecutionGate`/policy/HITL/audit/leasing -> handler -> `ToolResult` -> result synthesis.
- HITL action-center resume: durable `InteractionDecision` -> user resolution -> `ResumeExecutor` -> resume gate -> lease acquisition -> one deterministic local tool action.
- GUI workflow: user prompt -> intent/compiler/semantic workflow metadata -> execution mode/contract -> substrate planning -> GUI workflow/stage execution -> verifier/observable completion.
- Image: tool/command -> `ImageOrchestrator` -> local ComfyUI or cloud fallback -> WebSocket progress -> desktop event -> `ImageProgressChip`/message media.
- Voice: UI/tray -> desktop command/event -> v1/v2 voice pipeline -> STT -> agent turn -> TTS/playback -> `VoiceOverlay`.
- MCP: settings/config -> MCP server manager -> discovery -> tool bridge -> `ToolRegistry` -> agent loop.
- Eval: `kria-eval` CLI -> suite runner -> sandbox/fixtures/judge/report -> `tests-logs` or `testing/eval_reports` artifacts.

## Where Is This Feature Implemented?

| Feature | Start here |
|---|---|
| Chat/session/history | `commands/chat.rs`, `agent/loop_engine/mod.rs`, `memory/`, `ui/src/stores/app.ts`, `ChatView.tsx`, `SessionSidebar.tsx` |
| Prompt Lab | `PromptLabView.tsx`, desktop commands, agent execution profile types, `prompt_lab:*` events |
| HITL approval | `safety/hitl.rs`, `safety/policy.rs`, `agent/execution_gate.rs`, `HitlModal.tsx` |
| Durable decisions/action center | `agent/collaborative_decision.rs`, `agent/resume_executor.rs`, `agent/continuation_reentry.rs`, `DecisionActionCenter.tsx` |
| Runtime authority | `docs/orchestration/runtime-authority.md`, `agent/execution_authority.rs`, `agent/execution_gate.rs`, `agent/resource_lease.rs` |
| GUI cognition | `agent/semantic_workflow.rs`, `execution_mode_reasoner.rs`, `workflow_intent_contract.rs`, `gui_wiring.rs`, `gui_substrate_planner.rs` |
| GUI verification | `verifier_authority.rs`, `execution_verifier*.rs`, `observable_completion/`, `hybrid_synchronization.rs` |
| Browser/media cognition | `browser_cognition.rs`, `browser_media_governance.rs`, `tools/internet.rs`, `tools/browser_agent.rs` |
| IDE/code workflow | `ide_cognition.rs`, `tools/developer.rs`, `tools/shell.rs`, `tools/file_ops.rs` |
| Voice | `voice/`, `voice/v2/`, `commands/voice.rs`, `VoiceOverlay.tsx` |
| Image generation | `image/`, `tools/image_generation.rs`, `commands/image_chat.rs`, `ImageProgressChip.tsx` |
| Vision/OCR | `tools/vision.rs`, `tools/vision_automation.rs`, `preprocessing/image.rs`, sidecar processors |
| MCP | `mcp/`, `tools/registry.rs`, `commands/mcp.rs`, `SettingsModal.tsx` |
| Google Workspace | `tools/google_workspace.rs`, `tools/google_workspace_contract.rs`, desktop Google commands/settings/tests |
| N8N/OpenClaw | `tools/n8n.rs`, `crates/kria-core/src/n8n/`, `crates/kria-core/src/openclaw/`, `commands/n8n.rs`, `commands/openclaw.rs`, `openclaw-substrate/`, integration docs |
| Provisioning | `infra/provisioning.rs`, desktop provisioning commands, `SetupWizard.tsx`, `stores/provisioning.ts` |
| Device/fleet/remote | `device_control.rs`, `kria-connection-control`, server inventory/routes, `DeviceMatrix.tsx`, remote infra modules |
| Evaluation | `crates/kria-eval`, `testing/suites/playwright`, `crates/kria-core/tests` |

## Low-Token Workflow

1. Read `../index.md`, `../architecture/overview.md`, `../architecture/core-runtime.md`, then this guide.
2. Read `entry-points.md` for ownership and `project-graph-summary.md` for flow shape.
3. Use `project-graph.json` for machine-readable adjacency only.
4. For implementation work, inspect source files named in the relevant feature row before editing.
5. For GUI work, include `../architecture/gui-cognition-runtime.md` and `../orchestration/gui-execution.md`.
6. For HITL work, include `../architecture/safety-hitl-runtime.md` and `../contracts/hitl-mvp/`.
7. For eval work, include `../evaluations/overview.md` and the relevant `crates/kria-eval/src/*_eval` tree.
8. After edits, run focused tests for the touched crate/UI area where practical.
