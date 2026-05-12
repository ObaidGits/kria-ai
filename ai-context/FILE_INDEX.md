# KRIA File Index

## Core Library (kria-core)

### Agent
- `crates/kria-core/src/agent/mod.rs` -> agent types and exports.
- `crates/kria-core/src/agent/loop_engine/mod.rs` -> main ReAct loop with semantic injection.
- `crates/kria-core/src/agent/loop_engine/helpers.rs` -> loop helpers.
- `crates/kria-core/src/agent/loop_engine/intent_extractor.rs` -> intent extraction.
- `crates/kria-core/src/agent/loop_engine/intent_fallback.rs` -> intent fallback logic.
- `crates/kria-core/src/agent/loop_engine/response_helpers.rs` -> response formatting.
- `crates/kria-core/src/agent/planner.rs` -> multi-step planner.
- `crates/kria-core/src/agent/prompts.rs` -> system prompts.
- `crates/kria-core/src/agent/response_parser.rs` -> parse LLM output.
- `crates/kria-core/src/agent/router.rs` -> intent routing.
- `crates/kria-core/src/agent/turn_context.rs` -> turn state.
- `crates/kria-core/src/agent/turn_gate.rs` -> resource planning and admission.
- `crates/kria-core/src/agent/onnx_classifier.rs` -> ONNX intent classification.
- `crates/kria-core/src/agent/executive/controller.rs` -> executive control.
- `crates/kria-core/src/agent/perception/mod.rs` -> perception bus.
- `crates/kria-core/src/agent/curiosity/mod.rs` -> curiosity loop.
- `crates/kria-core/src/agent/failure_analyzer/mod.rs` -> failure analysis.
- `crates/kria-core/src/agent/planner_v2/mod.rs` -> planner v2.
- `crates/kria-core/src/agent/prompt_optimizer/mod.rs` -> prompt optimization.
- `crates/kria-core/src/agent/self_model/mod.rs` -> self modeling.
- `crates/kria-core/src/agent/skill_compiler/mod.rs` -> skill compilation.
- `crates/kria-core/src/agent/uncertainty/mod.rs` -> uncertainty quantification.
- `crates/kria-core/src/agent/working_set/mod.rs` -> working set management.
- `crates/kria-core/src/agent/world_model/mod.rs` -> world modeling.

### Tools
- `crates/kria-core/src/tools/mod.rs` -> tool exports.
- `crates/kria-core/src/tools/registry.rs` -> tool registration and lookup.
- `crates/kria-core/src/tools/app_lifecycle.rs` -> app install/uninstall/launch.
- `crates/kria-core/src/tools/desktop.rs` -> desktop integration.
- `crates/kria-core/src/tools/developer.rs` -> developer tools.
- `crates/kria-core/src/tools/disk.rs` -> disk management.
- `crates/kria-core/src/tools/documents.rs` -> document parsing.
- `crates/kria-core/src/tools/exec.rs` -> code execution.
- `crates/kria-core/src/tools/file_ops.rs` -> file read/write/copy/move.
- `crates/kria-core/src/tools/google_workspace.rs` -> Gmail/Calendar/Drive.
- `crates/kria-core/src/tools/google_workspace_contract.rs` -> Google contract types.
- `crates/kria-core/src/tools/i18n.rs` -> internationalization.
- `crates/kria-core/src/tools/image_generation.rs` -> image generation entry.
- `crates/kria-core/src/tools/interaction.rs` -> notifications, clipboard.
- `crates/kria-core/src/tools/internet.rs` -> web search, fetch, download.
- `crates/kria-core/src/tools/knowledge.rs` -> facts, snippets.
- `crates/kria-core/src/tools/mount_manager.rs` -> tool mounting.
- `crates/kria-core/src/tools/news.rs` -> news feed.
- `crates/kria-core/src/tools/packages.rs` -> package management.
- `crates/kria-core/src/tools/power.rs` -> shutdown, reboot, sleep.
- `crates/kria-core/src/tools/precognitive.rs` -> predictive tools.
- `crates/kria-core/src/tools/proactive.rs` -> proactive suggestions.
- `crates/kria-core/src/tools/process.rs` -> process management.
- `crates/kria-core/src/tools/rag.rs` -> RAG queries.
- `crates/kria-core/src/tools/scheduler.rs` -> scheduled tasks.
- `crates/kria-core/src/tools/shell.rs` -> shell commands.
- `crates/kria-core/src/tools/system_config.rs` -> system settings.
- `crates/kria-core/src/tools/system_info.rs` -> system telemetry.
- `crates/kria-core/src/tools/vision.rs` -> image analysis.
- `crates/kria-core/src/tools/browser_agent.rs` -> browser automation.

### Safety
- `crates/kria-core/src/safety/mod.rs` -> safety exports.
- `crates/kria-core/src/safety/policy.rs` -> risk classification.
- `crates/kria-core/src/safety/hitl.rs` -> human-in-the-loop.
- `crates/kria-core/src/safety/audit.rs` -> audit logging.
- `crates/kria-core/src/safety/rollback.rs` -> backup/restore.
- `crates/kria-core/src/safety/blacklist.rs` -> blocked actions.
- `crates/kria-core/src/safety/pin_guard.rs` -> PIN verification.

### LLM
- `crates/kria-core/src/llm/mod.rs` -> LLM exports.
- `crates/kria-core/src/llm/router.rs` -> model routing.
- `crates/kria-core/src/llm/local_client.rs` -> llama.cpp client.
- `crates/kria-core/src/llm/cloud_client.rs` -> cloud API client.
- `crates/kria-core/src/llm/model_manager.rs` -> model loading.
- `crates/kria-core/src/llm/tokenizer.rs` -> tokenization.

### LLM Orchestrator
- `crates/kria-core/src/llm/orchestrator/mod.rs` -> orchestration entry.
- `crates/kria-core/src/llm/orchestrator/gpu_watchdog.rs` -> VRAM monitoring.
- `crates/kria-core/src/llm/orchestrator/server_manager.rs` -> llama-server lifecycle.
- `crates/kria-core/src/llm/orchestrator/telemetry.rs` -> resource telemetry.
- `crates/kria-core/src/llm/orchestrator/strategy.rs` -> degradation strategy.
- `crates/kria-core/src/llm/orchestrator/tier_strategy.rs` -> hardware tier strategy.
- `crates/kria-core/src/llm/orchestrator/vision_strategy.rs` -> vision model strategy.
- `crates/kria-core/src/llm/orchestrator/vram_budget.rs` -> VRAM allocation.

### Image
- `crates/kria-core/src/image/mod.rs` -> image exports.
- `crates/kria-core/src/image/orchestrator.rs` -> generation orchestration.
- `crates/kria-core/src/image/comfyui.rs` -> ComfyUI backend.
- `crates/kria-core/src/image/cloud.rs` -> cloud fallback.
- `crates/kria-core/src/image/ws_bridge.rs` -> WebSocket progress.

### Voice
- `crates/kria-core/src/voice/mod.rs` -> voice exports.
- `crates/kria-core/src/voice/pipeline.rs` -> v1 pipeline.
- `crates/kria-core/src/voice/stt.rs` -> speech-to-text.
- `crates/kria-core/src/voice/tts.rs` -> text-to-speech.
- `crates/kria-core/src/voice/vad.rs` -> voice activity detection.
- `crates/kria-core/src/voice/v2/mod.rs` -> v2 streaming pipeline.

### Memory
- `crates/kria-core/src/memory/mod.rs` -> memory exports.
- `crates/kria-core/src/memory/manager.rs` -> memory operations.
- `crates/kria-core/src/memory/store.rs` -> SQLite store.
- `crates/kria-core/src/memory/embeddings.rs` -> vector embeddings.
- `crates/kria-core/src/memory/rag.rs` -> RAG retrieval.
- `crates/kria-core/src/memory/facts.rs` -> fact storage.

### OpenClaw
- `crates/kria-core/src/openclaw/mod.rs` -> OpenClaw exports.
- `crates/kria-core/src/openclaw/container_pool.rs` -> Docker container pool.
- `crates/kria-core/src/openclaw/skill_registry.rs` -> skill management.
- `crates/kria-core/src/openclaw/capability_resolver.rs` -> skill matching.
- `crates/kria-core/src/openclaw/audit_ledger.rs` -> invocation logging.
- `crates/kria-core/src/openclaw/types.rs` -> skill types and manifests.

### MCP
- `crates/kria-core/src/mcp/mod.rs` -> MCP exports.
- `crates/kria-core/src/mcp/client.rs` -> MCP client.
- `crates/kria-core/src/mcp/server_manager.rs` -> server lifecycle.
- `crates/kria-core/src/mcp/tool_bridge.rs` -> tool bridging.
- `crates/kria-core/src/mcp/protocol.rs` -> JSON-RPC protocol.

### Infra
- `crates/kria-core/src/infra/event_bus.rs` -> event broadcasting.
- `crates/kria-core/src/infra/health.rs` -> health registry.
- `crates/kria-core/src/infra/pool/mod.rs` -> resource pooling.
- `crates/kria-core/src/infra/snapshot/mod.rs` -> snapshot support.
- `crates/kria-core/src/infra/supervisor.rs` -> runtime supervision.
- `crates/kria-core/src/infra/environment/remote_qemu/mod.rs` -> remote execution.

### Resource
- `crates/kria-core/src/resource/gpu_lease.rs` -> GPU lease manager.
- `crates/kria-core/src/resource/telemetry.rs` -> resource telemetry.

## Desktop Backend (kria-desktop)
- `crates/kria-desktop/src/main.rs` -> Tauri entry and command registration.
- `crates/kria-desktop/src/commands/mod.rs` -> command module registration.
- `crates/kria-desktop/src/commands/app_commands.rs` -> app-level commands.
- `crates/kria-desktop/src/commands/app_state.rs` -> desktop app state.
- `crates/kria-desktop/src/commands/automation.rs` -> automation commands.
- `crates/kria-desktop/src/commands/chat.rs` -> chat commands.
- `crates/kria-desktop/src/commands/colab.rs` -> Colab commands.
- `crates/kria-desktop/src/commands/fleet_enrollment.rs` -> fleet enrollment.
- `crates/kria-desktop/src/commands/fleet_tools.rs` -> fleet tool commands.
- `crates/kria-desktop/src/commands/google_workspace.rs` -> Google Workspace.
- `crates/kria-desktop/src/commands/image_chat.rs` -> image chat.
- `crates/kria-desktop/src/commands/ironclad.rs` -> Ironclad commands.
- `crates/kria-desktop/src/commands/local_api.rs` -> local API bridge.
- `crates/kria-desktop/src/commands/mcp.rs` -> MCP commands.
- `crates/kria-desktop/src/commands/openclaw.rs` -> OpenClaw commands.
- `crates/kria-desktop/src/commands/provisioning.rs` -> provisioning.
- `crates/kria-desktop/src/commands/runtime.rs` -> runtime commands (including shutdown).
- `crates/kria-desktop/src/commands/sessions.rs` -> session management.
- `crates/kria-desktop/src/commands/telegram.rs` -> Telegram commands.
- `crates/kria-desktop/src/commands/voice.rs` -> voice commands.
- `crates/kria-desktop/src/fleet_control.rs` -> fleet state.
- `crates/kria-desktop/src/tray.rs` -> tray menu.

## Server / Eval / Connection Control
- `crates/kria-server/src/main.rs` -> standalone server entry.
- `crates/kria-server/src/routes.rs` -> HTTP handlers.
- `crates/kria-server/src/ws.rs` -> WebSocket handlers.
- `crates/kria-server/src/fleet.rs` -> fleet API.
- `crates/kria-eval/src/runner.rs` -> evaluation runner.
- `crates/kria-eval/src/judge.rs` -> evaluator/judge.
- `crates/kria-connection-control/src/manager.rs` -> connection/lease manager.

## Frontend
- `ui/src/App.tsx` -> app shell.
- `ui/src/stores/app.ts` -> primary frontend state.
- `ui/src/stores/provisioning.ts` -> provisioning wizard state.
- `ui/src/components/ChatView.tsx` -> chat surface.
- `ui/src/components/MessageBubble.tsx` -> message rendering.
- `ui/src/components/HitlModal.tsx` -> approval dialog.
- `ui/src/components/VoiceOverlay.tsx` -> voice UI.
- `ui/src/components/PromptLabView.tsx` -> prompt lab.
- `ui/src/components/SettingsModal.tsx` -> settings UI.
- `ui/src/components/SetupWizard.tsx` -> first-run wizard.
- `ui/src/components/FleetMatrix.tsx` -> fleet matrix.
- `ui/src/components/RemoteSkillCard.tsx` -> OpenClaw skill card.
- `ui/src/locales/*.json` -> localized strings (en, ar, de, es, fr, hi, zh).

## Sidecar / Tests / Docs
- `kria-modules/src/kria_modules/bridge.py` -> Python JSON-RPC sidecar.
- `kria-modules/src/kria_modules/processors/` -> Python processors.
- `tests/e2e/` -> Playwright E2E tests.
- `docs/` -> documentation (see docs/README for structure).

## File Index Change Watch
<!-- AI-CONTEXT:START generated-change-watch -->
- Last checked: 2026-05-12 18:34 UTC
- Commit: `4b327340d467`
- Reason: important file-level changes.
- `crates/kria-core/src/orchestrator/mod.rs` added; include only if it becomes an important navigation point.
- Next action: Use `ai-context/prompts/update_index.txt` only for important file additions/removals.
<!-- AI-CONTEXT:END generated-change-watch -->
