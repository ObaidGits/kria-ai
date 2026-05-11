# KRIA Codebase Map

## Core Rust Modules (kria-core)
- `agent` -> interaction types, loop engine (modular directory with helpers, intent extractors, intent fallback, response helpers, and tests), planner, prompts, response parser, router fallback, turn context/gate, ONNX classifier, executive controller, perception, curiosity, failure analyzer, planner v2, prompt optimizer, self model, skill compiler, uncertainty, working set, world model.
- `tools` -> registry plus all built-in tool domains: app lifecycle, desktop, developer, disk, documents, exec, file ops, Google Workspace (+ contract), i18n, image generation, interaction, internet, knowledge, mount manager, news, packages, power, precognitive, proactive, process, RAG, scheduler, shell, system config/info, vision, browser agent.
- `safety` -> audit, blacklist, HITL, PIN guard, policy, and rollback.
- `llm` -> cloud/local clients, model manager/router, server binary handling, tokenizer utilities, and orchestrator.
- `llm/orchestrator` -> child guard, GPU watchdog, runtime, server manager, strategy, telemetry, threshold, tier strategy, vision strategy, and VRAM budget.
- `image` -> backend abstraction, capabilities, ComfyUI, cloud fallback, mode, orchestrator, prompt enhancer, styles, swap, and WebSocket bridge.
- `voice` -> capture, metrics, pipeline, playback, STT, tier, TTS, VAD, and v2 pipeline modules.
- `voice/v2` -> AEC, pipeline, playback, post-edit, sentence splitting, STT, TTS, and wake detection.
- `memory` -> decay, embeddings, facts, manager, RAG, retrieval, semantic parser, SQLite store, and vectors.
- `routing` -> cache, decision, domain, embedding, out-of-domain handling, segmentation, tracing, and verbs.
- `platform` -> app registry, contacts, OS detection, inbox (adapter, approval, egress, media, policy, queue), intent (capability, dispatcher, grammar, linux/macos/windows resolution, scheme), OS abstractions, paths, sandbox, Telegram, and VRAM.
- `infra` -> circuit breaker, component, downloads, environment (docker, local, remote QEMU directory with tests/windows_spawn), event bus, hardware profiler, health, isolation, logging, observability, pipeline traces, pools, provisioning, QoS, sandbox, snapshots, and supervisor.
- `mcp` -> capability discovery, client, payload shaper, protocol, server manager, and tool bridge.
- `openclaw` -> container pool, skill registry, capability resolver, audit ledger, event stream, ClawHub client.
- `automation` -> event bus, macro recorder, proactive automation, scheduler, and workflows.
- `resource` -> GPU lease manager and telemetry.
- `preprocessing` -> code, document, image, web, and token-budget preprocessing.
- `plugin` -> runtime support for native plugin loading.
- `sidecar` -> bootstrap, bridge, health, and protocol modules.
- `bin` -> `kria-test` binary entry point.
- `test_runner` -> test runner utilities.

## Desktop Backend (kria-desktop)
- Runtime initialization wires config, paths, memory, embeddings, tool registry, sidecar, MCP, safety, model router, orchestrator, image orchestrator, voice pipeline, health registry, provisioning, OpenClaw substrate, and fleet state.
- Commands are organized in a modular directory (`commands/`) covering: app commands/state, automation, chat, Colab/dispatch, command helpers, constants, fleet enrollment/tools, Google Workspace, history helpers, image chat, Ironclad, local API, MCP, media, orchestrator helpers, openclaw, provisioning, runtime, sessions, Telegram, tests, tool result helpers, voice, voice diagnostics, voice runtime helpers, analytics, test runner.
- The desktop process emits UI events for streaming agent output, stages, HITL, voice telemetry, image progress, orchestrator/Colab status, Ironclad forensics, OpenClaw events, and tray actions.

## Server Areas (kria-server)
- `auth.rs` -> server authentication helpers.
- `routes.rs` -> HTTP route handlers.
- `ws.rs` -> WebSocket route setup.
- `fleet.rs` -> fleet API/domain surface.
- The standalone server is not the same as the desktop runtime; desktop remains authoritative for full assistant behavior.

## UI Areas
- `stores/app.ts` -> central app state, Tauri invokes, event listeners, sessions, health, settings, HITL, voice, image, MCP, Google, OpenClaw, and runtime status.
- `stores/provisioning.ts` -> first-run wizard state and provisioning commands.
- `stores/i18n.ts` + `locales/*.json` -> localization (en, ar, de, es, fr, hi, zh).
- `components/ChatView.tsx` -> primary chat UI.
- `components/MessageBubble.tsx` -> assistant/user/tool/media message rendering.
- `components/HitlModal.tsx` -> human approval UX.
- `components/VoiceOverlay.tsx` -> voice session UX.
- `components/PromptLabView.tsx` -> prompt experimentation surface.
- `components/SettingsModal.tsx` -> settings, MCP, Google, voice, OpenClaw, and runtime configuration.
- `components/SetupWizard.tsx` -> first-run provisioning UX.
- `components/FleetMatrix.tsx` and `AddTargetModal.tsx` -> Ironclad/fleet target UI.
- `components/ImageProgressChip.tsx` -> image generation status UX.
- `components/ExportDropdown.tsx` -> export actions UX.
- `components/SessionSidebar.tsx` -> session list/navigation UX.
- `components/RemoteSkillCard.tsx` -> OpenClaw skill marketplace UI.
- `hooks/useFleetHeartbeat.ts` -> fleet heartbeat hook.

## Documentation Structure
- `docs/ARCHITECTURE.md` — System architecture (detailed)
- `docs/TOOLS.md` — Tool system guide (detailed)
- `docs/OPENCLAW.md` — OpenClaw integration (detailed)
- `docs/SAFETY.md` — Safety model (detailed)
- `docs/DEVELOPMENT.md` — Development workflow (detailed)
- `docs/SYSTEM_DESIGN.md` — System design reference
- `docs/FAQ.md` — Frequently asked questions
- `docs/MEMORY.md` — Memory system (brief)
- `docs/HARDWARE.md` — GPU orchestration (brief)
- `docs/VOICE.md` — Voice pipeline (brief)
- `docs/EVAL.md` — Eval harness (brief)
- `docs/DEPLOYMENT.md` — Packaging (brief)
- `docs/ADR/` — Architecture Decision Records
  - `ADR_001_E2E_EVAL_HARNESS.md`
  - `ADR_002_TOOL_EXECUTION_LAYER_OVERHAUL.md`

## Structure Change Watch
<!-- AI-CONTEXT:START generated-change-watch -->
- Last checked: 2026-05-11 13:47 UTC
- Commit: `55bb04a50c32`
- Reason: structural changes.
- `Cargo.toml` changed; review stable summary only if its public role changed.
- `crates/kria-core/src/agent/ml_orchestrator/async_wrapper.rs` added; include only if it becomes an important navigation point.
- `crates/kria-core/src/agent/ml_orchestrator/code_gate.rs` added; include only if it becomes an important navigation point.
- `crates/kria-core/src/agent/ml_orchestrator/helpers_template.rs` added; include only if it becomes an important navigation point.
- `crates/kria-core/src/agent/ml_orchestrator/integrity.rs` added; include only if it becomes an important navigation point.
- `crates/kria-core/src/agent/ml_orchestrator/ledger.rs` added; include only if it becomes an important navigation point.
- `crates/kria-core/src/agent/ml_orchestrator/mod.rs` added; include only if it becomes an important navigation point.
- `crates/kria-core/src/agent/ml_orchestrator/plan_parser.rs` added; include only if it becomes an important navigation point.
- `crates/kria-core/src/agent/ml_orchestrator/poller.rs` added; include only if it becomes an important navigation point.
- `crates/kria-core/src/agent/ml_orchestrator/sync_cell.rs` added; include only if it becomes an important navigation point.
- `crates/kria-core/src/agent/ml_orchestrator/types.rs` added; include only if it becomes an important navigation point.
- `crates/kria-core/src/config.rs` changed; review stable summary only if its public role changed.
- 28 more relevant files omitted for brevity.
- Next action: Use `ai-context/prompts/update_map.txt` only if stable structure changed.
<!-- AI-CONTEXT:END generated-change-watch -->
