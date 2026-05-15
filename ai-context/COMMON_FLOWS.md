# KRIA Common Flows

## Agent Turn Flow
- UI calls `send_message` or prompt-lab equivalent.
- Desktop prepares session, history, attachments, routing context, tools, safety hooks.
- `TurnGate` classifies intent and creates `ResourcePlan`.
- `AgentLoop` coordinates prompt construction, model routing, streaming, tool execution.
- Tool calls flow through `ToolRegistry` → `PolicyEngine` → (HITL if needed) → Execute.
- Results serialized back to agent loop, rendered in UI, persisted to memory.

## Tool Call Flow
- Tool definitions come from `ToolRegistry` plus built-in modules, MCP bridges, and OpenClaw skills.
- Router/mount manager determines which tools are exposed to the model for the current turn.
- On tool call, safety policy classifies the action.
- GREEN actions can execute directly; higher-risk actions may require HITL approval, auditing, and rollback snapshots; BLACK actions are blocked.
- Tool output is serialized back into the agent loop and rendered in the UI.

## HITL Flow
- Backend emits an approval request for risky operations.
- `HitlModal` displays action details and collects approve/deny input.
- Backend resumes or rejects the pending operation based on the decision.
- Auditing should record both request and outcome.

## Voice Flow
- User toggles voice from UI or tray.
- Desktop uses configured voice engine and device/model settings.
- v1 path handles capture, VAD/STT, agent turn, TTS/playback, and metrics.
- v2 path exposes streaming state, wake/AEC/STT/TTS traits, sentence-level playback, post-edit, and session state where enabled.
- UI listens to `voice:*` events and renders `VoiceOverlay`.

## Image Generation Flow
- User/tool requests image generation.
- `tools/image_generation.rs` delegates to `ImageOrchestrator`.
- Orchestrator chooses local ComfyUI or cloud fallback according to mode/capabilities/config.
- WebSocket bridge streams generation progress to desktop events.
- UI shows status through image progress components and persists/render media details with messages.

## Vision/OCR Flow
- Image attachments are normalized and may be thumbnailed/preprocessed natively.
- Vision tools resolve file paths against attachment/data directories.
- Sidecar image analysis/OCR is attempted first when available.
- Native metadata and CLI fallback paths are used when sidecar capabilities are unavailable.
- Token budgets and visual metadata caps are passed as hints to avoid prompt bloat.

## MCP Tool Flow
- MCP server configs are loaded from settings/config.
- Server manager starts/stops configured MCP servers and tracks runtime state.
- Capability discovery summarizes available remote tools.
- Tool bridge exposes MCP tools as KRIA tool definitions with shaped payloads.
- UI settings show and control MCP server state.

## OpenClaw Skill Flow
- Skills fetched from ClawHub registry or local filesystem.
- Skill manifest declares capabilities (filesystem_read, subprocess, network, etc.).
- Capability resolver matches user request to skills via hybrid BM25 + dense retrieval.
- Container pool provides warm Docker container for execution.
- Audit ledger records invocation with HMAC signature.
- Trust tier determines auto-approval vs HITL requirement.
- UI shows skill marketplace via `RemoteSkillCard` component.

## Google Workspace Flow
- Google Workspace settings/auth configuration is controlled through desktop settings UI.
- Backend tool domain handles workspace operations through the shared tool registry.
- Contract types are defined in `tools/google_workspace_contract.rs`.
- UI tests cover Google settings behavior; preserve payload compatibility when changing settings.

## Provisioning / First Run Flow
- `SetupWizard` and `stores/provisioning.ts` drive first-run setup.
- Steps include hardware detection, backend choice, model download, sidecar setup, server verification, and completion.
- Backend emits/downloads progress and stores completion state.
- Provisioning should be restartable/idempotent and recover from partial failures.

## Local API Bridge Flow
- Desktop can start a local Axum bridge with `/api/health` and `/api/chat` endpoints (`commands/local_api.rs`).
- Health registry records whether the bridge is listening, reused, or degraded.
- Treat this as a desktop-adjacent API surface, not the standalone server's full runtime.

## Fleet / Ironclad Flow
- Desktop and server expose fleet-related state for targets, leases, heartbeats, and forensics.
- `FleetMatrix` reads commander/lease information from settings/status and heartbeat hooks.
- Add-target UI and fleet-control backend manage target metadata.
- Connection-control signing/lease primitives support authenticated target control.
- Remote QEMU, inventory pooling, snapshots, QoS, and resilient orchestration follow architecture in `docs/ARCHITECTURE.md`.

## Automation Flow
- Automation modules provide workflows, scheduler, macro recording, proactive triggers, and event bus.
- Scheduled/proactive actions still need normal safety classification before execution.
- New automation triggers should avoid bypassing HITL/audit/rollback paths.

## Evaluation Flow
- `kria-eval` loads suites, prepares sandbox/fixtures, executes scenarios, judges output, and writes reports.
- `llm_fixture.rs` provides LLM fixtures for evaluation scenarios.
- E2E UI tests live under `tests/e2e`; component/store tests live under `ui/src`.
- Prefer adding focused tests around command contracts, tool behavior, and UI state regressions when modifying core flows.

## Shutdown Flow
- `shutdown_runtime` command orchestrates cleanup:
  - Stops voice pipeline
  - Shuts down MCP servers
  - Stops orchestrator (llama-server)
  - Shuts down OpenClaw container pool (prevents Docker container leaks)
  - Flushes memory to disk
  - Emits shutdown complete event

## Flow Change Watch
<!-- AI-CONTEXT:START generated-change-watch -->
- Last checked: 2026-05-15 19:08 UTC
- Commit: `9c8f824d0631`
- Reason: logic or control-flow changes.
- `crates/kria-core/src/agent/curiosity/mod.rs` changed flow-adjacent logic; review exact behavior (needs verification).
- `crates/kria-core/src/agent/environment_grounder.rs` changed flow-adjacent logic; review exact behavior (needs verification).
- `crates/kria-core/src/agent/execution_verifier.rs` changed flow-adjacent logic; review exact behavior (needs verification).
- `crates/kria-core/src/agent/execution_verifier_impl.rs` added; include only if it becomes an important navigation point.
- `crates/kria-core/src/agent/goal_tree.rs` added; include only if it becomes an important navigation point.
- `crates/kria-core/src/agent/gui_planner.rs` added; include only if it becomes an important navigation point.
- `crates/kria-core/src/agent/gui_wiring.rs` changed flow-adjacent logic; review exact behavior (needs verification).
- `crates/kria-core/src/agent/htn_executor.rs` changed flow-adjacent logic; review exact behavior (needs verification).
- `crates/kria-core/src/agent/htn_integration.rs` changed flow-adjacent logic; review exact behavior (needs verification).
- `crates/kria-core/src/agent/intent_compiler.rs` changed flow-adjacent logic; review exact behavior (needs verification).
- `crates/kria-core/src/agent/intent_compiler_llm.rs` added; include only if it becomes an important navigation point.
- `crates/kria-core/src/agent/intent_gate.rs` added; include only if it becomes an important navigation point.
- 54 more relevant files omitted for brevity.
- Next action: Use `ai-context/prompts/update_flows.txt` only when behavior changed.
<!-- AI-CONTEXT:END generated-change-watch -->
