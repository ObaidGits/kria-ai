# KRIA Entry Points

## Desktop App
- `crates/kria-desktop/src/main.rs` is the primary product entry point.
- It registers Tauri plugins, tray integration, and the command handler list.
- It hands off most runtime behavior to `crates/kria-desktop/src/commands/mod.rs` and `kria-core`.

## Desktop Command Layer
- `crates/kria-desktop/src/commands/mod.rs` is the largest integration surface.
- It owns app state wiring for config, memory, tools, safety, voice, sidecar, MCP, model router, local orchestrator, image orchestrator, provisioning, health, local API bridge, and fleet/Ironclad state.
- Most frontend calls from `ui/src/stores/app.ts` terminate here.
- When adding/changing commands, update `main.rs`, the relevant `commands/` submodule, frontend store calls, event listeners, and tests.

## UI App
- `ui/src/index.tsx` renders the Solid app.
- `ui/src/App.tsx` coordinates shell-level UI: chat, prompt lab, settings, HITL modal, voice overlay, setup wizard, fleet matrix, add-target modal, export dropdown, session sidebar, toasts, and forensics panels.
- `ui/src/stores/app.ts` is the primary frontend/backend contract layer.
- `ui/src/stores/provisioning.ts` owns first-run wizard calls and state.

## Agent Runtime
- `crates/kria-core/src/agent/loop_engine/mod.rs` is the main assistant turn engine.
- Supporting submodules in `loop_engine/`: `helpers.rs`, `intent_extractors.rs`, `intent_fallback.rs`, `response_helpers.rs`, `tests.rs`.
- Other agent files: `prompts.rs`, `response_parser.rs`, `planner.rs`, `turn_context.rs`, `turn_gate.rs`, `router.rs`, `onnx_classifier.rs`, `interaction.rs`.
- Agent turns depend on model routing, memory/session history, tool registry, safety/HITL, routing/mounting, and UI streaming events.

## Tool System
- `crates/kria-core/src/tools/registry.rs` defines `ToolRegistry`, schemas, handlers, and registration behavior.
- Tool domains live under `crates/kria-core/src/tools/`.
- Important domains/integrations include image generation, vision, Google Workspace (+ `google_workspace_contract.rs`), MCP-backed tools, proactive/precognitive tools, scheduler, RAG/knowledge, and mount manager.
- Tool calls should route through registry and safety policy rather than direct command execution.

## Safety System
- `crates/kria-core/src/safety/policy.rs` classifies and gates actions.
- `hitl.rs` coordinates human approval.
- `audit.rs` records actions.
- `rollback.rs` creates rollback/snapshot coverage.
- `blacklist.rs` and `pin_guard.rs` add explicit blocking/authorization controls.

## Local LLM / Orchestrator
- `crates/kria-core/src/llm/model_router.rs` dispatches local/cloud/external model requests.
- `crates/kria-core/src/llm/orchestrator/` manages local llama-server runtime, child processes, GPU watchdog, tier strategy, vision strategy, VRAM budgeting, thresholds, telemetry, and runtime state.
- `crates/kria-core/src/resource/gpu_lease.rs` coordinates GPU-heavy workloads.
- `crates/kria-core/src/resource/telemetry.rs` provides resource-level telemetry.

## Image / Vision Runtime
- `crates/kria-core/src/image/orchestrator.rs` is the image generation facade.
- `comfy.rs`, `cloud.rs`, `ws_bridge.rs`, `prompt_enhancer.rs`, `styles.rs`, `mode.rs`, and `capabilities.rs` implement local/cloud generation behavior.
- `crates/kria-core/src/tools/vision.rs` and `preprocessing/image.rs` handle image analysis/OCR and attachment preparation.

## Voice Runtime
- `crates/kria-core/src/voice/pipeline.rs` is the v1 voice pipeline.
- `crates/kria-core/src/voice/v2/` contains the v2 streaming architecture: wake, AEC, STT, TTS, pipeline, playback, post_edit, and sentence splitting.
- Desktop state can expose either active pipeline depending on config and available engines.

## Python Sidecar
- `kria-modules/src/kria_modules/bridge.py` is the sidecar process entry.
- Rust manager: `crates/kria-core/src/sidecar/bridge.rs`.
- Protocol: `crates/kria-core/src/sidecar/protocol.rs`.
- Bootstrap: `crates/kria-core/src/sidecar/bootstrap.rs`.
- Health: `crates/kria-core/src/sidecar/health.rs`.
- Sidecar is optional at startup; dependent tools must degrade gracefully.

## HTTP Server
- `crates/kria-server/src/main.rs` starts standalone server mode.
- `lib.rs` assembles routes.
- `routes.rs`, `ws.rs`, `auth.rs`, and `fleet.rs` implement HTTP, WebSocket, auth, and fleet API behavior.
- Desktop also starts a local API bridge from `commands/local_api.rs`; do not assume server and desktop routes are equivalent.

## Fleet / Remote Execution
- `crates/kria-desktop/src/fleet_control.rs` handles desktop-side fleet/Ironclad state.
- `crates/kria-server/src/fleet.rs` handles server-side fleet API concepts.
- `crates/kria-connection-control/src/manager.rs` and `signer.rs` handle signed leases/connections.
- `crates/kria-core/src/infra/environment/remote_qemu/mod.rs` (with `tests.rs` and `windows_spawn.rs`), `pool/`, `qos/`, `snapshot/`, and `supervisor.rs` support remote QEMU, pooled inventory, QoS, snapshots, and resilient orchestration.
- RFCs 001-006 are required context before modifying these paths.

## Evaluation
- `crates/kria-eval/src/main.rs` starts the evaluation harness.
- `runner.rs`, `suite.rs`, `judge.rs`, `sandbox.rs`, `llm_fixture.rs`, and `report.rs` implement scenarios, fixtures, judging, isolation, and output reports.

## Test Utilities
- `crates/kria-core/src/bin/kria-test.rs` is the test binary entry point.
- `crates/kria-core/src/test_runner.rs` provides test runner utilities.
- `crates/kria-desktop/src/commands/tests.rs` contains desktop command tests.
