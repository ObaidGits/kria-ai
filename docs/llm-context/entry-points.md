# KRIA Entry Points

**Last updated:** 2026-05-27

Use this as a first map. Confirm details in source before changing behavior.

## Workspace

- `Cargo.toml` defines the Rust workspace: `kria-core`, `kria-desktop`, `kria-server`, `kria-eval`, `kria-connection-control`, `kria-uinput-daemon`, and `kria-test-app`.
- `ui/` is the SolidJS/Tauri frontend.
- `kria-modules/` is the Python sidecar package.
- `sidecars/kria-vision/` is the standalone vision sidecar.
- `openclaw-substrate/` is the OpenClaw substrate bridge.

## Desktop App

- `crates/kria-desktop/src/main.rs` is the Tauri product entry point.
- It registers plugins, tray integration, managed state, and command handlers.
- `crates/kria-desktop/src/commands/mod.rs` is the primary backend integration surface.
- Important command submodules include `chat.rs`, `gui_automation_control.rs`, `runtime.rs`, `runtime_status.rs`, `test_runner.rs`, `voice.rs`, `mcp.rs`, `providers.rs`, `provisioning.rs`, `local_api.rs`, `n8n.rs`, `openclaw.rs`, `device_tools.rs`, and `device_enrollment.rs`.

## UI App

- `ui/src/index.tsx` renders the Solid app.
- `ui/src/App.tsx` coordinates the shell-level product UI.
- `ui/src/stores/app.ts` is the main frontend/backend contract layer for Tauri invokes and events.
- `ui/src/stores/provisioning.ts` owns first-run provisioning state.
- Key components:
  - `ChatView.tsx`
  - `MessageBubble.tsx`
  - `DecisionActionCenter.tsx`
  - `HitlModal.tsx`
  - `GuiWorkflowViewer.tsx`
  - `PromptLabView.tsx`
  - `SettingsModal.tsx`
  - `SetupWizard.tsx`
  - `DeviceMatrix.tsx`
  - `ExecutiveDashboard.tsx`
  - `TestRunnerDashboard.tsx`

## Agent Runtime

- `crates/kria-core/src/agent/loop_engine/mod.rs` is the main assistant turn engine.
- Supporting loop modules: `helpers.rs`, `intent_extractors.rs`, `intent_fallback.rs`, and `response_helpers.rs`.
- Turn gating and routing: `turn_gate.rs`, `intent_gate.rs`, `router.rs`, `onnx_classifier.rs`, `turn_context.rs`, `turn_memory.rs`, and `crates/kria-core/src/routing/`.
- Result shaping: `response_parser.rs`, `result_synthesizer.rs`, `synthesis_prompt.rs`, and `execution_transparency/`.

## GUI Cognition Runtime

- `gui_wiring.rs` wires GUI task execution into the broader runtime.
- `semantic_workflow.rs`, `execution_mode_reasoner.rs`, and `workflow_intent_contract.rs` provide semantic workflow metadata, mode decisions, and declarative workflow contracts.
- `gui_substrate_planner.rs`, `gui_planner.rs`, `stage_executor.rs`, `goal_tree.rs`, and `workflow_compiler.rs` plan bounded GUI/workflow actions.
- `verifier_authority.rs`, `execution_verifier*.rs`, `observable_completion/`, and `hybrid_synchronization.rs` model evidence, authority, visible completion, and structural/visible synchronization.
- `browser_cognition.rs`, `browser_media_governance.rs`, `ide_cognition.rs`, `window_observer.rs`, `atspi_engine.rs`, `ocr_engine.rs`, and `visual_reasoning.rs` handle app-specific or perception-adjacent GUI cognition.
- `crates/kria-uinput-daemon/src/main.rs` is the separate uinput daemon used for lower-level input automation.

## Safety, HITL, And Runtime Authority

- Policy: `crates/kria-core/src/safety/policy.rs`.
- HITL gateway: `crates/kria-core/src/safety/hitl.rs`.
- Audit logger: `crates/kria-core/src/safety/audit.rs`.
- Runtime authority and action gating:
  - `agent/execution_authority.rs`
  - `agent/execution_gate.rs`
  - `agent/collaborative_decision.rs`
  - `agent/resource_lease.rs`
  - `agent/resume_executor.rs`
  - `agent/continuation_reentry.rs`
- UI surfaces: `DecisionActionCenter.tsx` and `HitlModal.tsx`.
- Canonical docs: `docs/orchestration/runtime-authority.md` and `docs/contracts/hitl-mvp/`.

## Tool System

- `crates/kria-core/src/tools/registry.rs` defines `ToolRegistry`, schemas, handlers, execution context, and resume capability metadata.
- Tool modules live under `crates/kria-core/src/tools/`.
- Important domains include shell/exec, file ops, GUI automation, browser/Internet, desktop/system config, packages, power, developer, vision, image generation, Google Workspace, MCP, RAG/knowledge, N8N, scheduler, proactive/precognitive tools, and quarantine.
- OpenClaw has its own core module under `crates/kria-core/src/openclaw/` plus desktop commands and the `openclaw-substrate/` bridge.
- Tool calls should flow through registry, policy/HITL, audit, and preflight paths instead of bypassing runtime authority.

## Model And Provider Runtime

- `crates/kria-core/src/llm/model_router.rs` dispatches requests across local, cloud, and external model backends.
- `crates/kria-core/src/llm/orchestrator/` manages local llama-server lifecycle, watchdog, tier strategy, telemetry, thresholds, vision strategy, VRAM budgeting, and runtime state.
- `crates/kria-core/src/resource/gpu_lease.rs` coordinates GPU-heavy workloads.
- Provider/server routes live in `crates/kria-server/src/provider_routes.rs` and `crates/kria-server/src/intelligence_routes.rs`.

## Image And Vision

- `crates/kria-core/src/image/orchestrator.rs` is the image generation facade.
- `image/comfy.rs`, `cloud.rs`, `ws_bridge.rs`, `prompt_enhancer.rs`, `styles.rs`, `mode.rs`, and `capabilities.rs` implement local/cloud behavior.
- `tools/vision.rs`, `tools/vision_automation.rs`, and `preprocessing/image.rs` handle image analysis, OCR, attachments, and GUI vision-adjacent tools.

## Voice

- `crates/kria-core/src/voice/pipeline.rs` is the legacy voice pipeline.
- `crates/kria-core/src/voice/v2/` contains wake/AEC/STT/TTS/playback/post-edit streaming voice components.
- Desktop commands live in `crates/kria-desktop/src/commands/voice.rs` and `voice_diagnostics.rs`.
- UI surface: `VoiceOverlay.tsx`.

## Python And Sidecars

- `kria-modules/src/kria_modules/bridge.py` is the Python JSON-RPC sidecar dispatcher.
- Rust bridge: `crates/kria-core/src/sidecar/bridge.rs`.
- Protocol: `crates/kria-core/src/sidecar/protocol.rs`.
- Bootstrap/health: `bootstrap.rs`, `health.rs`.
- Python processors live in `kria-modules/src/kria_modules/processors/`.

## Server, Fleet, And Remote Control

- `crates/kria-server/src/main.rs` starts standalone server mode.
- `lib.rs`, `routes.rs`, `ws.rs`, `auth.rs`, `provider_routes.rs`, `intelligence_routes.rs`, and `inventory.rs` provide API/WebSocket/auth/provider/inventory surfaces.
- Desktop device/fleet control lives in `crates/kria-desktop/src/device_control.rs` and device command modules.
- Signed connection/lease primitives live in `crates/kria-connection-control/src/manager.rs` and `signer.rs`.
- Remote QEMU/VM/pool/QoS/snapshot/supervisor code lives under `crates/kria-core/src/infra/`.

## Evaluation

- `crates/kria-eval/src/main.rs` starts the eval harness.
- Base eval harness: `runner.rs`, `suite.rs`, `judge.rs`, `sandbox.rs`, `llm_fixture.rs`, and `report.rs`.
- GUI evals: `crates/kria-eval/src/gui_eval/`.
- Workflow evals: `crates/kria-eval/src/workflow_eval/`.
- Integration evals: `crates/kria-eval/src/integration_eval/`.
- Rust integration tests live under `crates/kria-core/tests/`, `crates/kria-server/tests/`, and `crates/kria-connection-control/tests/`.
- Browser/UI e2e tests live under `testing/suites/playwright/`.

## Test Utilities

- `crates/kria-core/src/bin/kria-test.rs` is the core test binary.
- `crates/kria-core/src/test_runner/` provides test runner utilities.
- `crates/kria-desktop/src/commands/tests.rs` covers command-layer behavior.
- `crates/kria-test-app/src/main.rs` is a minimal app harness.
