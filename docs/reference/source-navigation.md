# KRIA Source Navigation

## Purpose

This is the practical maintainer map for the KRIA source tree. Use it when you
know the subsystem or behavior you want to inspect and need the first file to
open.

This guide is not an architecture handbook. It points to implementation entry
points and current runtime paths. For subsystem reasoning, use the architecture
and orchestration docs after you land in the right code.

Related docs:
- `docs/index.md`
- `docs/architecture/overview.md`
- `docs/architecture/core-runtime.md`
- `docs/architecture/llm-orchestrator-runtime.md`
- `docs/architecture/gui-cognition-runtime.md`
- `docs/architecture/safety-hitl-runtime.md`
- `docs/orchestration/runtime-authority.md`
- `docs/orchestration/tool-system.md`
- `docs/orchestration/gui-execution.md`
- `docs/orchestration/opgraph-contract.md`

## Mental Model

KRIA is easiest to read as a bounded execution runtime:

```text
desktop UI / API / eval harness
  -> AgentLoop
  -> intent gates and turn planning
  -> model/provider routing
  -> tool-call parsing and fallback
  -> policy, HITL, audit, target authority, isolation
  -> tools, GUI, shell, browser, MCP, integrations
  -> verifier, synthesis, memory, transparency
```

The three fastest starting points are:

| Question | Start here |
|---|---|
| How does a prompt execute? | `crates/kria-core/src/agent/loop_engine/mod.rs` |
| How are model providers selected? | `crates/kria-core/src/llm/model_router.rs` and `crates/kria-core/src/llm/provider/registry.rs` |
| How does a tool become safe to run? | `crates/kria-core/src/safety/policy.rs`, `crates/kria-core/src/agent/execution_authority.rs`, `crates/kria-core/src/tools/preflight.rs` |

## Repository Topology

```text
KRIA/
  crates/
    kria-core/                 core runtime, agent loop, tools, safety, LLM, GUI
    kria-desktop/              Tauri desktop app and command bridge
    kria-server/               HTTP/WebSocket server surfaces
    kria-eval/                 eval suites and report generation
    kria-connection-control/   device orchestration and signed control plane
    kria-uinput-daemon/        Linux input helper daemon
    kria-test-app/             small runtime test app
  docs/
    architecture/              canonical architecture handbooks
    orchestration/             execution authority and runtime contracts
    contracts/hitl-mvp/        HITL MVP implementation contracts
    operations/                dev, deployment, providers, hardware
    evaluations/               eval/runbook docs
    integrations/              external integration docs
    llm-context/               AI-facing project context and graphs
    decisions/                 ADR/RFC records
    reference/                 navigation and source maps
    voice/                     voice runtime docs
  tests/
    e2e/                       Playwright/API/Tauri-style tests
  tests-logs/                  generated eval/test logs
  testing/eval_reports/                generated eval summaries
```

## Main Crate Entry Points

| Crate | Entry point | What it owns |
|---|---|---|
| `kria-core` | `crates/kria-core/src/lib.rs` | Core modules exported to desktop/server/evals |
| `kria-desktop` | `crates/kria-desktop/src/main.rs` | Tauri app startup and command registration |
| `kria-server` | `crates/kria-server/src/main.rs` | Server startup |
| `kria-eval` | `crates/kria-eval/src/main.rs` | Eval CLI runner |
| `kria-connection-control` | `crates/kria-connection-control/src/lib.rs` | Device orchestration library |
| `kria-uinput-daemon` | `crates/kria-uinput-daemon/src/main.rs` | Linux uinput daemon |
| `kria-test-app` | `crates/kria-test-app/src/main.rs` | Test app harness |

## Prompt-To-Execution Spine

```text
desktop chat command
  -> AgentLoop
  -> TurnAdmission / cancellation tree
  -> IntentGate
  -> TurnGate
  -> prompt/model routing
  -> response parser
  -> policy/HITL/preflight/target authority
  -> ToolRegistry handler
  -> run_isolated
  -> optional verifier
  -> result synthesis
  -> stream events / final answer
```

Open these files first:

| Runtime area | File |
|---|---|
| Desktop chat stream | `crates/kria-desktop/src/commands/chat.rs` |
| Runtime construction | `crates/kria-desktop/src/commands/runtime.rs` |
| Main agent loop | `crates/kria-core/src/agent/loop_engine/mod.rs` |
| Turn admission/cancellation | `crates/kria-core/src/agent/turn_context.rs` |
| Intent gate | `crates/kria-core/src/agent/intent_gate.rs` |
| Turn gate | `crates/kria-core/src/agent/turn_gate.rs` |
| Tool-call parsing | `crates/kria-core/src/agent/response_parser.rs` |
| Result synthesis | `crates/kria-core/src/agent/result_synthesizer.rs` |

## Tool Execution Spine

```text
ParsedToolCall
  -> allowed tool/schema checks
  -> PolicyEngine
  -> HITL when required
  -> preflight
  -> execution authority
  -> ToolRegistry handler lookup
  -> ToolContext
  -> run_isolated
  -> ToolResult
  -> verifier
  -> synthesizer
```

Open these files first:

| Area | File |
|---|---|
| Tool schemas and handlers | `crates/kria-core/src/tools/registry.rs` |
| Tool execution loop | `crates/kria-core/src/agent/loop_engine/mod.rs` |
| Deterministic preflight | `crates/kria-core/src/tools/preflight.rs` |
| Target validation | `crates/kria-core/src/agent/execution_authority.rs` |
| Policy risk tiers | `crates/kria-core/src/safety/policy.rs` |
| Shell command classifier | `crates/kria-core/src/safety/command_classifier.rs` |
| HITL approval gateway | `crates/kria-core/src/safety/hitl.rs` |
| Audit log | `crates/kria-core/src/safety/audit.rs` |
| Isolated execution | `crates/kria-core/src/infra/isolation.rs` |

## GUI Cognition And Desktop Automation

Current GUI cognition is a semantic contract layer resolved before substrate
planning. Concrete action steps are still owned by the GUI substrate planner and
executor.

```text
GuiTaskSpec
  -> SemanticWorkflowAnalysis
  -> ExecutionModeDecision
  -> WorkflowIntentContract check
  -> VerifierAuthorityAssessment
  -> SubstratePlanner
  -> GuiWorkflow / GoalTree / StageExecutor
  -> GUI/browser/app tools
```

Open these files first:

| Area | File |
|---|---|
| GUI runtime wiring | `crates/kria-core/src/agent/gui_wiring.rs` |
| Semantic workflow frame/fidelity | `crates/kria-core/src/agent/semantic_workflow.rs` |
| Execution mode reasoner | `crates/kria-core/src/agent/execution_mode_reasoner.rs` |
| Workflow contracts | `crates/kria-core/src/agent/workflow_intent_contract.rs` |
| Verifier authority metadata | `crates/kria-core/src/agent/verifier_authority.rs` |
| Hybrid sync checkpoints | `crates/kria-core/src/agent/hybrid_synchronization.rs` |
| GUI readiness checks | `crates/kria-core/src/agent/gui_production_readiness.rs` |
| GUI substrate planner | `crates/kria-core/src/agent/gui_substrate_planner.rs` |
| GUI planner traits | `crates/kria-core/src/agent/gui_planner.rs` |
| HTN executor | `crates/kria-core/src/agent/htn_executor.rs` |
| App lifecycle tools | `crates/kria-core/src/tools/app_lifecycle.rs` |
| GUI automation tools | `crates/kria-core/src/tools/gui_automation.rs` |
| AT-SPI tools | `crates/kria-core/src/tools/atspi_tools.rs` |
| Vision automation | `crates/kria-core/src/tools/vision_automation.rs` |
| Global GUI halt | `crates/kria-core/src/safety/global_halt.rs` |

Related state/cognition files:

| Area | Files |
|---|---|
| Browser cognition | `crates/kria-core/src/agent/browser_cognition.rs`, `crates/kria-core/src/tools/browser_agent.rs` |
| Browser/media governance | `crates/kria-core/src/agent/browser_media_governance.rs` |
| IDE cognition | `crates/kria-core/src/agent/ide_cognition.rs`, `crates/kria-core/src/tools/developer.rs` |
| Desktop awareness | `crates/kria-core/src/agent/desktop_awareness/mod.rs` |
| Observable completion | `crates/kria-core/src/agent/observable_completion/mod.rs` |
| Bounded execution verifier | `crates/kria-core/src/agent/execution_verifier_bounded.rs` |
| Window observation | `crates/kria-core/src/agent/window_observer.rs` |

## OpGraph, GoalTree, And Workflow Execution

```text
OpGraph
  -> validate graph
  -> GoalTreeOpGraphCompiler
  -> GoalTree validation
  -> StageExecutor
  -> tool actions and checkpoints
```

Open these files first:

| Area | File |
|---|---|
| OpGraph model | `crates/kria-core/src/agent/opgraph.rs` |
| OpGraph compiler | `crates/kria-core/src/agent/opgraph_compiler.rs` |
| GoalTree model | `crates/kria-core/src/agent/goal_tree.rs` |
| Workflow compiler | `crates/kria-core/src/agent/workflow_compiler.rs` |
| Stage executor | `crates/kria-core/src/agent/stage_executor.rs` |
| Workflow sessions | `crates/kria-core/src/agent/workflow_session.rs` |
| Workflow continuation | `crates/kria-core/src/agent/workflow_continuation/mod.rs` |
| Workflow expectations | `crates/kria-core/src/agent/workflow_expectation/mod.rs` |
| Resume executor | `crates/kria-core/src/agent/resume_executor.rs` |

## LLM And Provider Runtime

```text
AgentLoop
  -> ModelRouter / FailoverRouter
  -> LlmBackend
  -> provider adapter
  -> normalized response stream/tool calls
```

Open these files first:

| Area | File |
|---|---|
| LLM traits and shared types | `crates/kria-core/src/llm/mod.rs` |
| Model routing | `crates/kria-core/src/llm/model_router.rs` |
| Failover state machine | `crates/kria-core/src/llm/failover.rs` |
| Token budgets | `crates/kria-core/src/llm/budget.rs` |
| Token counting | `crates/kria-core/src/llm/tokenize.rs` |
| Local backend | `crates/kria-core/src/llm/local.rs` |
| Generic cloud backend | `crates/kria-core/src/llm/cloud.rs` |
| Provider registry | `crates/kria-core/src/llm/provider/registry.rs` |
| Provider config | `crates/kria-core/src/llm/provider/config.rs` |
| Provider capabilities | `crates/kria-core/src/llm/provider/capabilities.rs` |
| Provider errors | `crates/kria-core/src/llm/provider/error.rs` |
| Provider streaming | `crates/kria-core/src/llm/provider/streaming.rs` |
| OpenAI adapter | `crates/kria-core/src/llm/provider/openai.rs` |
| OpenRouter adapter | `crates/kria-core/src/llm/provider/openrouter.rs` |
| Anthropic adapter | `crates/kria-core/src/llm/provider/anthropic.rs` |
| Gemini adapter | `crates/kria-core/src/llm/provider/gemini.rs` |
| Ollama adapter | `crates/kria-core/src/llm/provider/ollama.rs` |

## Local Model Orchestrator

Open these files first:

| Area | File |
|---|---|
| llama-server lifecycle | `crates/kria-core/src/llm/orchestrator/server_manager.rs` |
| Runtime control contract | `crates/kria-core/src/llm/orchestrator/runtime.rs` |
| Strategy planning | `crates/kria-core/src/llm/orchestrator/strategy.rs` |
| Tier strategy | `crates/kria-core/src/llm/orchestrator/tier_strategy.rs` |
| VRAM budget | `crates/kria-core/src/llm/orchestrator/vram_budget.rs` |
| Vision strategy | `crates/kria-core/src/llm/orchestrator/vision_strategy.rs` |
| GPU watchdog | `crates/kria-core/src/llm/orchestrator/gpu_watchdog.rs` |
| Child cleanup | `crates/kria-core/src/llm/orchestrator/child_guard.rs` |
| Telemetry | `crates/kria-core/src/llm/orchestrator/telemetry.rs` |
| Thresholds | `crates/kria-core/src/llm/orchestrator/threshold.rs` |

## Tools And Integrations

| Tool area | File |
|---|---|
| Registry and schema | `crates/kria-core/src/tools/registry.rs` |
| File operations | `crates/kria-core/src/tools/file_ops.rs` |
| Shell/process execution | `crates/kria-core/src/tools/exec.rs`, `crates/kria-core/src/tools/shell.rs` |
| Subprocess helper | `crates/kria-core/src/tools/subprocess_executor.rs` |
| Internet/search | `crates/kria-core/src/tools/internet.rs`, `crates/kria-core/src/tools/news.rs` |
| Browser agent | `crates/kria-core/src/tools/browser_agent.rs` |
| App lifecycle | `crates/kria-core/src/tools/app_lifecycle.rs` |
| Desktop tools | `crates/kria-core/src/tools/desktop.rs` |
| Developer tools | `crates/kria-core/src/tools/developer.rs` |
| Documents | `crates/kria-core/src/tools/documents.rs` |
| Google Workspace | `crates/kria-core/src/tools/google_workspace.rs`, `crates/kria-core/src/tools/google_workspace_contract.rs` |
| n8n | `crates/kria-core/src/tools/n8n.rs` |
| Image generation | `crates/kria-core/src/tools/image_generation.rs` |
| Memory/knowledge | `crates/kria-core/src/tools/knowledge.rs` |
| RAG | `crates/kria-core/src/tools/rag.rs` |
| Proactive tools | `crates/kria-core/src/tools/proactive.rs` |
| Package management | `crates/kria-core/src/tools/packages.rs` |
| Power/system/process/disk | `crates/kria-core/src/tools/power.rs`, `crates/kria-core/src/tools/system_config.rs`, `crates/kria-core/src/tools/process.rs`, `crates/kria-core/src/tools/disk.rs` |
| Tool mounting | `crates/kria-core/src/tools/mount_manager.rs` |
| Quarantine | `crates/kria-core/src/tools/quarantine.rs` |

## MCP

| Area | File |
|---|---|
| MCP client | `crates/kria-core/src/mcp/client.rs` |
| MCP server lifecycle | `crates/kria-core/src/mcp/server_manager.rs` |
| MCP tool bridge | `crates/kria-core/src/mcp/tool_bridge.rs` |
| Payload shaping | `crates/kria-core/src/mcp/payload_shaper.rs` |
| Capability registry | `crates/kria-core/src/mcp/capability_registry.rs` |

## Memory, Context, And World Model

| Area | File |
|---|---|
| Memory subsystem | `crates/kria-core/src/memory/` |
| PSDG coordinator | `crates/kria-core/src/agent/psdg/coordinator.rs` |
| Context injection | `crates/kria-core/src/agent/psdg/context_injector.rs` |
| Environment tracking | `crates/kria-core/src/agent/psdg/env_tracker.rs` |
| World model store | `crates/kria-core/src/agent/world_model/store.rs` |
| World model types | `crates/kria-core/src/agent/world_model/types.rs` |
| Desktop graph | `crates/kria-core/src/agent/world_model/desktop_graph.rs` |
| Working set extraction | `crates/kria-core/src/agent/working_set/extractor.rs` |
| Workspace memory | `crates/kria-core/src/agent/workspace_memory.rs` |
| Procedural memory | `crates/kria-core/src/agent/procedural_memory/mod.rs` |

Rules:
- inject compact facts, not raw memory dumps,
- current tool evidence outranks old memory,
- keep desktop context operation-scoped,
- prefer structured facts over free-form text.

## Safety And HITL

| Area | File |
|---|---|
| Policy engine | `crates/kria-core/src/safety/policy.rs` |
| Command classifier | `crates/kria-core/src/safety/command_classifier.rs` |
| Policy gate | `crates/kria-core/src/safety/policy_gate.rs` |
| HITL gateway | `crates/kria-core/src/safety/hitl.rs` |
| Audit logger | `crates/kria-core/src/safety/audit.rs` |
| Rollback snapshots | `crates/kria-core/src/safety/rollback.rs` |
| Global GUI halt | `crates/kria-core/src/safety/global_halt.rs` |
| Blacklist | `crates/kria-core/src/safety/blacklist.rs` |
| PIN guard | `crates/kria-core/src/safety/pin_guard.rs` |
| Action execution gate | `crates/kria-core/src/agent/execution_gate.rs` |
| Collaborative decision | `crates/kria-core/src/agent/collaborative_decision.rs` |

## Infrastructure

| Area | File |
|---|---|
| Tool isolation | `crates/kria-core/src/infra/isolation.rs` |
| Pipeline trace | `crates/kria-core/src/infra/pipeline_trace.rs` |
| Execution trace | `crates/kria-core/src/infra/execution_trace.rs` |
| Event bus | `crates/kria-core/src/infra/event_bus.rs` |
| Health registry | `crates/kria-core/src/infra/health.rs` |
| QoS | `crates/kria-core/src/infra/qos/mod.rs` |
| Environment abstraction | `crates/kria-core/src/infra/environment/` |
| Supervisor | `crates/kria-core/src/infra/supervisor.rs` |

## Desktop Command Bridge

| Command area | File |
|---|---|
| Command module root | `crates/kria-desktop/src/commands/mod.rs` |
| Shared app state | `crates/kria-desktop/src/commands/app_state.rs` |
| Chat | `crates/kria-desktop/src/commands/chat.rs` |
| Runtime setup/status | `crates/kria-desktop/src/commands/runtime.rs`, `crates/kria-desktop/src/commands/runtime_status.rs` |
| Providers | `crates/kria-desktop/src/commands/providers.rs` |
| MCP | `crates/kria-desktop/src/commands/mcp.rs` |
| GUI automation controls | `crates/kria-desktop/src/commands/gui_automation_control.rs` |
| Image chat | `crates/kria-desktop/src/commands/image_chat.rs` |
| Document chat | `crates/kria-desktop/src/commands/document_chat.rs` |
| Voice | `crates/kria-desktop/src/commands/voice.rs`, `crates/kria-desktop/src/commands/voice_diagnostics.rs` |
| Google Workspace | `crates/kria-desktop/src/commands/google_workspace.rs` |
| n8n | `crates/kria-desktop/src/commands/n8n.rs` |
| OpenClaw | `crates/kria-desktop/src/commands/openclaw.rs` |
| Colab | `crates/kria-desktop/src/commands/colab.rs`, `crates/kria-desktop/src/commands/colab_dispatch.rs` |
| Device enrollment/tools | `crates/kria-desktop/src/commands/device_enrollment.rs`, `crates/kria-desktop/src/commands/device_tools.rs` |
| Test runner | `crates/kria-desktop/src/commands/test_runner.rs` |

## Server And Connection Control

| Area | File |
|---|---|
| Server startup | `crates/kria-server/src/main.rs` |
| Server library | `crates/kria-server/src/lib.rs` |
| HTTP routes | `crates/kria-server/src/routes.rs` |
| Provider routes | `crates/kria-server/src/provider_routes.rs` |
| Intelligence routes | `crates/kria-server/src/intelligence_routes.rs` |
| WebSocket | `crates/kria-server/src/ws.rs` |
| Auth | `crates/kria-server/src/auth.rs` |
| Inventory | `crates/kria-server/src/inventory.rs` |
| Device manager | `crates/kria-connection-control/src/manager.rs` |
| Device signing | `crates/kria-connection-control/src/signer.rs` |
| Device DB schema | `crates/kria-connection-control/sql/0001_device_orchestration.sql` |

## Eval And Test Navigation

| Need | Where to look |
|---|---|
| Playwright/API e2e tests | `testing/suites/playwright/` |
| General test notes | `testing/docs/legacy-testing.md` |
| Core test binary | `crates/kria-core/src/bin/kria-test.rs` |
| GUI e2e test binary | `crates/kria-core/src/bin/test_gui_e2e.rs` |
| Eval crate entry | `crates/kria-eval/src/main.rs` |
| General eval runner | `crates/kria-eval/src/runner.rs` |
| GUI eval runner | `crates/kria-eval/src/gui_eval/runner.rs` |
| GUI eval suites | `crates/kria-eval/src/gui_eval/` |
| Workflow eval suites | `crates/kria-eval/src/workflow_eval/` |
| Integration eval suites | `crates/kria-eval/src/integration_eval/` |
| Generated reports | `tests-logs/`, `testing/eval_reports/` |

Useful docs:
- `docs/evaluations/overview.md`
- `docs/evaluations/gui-e2e.md`
- `docs/evaluations/voice-validation.md`
- `docs/decisions/adr/001-e2e-eval-harness.md`
- `docs/decisions/adr/002-tool-execution-overhaul.md`

## Change Maps

### Add Or Modify A Tool

Start here:
1. `crates/kria-core/src/tools/registry.rs`
2. relevant file under `crates/kria-core/src/tools/`
3. `crates/kria-core/src/safety/policy.rs`
4. `crates/kria-core/src/tools/preflight.rs`
5. `crates/kria-core/src/agent/execution_authority.rs`
6. `crates/kria-core/src/agent/loop_engine/tests.rs`

Checklist:
- register schema and handler,
- set or infer resume capability,
- assign risk behavior,
- add preflight if parameters can be unsafe,
- add target authority if the tool is target-specific,
- add verifier or result-synthesis handling if success can be misleading,
- add routing/safety tests.

### Add Or Modify A Provider

Start here:
1. `crates/kria-core/src/llm/provider/config.rs`
2. `crates/kria-core/src/llm/provider/registry.rs`
3. provider adapter under `crates/kria-core/src/llm/provider/`
4. `crates/kria-core/src/llm/provider/capabilities.rs`
5. `crates/kria-core/src/llm/provider/streaming.rs`
6. `crates/kria-core/src/llm/provider/tests.rs`

Checklist:
- add provider config/type,
- implement backend adapter,
- normalize capabilities,
- normalize streaming and tool-call output,
- normalize errors,
- add connection tests.

### Change Prompt Construction

Start here:
1. `crates/kria-core/src/agent/prompt_compiler.rs`
2. `crates/kria-core/src/agent/prompts.rs`
3. `crates/kria-core/src/agent/psdg/context_injector.rs`
4. `crates/kria-core/src/llm/budget.rs`

Rules:
- keep context bounded,
- do not put runtime authority only in prompt text,
- preserve deterministic section order,
- verify tool-selection impact.

### Change Safety Or HITL

Start here:
1. `crates/kria-core/src/safety/policy.rs`
2. `crates/kria-core/src/safety/command_classifier.rs`
3. `crates/kria-core/src/safety/hitl.rs`
4. `crates/kria-core/src/safety/audit.rs`
5. `crates/kria-core/src/agent/loop_engine/mod.rs`
6. `docs/contracts/hitl-mvp/`

Never skip:
- blacklist,
- command classification,
- protected-path escalation,
- audit logging,
- timeout auto-deny,
- denial result injection back into the tool loop.

### Change GUI Automation

Start here:
1. `crates/kria-core/src/agent/gui_wiring.rs`
2. `crates/kria-core/src/agent/semantic_workflow.rs`
3. `crates/kria-core/src/agent/execution_mode_reasoner.rs`
4. `crates/kria-core/src/agent/workflow_intent_contract.rs`
5. `crates/kria-core/src/agent/gui_substrate_planner.rs`
6. `crates/kria-core/src/agent/htn_executor.rs`
7. `crates/kria-core/src/tools/gui_automation.rs`

Check:
- semantic workflow frame,
- execution mode,
- contract check,
- substrate selected,
- app/window/focus state,
- global halt,
- readiness preflight,
- verifier authority and partial-completion result.

### Change Browser Or IDE Cognition

Start here:
- browser: `crates/kria-core/src/agent/browser_cognition.rs`
- browser governance: `crates/kria-core/src/agent/browser_media_governance.rs`
- browser tools: `crates/kria-core/src/tools/browser_agent.rs`
- IDE cognition: `crates/kria-core/src/agent/ide_cognition.rs`
- developer tools: `crates/kria-core/src/tools/developer.rs`
- world model: `crates/kria-core/src/agent/world_model/`

### Change Evals

Start here:
1. `crates/kria-eval/src/main.rs`
2. `crates/kria-eval/src/gui_eval/runner.rs`
3. `crates/kria-eval/src/gui_eval/suites.rs`
4. specific suite file under `crates/kria-eval/src/gui_eval/`
5. `crates/kria-eval/src/report.rs`

For live GUI evals, verify environment gates and artifacts:
- `KRIA_EVAL_GUI=1`,
- display server,
- AT-SPI,
- uinput daemon,
- OCR/vision support when required,
- temp-only artifacts,
- screenshot/log capture on failure.

## Debugging Paths

### Model Unavailable

Open:
1. `crates/kria-core/src/llm/model_router.rs`
2. `crates/kria-core/src/llm/failover.rs`
3. `crates/kria-core/src/llm/provider/registry.rs`
4. `crates/kria-core/src/llm/local.rs`
5. `crates/kria-core/src/llm/cloud.rs`
6. `crates/kria-desktop/src/commands/providers.rs`

Check provider config, local server health, auth, context length, vision
requirement, and fallback provider status.

### Tool Did Not Run

Open:
1. `crates/kria-core/src/agent/loop_engine/mod.rs`
2. `crates/kria-core/src/tools/registry.rs`
3. `crates/kria-core/src/safety/policy.rs`
4. `crates/kria-core/src/tools/preflight.rs`
5. `crates/kria-core/src/agent/execution_authority.rs`
6. `crates/kria-core/src/safety/hitl.rs`
7. `crates/kria-core/src/infra/isolation.rs`

Check schema exposure, policy, HITL result, preflight, target authority,
handler lookup, timeout, and tool result.

### GUI Workflow Started But Failed

Open:
1. `crates/kria-core/src/agent/gui_wiring.rs`
2. `crates/kria-core/src/agent/gui_production_readiness.rs`
3. `crates/kria-core/src/agent/gui_substrate_planner.rs`
4. `crates/kria-core/src/agent/htn_executor.rs`
5. `crates/kria-core/src/tools/gui_automation.rs`
6. `crates/kria-core/src/agent/observable_completion/mod.rs`
7. `crates/kria-core/src/agent/workflow_continuation/mod.rs`

Check global halt, display readiness, AT-SPI/uinput/OCR availability, selected
substrate, window identity, focus, visible evidence, and partial-completion
status.

### Browser Navigation Failed

Open:
1. `crates/kria-core/src/tools/app_lifecycle.rs`
2. `crates/kria-core/src/tools/browser_agent.rs`
3. `crates/kria-core/src/agent/browser_cognition.rs`
4. `crates/kria-core/src/agent/execution_authority.rs`
5. `crates/kria-core/src/agent/verifier_authority.rs`

Check target authority, URL normalization, managed browser/CDP fallback,
visible page evidence, and freshness.

### Final Answer Claimed Success Incorrectly

Open:
1. `crates/kria-core/src/agent/result_synthesizer.rs`
2. `crates/kria-core/src/agent/execution_interpreter.rs`
3. `crates/kria-core/src/agent/loop_engine/response_helpers.rs`
4. `crates/kria-core/src/agent/execution_verifier.rs`
5. `crates/kria-core/src/agent/verifier_authority.rs`
6. `crates/kria-core/src/mcp/payload_shaper.rs`

Check raw tool result, shaped payload, verifier result, authority level,
partial-completion metadata, and synthesized summary.

### Dangerous Command Was Allowed

Open immediately:
1. `crates/kria-core/src/safety/blacklist.rs`
2. `crates/kria-core/src/safety/command_classifier.rs`
3. `crates/kria-core/src/safety/policy.rs`
4. `crates/kria-core/src/tools/preflight.rs`
5. `crates/kria-core/src/agent/execution_authority.rs`
6. `crates/kria-core/src/safety/audit.rs`
7. `crates/kria-core/src/agent/loop_engine/mod.rs`

Check blacklist match, command tier, protected-path escalation, destructive
modality hint, eval mode, audit record, and whether the path bypassed policy.

## Concept Index

| Concept | First file | Supporting files |
|---|---|---|
| Prompt lifecycle | `crates/kria-core/src/agent/loop_engine/mod.rs` | `crates/kria-core/src/agent/prompt_compiler.rs`, `crates/kria-core/src/llm/model_router.rs` |
| Stream events | `crates/kria-core/src/agent/loop_engine/mod.rs` | `crates/kria-desktop/src/commands/chat.rs` |
| Turn admission | `crates/kria-core/src/agent/turn_context.rs` | `crates/kria-core/src/agent/loop_engine/mod.rs` |
| Intent routing | `crates/kria-core/src/agent/turn_gate.rs` | `crates/kria-core/src/agent/intent_gate.rs`, `crates/kria-core/src/agent/router.rs` |
| Tool calls | `crates/kria-core/src/agent/response_parser.rs` | `crates/kria-core/src/tools/registry.rs` |
| Tool safety | `crates/kria-core/src/safety/policy.rs` | `crates/kria-core/src/tools/preflight.rs`, `crates/kria-core/src/agent/execution_authority.rs` |
| HITL | `crates/kria-core/src/safety/hitl.rs` | `docs/contracts/hitl-mvp/` |
| Provider switching | `crates/kria-core/src/llm/provider/registry.rs` | `crates/kria-core/src/llm/model_router.rs` |
| Local model process | `crates/kria-core/src/llm/orchestrator/server_manager.rs` | `crates/kria-core/src/llm/local.rs` |
| Context budget | `crates/kria-core/src/llm/budget.rs` | `crates/kria-core/src/mcp/payload_shaper.rs` |
| GUI cognition | `crates/kria-core/src/agent/gui_wiring.rs` | `crates/kria-core/src/agent/semantic_workflow.rs`, `crates/kria-core/src/agent/execution_mode_reasoner.rs` |
| OpGraph | `crates/kria-core/src/agent/opgraph.rs` | `crates/kria-core/src/agent/opgraph_compiler.rs`, `crates/kria-core/src/agent/goal_tree.rs` |
| Workflow continuation | `crates/kria-core/src/agent/workflow_continuation/mod.rs` | `crates/kria-core/src/agent/resume_executor.rs` |
| MCP tools | `crates/kria-core/src/mcp/tool_bridge.rs` | `crates/kria-core/src/mcp/client.rs`, `crates/kria-core/src/mcp/server_manager.rs` |
| Google Workspace | `crates/kria-core/src/tools/google_workspace.rs` | `crates/kria-core/src/tools/google_workspace_contract.rs` |
| n8n | `crates/kria-core/src/tools/n8n.rs` | `docs/integrations/n8n.md` |
| OpenClaw | `crates/kria-desktop/src/commands/openclaw.rs` | `docs/integrations/openclaw.md` |
| Voice runtime | `crates/kria-core/src/voice/` | `crates/kria-desktop/src/commands/voice.rs` |
| Image generation | `crates/kria-core/src/image/` | `crates/kria-core/src/tools/image_generation.rs` |
| Server API | `crates/kria-server/src/routes.rs` | `crates/kria-server/src/ws.rs` |
| Device control | `crates/kria-connection-control/src/manager.rs` | `crates/kria-desktop/src/commands/device_tools.rs` |

## Minimal Mental Model

```text
AgentLoop owns the turn.
TurnGate owns top-level operation planning.
ModelRouter owns model/provider selection.
ToolRegistry owns schemas, handlers, environment, and shell state.
PolicyEngine owns risk.
HitlGateway owns approval.
ExecutionAuthority owns target validity.
run_isolated owns bounded execution.
Verifier owns evidence truth.
ResultSynthesizer owns user-facing result wording.
```
