# KRIA Source Navigation Guide

Human-readable map for finding your way around the KRIA codebase.

This guide answers the practical maintainer question:

> "I know the concept. Where is the code, what file should I open first, and what
> path does runtime execution actually follow?"

## Reader Contract

This guide is intentionally practical:

- **It is a map, not a replacement for architecture docs.** Use it to find the right files,
  then use the deep handbooks for subsystem reasoning.
- **Current implementation only:** file paths and entry points refer to the current KRIA
  working tree.
- **Mixed audience:** plain-language descriptions explain why a file matters; source-level
  names help engineers jump directly into code.
- **No roadmap content:** future improvements are intentionally left to the subsystem
  architecture handbooks.
- **Source truth:** if a path moves, update this guide immediately.

It is a companion to:

- `docs/architecture/core-runtime.md`
- `docs/architecture/llm-orchestrator-runtime.md`
- `docs/architecture/gui-cognition-runtime.md`

---

## 1. How To Read The Codebase

KRIA is easiest to understand as layered runtime code, not as a list of features.

```text
Desktop UI / Tauri commands
        |
        v
AgentLoop orchestration
        |
        v
Intent, routing, prompt, model, and tool planning
        |
        v
Policy, HITL, audit, isolation
        |
        v
Tools, MCP, GUI, browser, filesystem, shell, provider calls
        |
        v
Verification, synthesis, memory, transparency
```

The three most important starting points are:

| Question | Start Here |
| -------- | ---------- |
| "How does a chat prompt execute?" | `crates/kria-core/src/agent/loop_engine/mod.rs` |
| "How does KRIA call LLMs/providers?" | `crates/kria-core/src/llm/mod.rs` and `crates/kria-core/src/llm/model_router.rs` |
| "How does a tool become safe to run?" | `crates/kria-core/src/safety/policy.rs` and `crates/kria-core/src/agent/loop_engine/mod.rs` |

---

## 2. Repository Topology

```text
KRIA/
  crates/
    kria-core/
      src/
        agent/          cognition, orchestration, GUI planning, verification
        llm/            model/provider orchestration
        tools/          local tool handlers and registry
        safety/         policy, HITL, audit, rollback, halt
        mcp/            MCP client/server/tool bridge
        routing/        intent and tool routing
        memory/         user memory, facts, RAG pieces
        infra/          isolation, health, tracing, environment, QoS
        platform/       OS intent dispatch and platform-specific capability layer
        voice/          voice runtime
        image/          image generation/runtime
        orchestrator/   service orchestration
    kria-desktop/
      src/
        main.rs         Tauri desktop entry
        commands/       UI command bridge into core runtime
        device_control.rs fleet/remote device UI runtime
  docs/
    architecture/       architecture handbooks
    orchestration/      execution contracts
    evaluations/        eval and runbooks
    contracts/          implementation-binding contracts
    operations/         deployment, development, providers, hardware
    decisions/          ADR and RFC records
  tests/
    e2e/                Playwright/Tauri-style e2e tests
  tests-logs/           generated test reports and JSON outcomes when evals run
```

---

## 3. Fast Runtime Map

### Prompt-To-Execution Spine

```text
UI command
  |
  v
crates/kria-desktop/src/commands/chat.rs
  |
  v
AgentLoop
  crates/kria-core/src/agent/loop_engine/mod.rs
  |
  +-- TurnAdmission / TurnGate
  +-- Prompt compiler
  +-- ModelRouter / FailoverRouter
  +-- Tool parser and fallback
  +-- PolicyEngine / HITL / AuditLogger
  +-- run_isolated tool handler
  +-- verifier / synthesizer / final stream events
```

### LLM Runtime Spine

```text
AgentLoop
  |
  v
ModelRouter or FailoverRouter
  |
  v
LlmBackend trait
  |
  +-- LocalBackend
  +-- CloudBackend
  +-- OpenAIBackend
  +-- OpenRouterBackend
  +-- AnthropicBackend
  +-- GeminiBackend
  +-- OllamaBackend
```

### Tool Runtime Spine

```text
Tool schema visible to model
  |
  v
ParsedToolCall
  |
  v
allowed_tool_names check
  |
  v
PolicyEngine
  |
  v
HITL if needed
  |
  v
ToolRegistry handler
  |
  v
run_isolated
  |
  v
ToolResult + shaped payload + synthesis
```

---

## 4. Entry Points You Should Know

| Runtime Area | File | What To Look For |
| ------------ | ---- | ---------------- |
| Desktop app entry | `crates/kria-desktop/src/main.rs` | Tauri startup |
| Tauri command wiring | `crates/kria-desktop/src/commands/mod.rs` | command modules and shared imports |
| Chat command | `crates/kria-desktop/src/commands/chat.rs` | UI chat stream bridge |
| Runtime construction | `crates/kria-desktop/src/commands/runtime.rs` | `PolicyEngine`, `HitlGateway`, `AgentLoop` setup |
| Core library exports | `crates/kria-core/src/lib.rs` | module boundaries exported by core |
| Main agent loop | `crates/kria-core/src/agent/loop_engine/mod.rs` | central prompt/tool orchestration |
| Model routing | `crates/kria-core/src/llm/model_router.rs` | local/cloud/vision backend selection |
| Provider registry | `crates/kria-core/src/llm/provider/registry.rs` | configured providers and model switching |
| Tool registry | `crates/kria-core/src/tools/registry.rs` | registering and executing tools |
| Policy gate | `crates/kria-core/src/safety/policy.rs` | risk tiers and approvals |

---

## 5. If You Want To Change A Feature

### Add Or Modify A Tool

Start here:

1. `crates/kria-core/src/tools/registry.rs`
2. The relevant tool module in `crates/kria-core/src/tools/`
3. `crates/kria-core/src/safety/policy.rs`
4. `crates/kria-core/src/mcp/capability_registry.rs`
5. `crates/kria-core/src/agent/loop_engine/tests.rs`

Flow:

```text
Define handler and schema
  |
  v
Register in ToolRegistry
  |
  v
Assign safety tier
  |
  v
Assign capability profile / execution mode
  |
  v
Add routing/fallback tests
```

Important questions:

- Is it read-only, user-state modifying, destructive, or system-dangerous?
- Should it be preferred over GUI/browser automation?
- Does it need preflight validation?
- Should raw output be shaped before the LLM sees it?
- Does final success need a verifier?

### Add Or Modify An LLM Provider

Start here:

1. `crates/kria-core/src/llm/mod.rs`
2. `crates/kria-core/src/llm/provider/config.rs`
3. `crates/kria-core/src/llm/provider/registry.rs`
4. Existing adapter such as `llm/provider/openai.rs`
5. `crates/kria-core/src/llm/provider/tests.rs`

Flow:

```text
ProviderConfig / ProviderType
  |
  v
Backend adapter implements LlmBackend
  |
  v
ProviderRegistry creates backend
  |
  v
Capabilities normalized
  |
  v
Connection test + stream/tool parsing tests
```

Watch for:

- message format differences,
- system prompt support,
- function/tool calling format,
- streaming format,
- image input format,
- usage token fields,
- error code normalization.

### Change Prompt Construction

Start here:

1. `crates/kria-core/src/agent/prompt_compiler.rs`
2. `crates/kria-core/src/agent/prompts.rs`
3. `crates/kria-core/src/agent/psdg/context_injector.rs`
4. `crates/kria-core/src/llm/budget.rs`

Flow:

```text
Prompt section
  |
  v
Priority and budget behavior
  |
  v
Context injection policy
  |
  v
Tool catalog effect
  |
  v
LLM behavior and tests
```

Be careful:

- Do not add unbounded context.
- Do not put unsafe authority into prompt text.
- Do not rely on prompt rules where runtime policy is needed.
- Preserve deterministic assembly order where possible.

### Change Safety Or HITL

Start here:

1. `crates/kria-core/src/safety/policy.rs`
2. `crates/kria-core/src/safety/command_classifier.rs`
3. `crates/kria-core/src/safety/hitl.rs`
4. `crates/kria-core/src/safety/audit.rs`
5. `crates/kria-core/src/agent/loop_engine/mod.rs`
6. `crates/kria-core/src/agent/gui_wiring.rs`

Flow:

```text
Tool call
  |
  v
PolicyEngine.evaluate_with_modality_hint()
  |
  +-- blocked
  +-- approval required
  +-- allowed
  |
  v
AuditLogger
  |
  v
HITL gateway if needed
```

Never skip:

- blacklist,
- protected path escalation,
- audit logging,
- timeout auto-deny,
- denial result injection back into the tool loop.

### Change GUI Automation

Start here:

1. `crates/kria-core/src/agent/gui_wiring.rs`
2. `crates/kria-core/src/agent/gui_planner.rs`
3. `crates/kria-core/src/agent/gui_substrate_planner.rs`
4. `crates/kria-core/src/agent/htn_executor.rs`
5. `crates/kria-core/src/tools/gui_automation.rs`
6. `crates/kria-core/src/safety/global_halt.rs`

Flow:

```text
Intent
  |
  v
GUI planner / substrate planner
  |
  v
HTN executor or GUI tool
  |
  v
global halt check
  |
  v
focus/input/window actions
  |
  v
verification / recovery
```

Debug hints:

- If the tool fails with `GLOBAL_SAFETY_HALT`, inspect sidecar/uinput/vision service health.
- If the app opens but does not act, inspect focus and window verification.
- If Wayland is involved, expect input/focus limitations.

### Change Browser Or IDE Cognition

Start here:

| Area | Files |
| ---- | ----- |
| Browser cognition | `crates/kria-core/src/agent/browser_cognition.rs`, `crates/kria-core/src/tools/browser_agent.rs` |
| IDE cognition | `crates/kria-core/src/agent/ide_cognition.rs`, `crates/kria-core/src/tools/developer.rs` |
| Desktop context | `crates/kria-core/src/agent/psdg/context_injector.rs`, `crates/kria-core/src/agent/desktop_awareness/mod.rs` |
| Verification | `crates/kria-core/src/agent/execution_verifier_bounded.rs`, `crates/kria-core/src/agent/observable_completion/mod.rs` |

### Change Memory / PSDG

Start here:

1. `crates/kria-core/src/agent/psdg/mod.rs`
2. `crates/kria-core/src/agent/psdg/coordinator.rs`
3. `crates/kria-core/src/agent/psdg/env_tracker.rs`
4. `crates/kria-core/src/agent/world_model/*`
5. `crates/kria-core/src/memory/*`

Rules of thumb:

- Inject compact facts, not raw memory.
- Keep desktop context operation-specific.
- Do not let old memory outrank current tool evidence.
- Prefer structured facts over free-form text blobs.

---

## 6. Main Subsystems And Files

### Agent Orchestration

| File | Purpose |
| ---- | ------- |
| `agent/loop_engine/mod.rs` | Main ReAct/runtime loop, stream events, tool rounds, policy calls |
| `agent/turn_context.rs` | Turn admission, queueing, cancellation, supersession |
| `agent/turn_gate.rs` | Operation classification and fallback tool hints |
| `agent/intent_gate.rs` | Intent gate confidence and execution permission logic |
| `agent/router.rs` | Intent routing helpers and tests |
| `agent/response_parser.rs` | Extracts text and tool calls from model output |
| `agent/result_synthesizer.rs` | Grounded human-readable result generation |
| `agent/execution_interpreter.rs` | Interprets tool result outcomes |
| `agent/execution_transparency/mod.rs` | Workflow trace, blockers, explanations |

### Planning And Workflow

| File | Purpose |
| ---- | ------- |
| `agent/goal_tree.rs` | Goal tree, stages, checkpoints, recovery paths |
| `agent/workflow_compiler.rs` | Workflow compilation |
| `agent/opgraph.rs` | Operational graph model |
| `agent/opgraph_compiler.rs` | Compiles operational graphs |
| `agent/stage_executor.rs` | Stage execution support |
| `agent/workflow_session.rs` | Persistent workflow checkpoints |
| `agent/workflow_continuation/mod.rs` | Interruption classification and continuation |
| `agent/workflow_expectation/mod.rs` | Expected visible outcomes for workflow categories |

### GUI / Desktop Cognition

| File | Purpose |
| ---- | ------- |
| `agent/gui_wiring.rs` | Connects GUI planner/executor to policy, HITL, audit |
| `agent/gui_planner.rs` | GUI planning traits and implementations |
| `agent/gui_substrate_planner.rs` | Selects GUI substrate path |
| `agent/htn_executor.rs` | Hierarchical task execution for GUI workflows |
| `agent/htn_integration.rs` | HTN workflow generation/integration |
| `agent/atspi_engine.rs` | AT-SPI accessibility substrate |
| `agent/ocr_engine.rs` | OCR screen/region text reading |
| `agent/browser_cognition.rs` | Browser semantic state |
| `agent/ide_cognition.rs` | IDE/workspace state |
| `tools/gui_automation.rs` | GUI automation tool handlers |
| `tools/atspi_tools.rs` | Accessibility tools |
| `tools/vision_automation.rs` | Vision automation tools |

### LLM And Provider Runtime

| File | Purpose |
| ---- | ------- |
| `llm/mod.rs` | Core LLM types and `LlmBackend` trait |
| `llm/model_router.rs` | Local/cloud/vision routing |
| `llm/failover.rs` | Provider failover FSM |
| `llm/budget.rs` | Token budgets and turn ledger |
| `llm/tokenize.rs` | Token counting |
| `llm/local.rs` | Local llama.cpp backend |
| `llm/cloud.rs` | Generic OpenAI-compatible cloud backend |
| `llm/provider/registry.rs` | Provider lifecycle and switching |
| `llm/provider/config.rs` | Provider config model |
| `llm/provider/capabilities.rs` | Normalized model capabilities |
| `llm/provider/error.rs` | Provider error classification |
| `llm/provider/openai.rs` | OpenAI-compatible adapter |
| `llm/provider/openrouter.rs` | OpenRouter adapter |
| `llm/provider/anthropic.rs` | Anthropic adapter |
| `llm/provider/gemini.rs` | Gemini adapter |
| `llm/provider/ollama.rs` | Ollama adapter |

### Local Model Orchestrator

| File | Purpose |
| ---- | ------- |
| `llm/orchestrator/server_manager.rs` | llama-server lifecycle and state |
| `llm/orchestrator/runtime.rs` | L1 runtime control contract |
| `llm/orchestrator/strategy.rs` | VRAM/layer/context planning |
| `llm/orchestrator/vram_budget.rs` | Vision token/VRAM preflight |
| `llm/orchestrator/vision_strategy.rs` | Vision runtime mode selection |
| `llm/orchestrator/gpu_watchdog.rs` | GPU health/watchdog behavior |
| `llm/orchestrator/child_guard.rs` | Child process cleanup guard |

### Tools And MCP

| File | Purpose |
| ---- | ------- |
| `tools/registry.rs` | Tool definitions, schemas, handlers |
| `tools/preflight.rs` | Deterministic preflight validation |
| `tools/file_ops.rs` | File operations |
| `tools/exec.rs` | Shell/Python execution |
| `tools/packages.rs` | Package operations |
| `tools/google_workspace.rs` | Google Workspace tools |
| `tools/browser_agent.rs` | Browser tools |
| `tools/developer.rs` | Developer/project tools |
| `mcp/client.rs` | MCP client protocol runtime |
| `mcp/server_manager.rs` | MCP server lifecycle |
| `mcp/tool_bridge.rs` | MCP tools exposed as KRIA tools |
| `mcp/payload_shaper.rs` | Compact tool payloads for LLM context |
| `mcp/capability_registry.rs` | Tool execution mode/reliability metadata |

### Safety

| File | Purpose |
| ---- | ------- |
| `safety/policy.rs` | Tool risk tiering and capability classification |
| `safety/command_classifier.rs` | Command-level risk classifier |
| `safety/policy_gate.rs` | Capability-based command safety gate |
| `safety/hitl.rs` | Human approval gateway |
| `safety/audit.rs` | Hash-chained audit logger |
| `safety/rollback.rs` | File rollback snapshots |
| `safety/global_halt.rs` | Global GUI automation kill switch |
| `safety/blacklist.rs` | Hard blacklisted patterns |
| `safety/pin_guard.rs` | PIN-based guard support |

### Infrastructure

| File | Purpose |
| ---- | ------- |
| `infra/isolation.rs` | Isolated tool execution with timeout/cancellation |
| `infra/pipeline_trace.rs` | Runtime trace logging |
| `infra/execution_trace.rs` | Causal tool execution trace |
| `infra/event_bus.rs` | Event bus |
| `infra/health.rs` | Health registry |
| `infra/qos/mod.rs` | Adaptive QoS scheduler |
| `infra/environment/*` | Local/Docker/remote environment abstractions |
| `infra/supervisor.rs` | Supervised task helper |

### Desktop Command Bridge

| File | Purpose |
| ---- | ------- |
| `crates/kria-desktop/src/commands/chat.rs` | Chat stream command |
| `crates/kria-desktop/src/commands/image_chat.rs` | Image chat stream |
| `crates/kria-desktop/src/commands/providers.rs` | Provider UI commands |
| `crates/kria-desktop/src/commands/runtime.rs` | Runtime initialization/status |
| `crates/kria-desktop/src/commands/gui_automation_control.rs` | GUI automation controls |
| `crates/kria-desktop/src/commands/mcp.rs` | MCP management commands |
| `crates/kria-desktop/src/commands/test_runner.rs` | Test runner UI command |
| `crates/kria-desktop/src/commands/voice.rs` | Voice runtime bridge |

---

## 7. Common Debugging Paths

### "The Model Is Unavailable"

Open:

1. `llm/model_router.rs`
2. `llm/failover.rs`
3. `llm/provider/registry.rs`
4. `llm/local.rs`
5. `llm/cloud.rs`
6. `crates/kria-desktop/src/commands/providers.rs`

Check:

- active provider config,
- fallback provider config,
- local server health,
- provider auth,
- context-too-large errors,
- whether vision was required.

### "The Tool Did Not Run"

Open:

1. `agent/loop_engine/mod.rs`
2. `tools/registry.rs`
3. `safety/policy.rs`
4. `safety/hitl.rs`
5. `infra/isolation.rs`

Check:

- Was the tool in `allowed_tool_names`?
- Did policy block it?
- Did HITL timeout or deny?
- Did preflight fail?
- Did `run_isolated` timeout?
- Did the handler return `success: false`?

### "The GUI Workflow Started But Failed"

Open:

1. `agent/gui_wiring.rs`
2. `agent/htn_executor.rs`
3. `tools/gui_automation.rs`
4. `safety/global_halt.rs`
5. `agent/workflow_continuation/mod.rs`
6. `agent/execution_verifier_bounded.rs`

Check:

- global halt state,
- uinput/sidecar health,
- focus/window state,
- Wayland/X11 compatibility,
- observable completion result,
- continuation blockers.

### "The Final Answer Claimed Success Incorrectly"

Open:

1. `agent/result_synthesizer.rs`
2. `agent/execution_interpreter.rs`
3. `agent/loop_engine/response_helpers.rs`
4. `agent/execution_verifier_bounded.rs`
5. `mcp/payload_shaper.rs`

Check:

- Did the LLM see a shaped payload that hid the error?
- Did the tool result include `TOOL_ERROR`?
- Did the synthesizer override raw model text?
- Was observable completion enabled?

### "A Dangerous Command Was Allowed"

Open immediately:

1. `safety/blacklist.rs`
2. `safety/command_classifier.rs`
3. `safety/policy.rs`
4. `safety/audit.rs`
5. `agent/loop_engine/mod.rs`

Check:

- command classification,
- protected path escalation,
- destructive modality hint,
- eval mode (`KRIA_EVAL_MODE`),
- audit decision,
- whether the path bypassed `PolicyEngine`.

---

## 8. Test And Eval Navigation

| What You Need | Where To Look |
| ------------- | ------------- |
| General test notes | `tests/testing.md` |
| E2E browser/Tauri tests | `tests/e2e/` |
| KRIA test binary | `crates/kria-core/src/bin/kria-test.rs` |
| GUI E2E test binary | `crates/kria-core/src/bin/test_gui_e2e.rs` |
| Test runner core | `crates/kria-core/src/test_runner/mod.rs` |
| Desktop test command | `crates/kria-desktop/src/commands/test_runner.rs` |
| Generated reports | `tests-logs/` after running eval/test suites |
| JSON test results | `tests-logs/**/test_result_*.json` after running eval/test suites |

Useful eval docs:

- `docs/evaluations/overview.md`
- `docs/evaluations/gui-e2e.md`
- `docs/decisions/adr/001-e2e-eval-harness.md`
- `docs/decisions/adr/002-tool-execution-overhaul.md`

---

## 9. Concept-To-File Index

| Concept | First File To Open | Supporting Files |
| ------- | ------------------ | ---------------- |
| Prompt lifecycle | `agent/loop_engine/mod.rs` | `agent/prompt_compiler.rs`, `llm/model_router.rs` |
| Streaming UI events | `agent/loop_engine/mod.rs` | `crates/kria-desktop/src/commands/chat.rs` |
| Tool calls | `agent/response_parser.rs` | `tools/registry.rs`, `agent/loop_engine/mod.rs` |
| Tool safety | `safety/policy.rs` | `safety/hitl.rs`, `safety/audit.rs` |
| Shell command safety | `safety/command_classifier.rs` | `safety/policy_gate.rs` |
| Provider switching | `llm/provider/registry.rs` | `llm/model_router.rs`, `llm/failover.rs` |
| Local model process | `llm/orchestrator/server_manager.rs` | `llm/local.rs` |
| Context budget | `llm/budget.rs` | `llm/mod.rs`, `mcp/payload_shaper.rs` |
| PSDG context | `agent/psdg/context_injector.rs` | `agent/psdg/*`, `agent/world_model/*` |
| GUI automation | `agent/gui_wiring.rs` | `agent/htn_executor.rs`, `tools/gui_automation.rs` |
| Observable completion | `agent/observable_completion/mod.rs` | `agent/execution_verifier_bounded.rs` |
| Recovery options | `agent/loop_engine/mod.rs` | `agent/workflow_continuation/mod.rs` |
| Google Workspace | `tools/google_workspace.rs` | `tools/google_workspace_contract.rs` |
| MCP tools | `mcp/server_manager.rs` | `mcp/client.rs`, `mcp/tool_bridge.rs` |
| Voice runtime | `voice/*` | `crates/kria-desktop/src/commands/voice.rs` |
| Image generation | `image/*` | `tools/image_generation.rs` |
| Fleet/remote devices | `crates/kria-desktop/src/device_control.rs` | `tools/device_*` command modules |

---

## 10. Maintainer Checklists

### Adding A New Tool

- Add handler and schema.
- Register it in `ToolRegistry`.
- Add risk tier in `PolicyEngine`.
- Add capability profile if execution mode matters.
- Add preflight validation if arguments can be unsafe or ambiguous.
- Add tests for routing and safety.
- Confirm final output is grounded in tool result.

### Adding A New Provider

- Add `ProviderType`.
- Add `ProviderConfig` default behavior.
- Implement `LlmBackend`.
- Normalize capabilities.
- Normalize errors.
- Normalize streaming.
- Normalize tool calls.
- Wire into `ProviderRegistry`.
- Add connection tests.

### Adding A GUI Workflow

- Decide whether GUI is truly needed.
- Add/adjust intent classification.
- Add GUI plan/substrate behavior.
- Ensure policy/HITL is still applied.
- Add observable completion checkpoint.
- Test on X11/XWayland/Wayland where relevant.

### Changing Safety Policy

- Add or modify risk tier.
- Add command classifier case if shell-related.
- Add protected path or blacklist case if needed.
- Add audit expectations.
- Add HITL behavior tests.
- Check eval mode does not mask production behavior.

### Changing Prompt Behavior

- Prefer typed prompt sections.
- Define section priority.
- Keep context bounded.
- Add omission/truncation behavior.
- Validate tool selection behavior.
- Avoid putting policy authority in the prompt.

---

## 11. Minimal Mental Model

If you are lost, return to this:

```text
AgentLoop owns the turn.
ModelRouter chooses cognition.
LlmBackend normalizes providers.
ToolRegistry owns tools.
PolicyEngine owns permission.
HitlGateway owns approval.
AuditLogger records decisions.
run_isolated executes handlers.
Verifier/Synthesizer decide what can be truthfully reported.
```

---

## Vision Gap Navigation: Where To Fix Current Weaknesses

This section is a practical repair map. It points from KRIA's desired coworker behavior
to the files most likely responsible for current architecture, implementation, running,
or data-flow issues.

### Current Failing Signals

From the latest full test report:

| Failure | Symptom | Start Here |
| ------- | ------- | ---------- |
| MCP prompt-output tests | missing `ToolEnd` for `mcp_gworkspace_*` tools | `agent/loop_engine/mod.rs`, `mcp/tool_bridge.rs`, `mcp/client.rs`, `tests/mcp_prompt_output_integration_tests.rs` |
| Smoke routing test | service-status prompt routed to `manage_service` instead of expected `execute_bash` | `agent/router.rs`, `routing/*`, `tools/system_config.rs`, `tests/test_smoke_system.rs` |

### If KRIA Feels Like A Chatbot

Open these first:

1. `agent/loop_engine/mod.rs`
2. `agent/goal_tree.rs`
3. `agent/opgraph.rs`
4. `agent/workflow_compiler.rs`
5. `agent/stage_executor.rs`

Likely issue:

- multi-step tasks are not being promoted into explicit workflow graphs.

Implementation target:

- make complex prompts graph-first and use the LLM as planner/synthesizer, not as the
  only step-by-step controller.

Impact:

- KRIA behaves more like an operational coworker with a plan and less like a chat loop.

### If KRIA Chooses The Wrong Tool

Open these first:

1. `agent/turn_gate.rs`
2. `agent/router.rs`
3. `routing/intent_classifier.rs`
4. `routing/tool_index.rs`
5. `agent/loop_engine/intent_fallback.rs`
6. `mcp/capability_registry.rs`

Likely issue:

- route semantics are unclear between inspect/manage, browser/search, GUI/API, or
  local/remote variants.

Implementation target:

- add route contracts per operation class:
  - inspect,
  - create,
  - modify,
  - delete,
  - execute,
  - open/navigate,
  - recover.

Impact:

- fewer surprising tool choices and easier tests.

### If KRIA Opens Apps But Does Not Complete Work

Open these first:

1. `agent/gui_wiring.rs`
2. `agent/htn_executor.rs`
3. `agent/observable_completion/mod.rs`
4. `agent/execution_verifier_bounded.rs`
5. `agent/workflow_expectation/mod.rs`

Likely issue:

- app/window success is being confused with semantic completion.

Implementation target:

- add completion contracts per workflow class and require verifier evidence before final
  "done" responses.

Impact:

- fewer hidden-output and false-success failures.

### If External Systems / MCP Feel Unreliable

Open these first:

1. `mcp/client.rs`
2. `mcp/server_manager.rs`
3. `mcp/tool_bridge.rs`
4. `mcp/payload_shaper.rs`
5. `tools/mount_manager.rs`
6. `agent/loop_engine/mod.rs`

Likely issue:

- MCP tool discovery/invocation may not produce complete stream lifecycle events.

Implementation target:

- enforce event contract:

```text
MCP tool selected
  -> ToolStart
  -> raw payload captured
  -> shaped LLM payload
  -> ToolPayloadChunk if needed
  -> ToolEnd success/failure
```

Impact:

- Jira/API/MCP workflows become visible, testable, and recoverable.

### If Local Model Runtime Is Too Heavy Or Unstable

Open these first:

1. `llm/local.rs`
2. `llm/orchestrator/server_manager.rs`
3. `llm/orchestrator/strategy.rs`
4. `llm/orchestrator/vram_budget.rs`
5. `llm/model_router.rs`

Likely issue:

- routing does not fully account for task capability, VRAM pressure, context size, and
  fallback requirements.

Implementation target:

- add task capability profiles and hardware-aware routing decisions.

Impact:

- better RTX 4050 6GB behavior, fewer local OOM/degraded surprises.

### If Safety Feels Too Noisy Or Too Weak

Open these first:

1. `safety/policy.rs`
2. `safety/command_classifier.rs`
3. `safety/hitl.rs`
4. `safety/audit.rs`
5. `tools/registry.rs`

Likely issue:

- policy tier, reversibility, or destructive modality metadata is incomplete.

Implementation target:

- add `SafetyEnvelope`, per-tool reversibility, and full policy coverage tests.

Impact:

- KRIA asks for help at the right times and acts automatically when safe.

### Best Next Engineering Order

| Order | Work | Why First |
| ----- | ---- | --------- |
| 1 | Fix failing MCP and smoke tests | Stabilizes current runtime before new intelligence |
| 2 | Add event lifecycle invariant tests | Prevents silent tool execution gaps |
| 3 | Add unified `TurnFrame` / runtime ledger | Makes orchestration debuggable |
| 4 | Make multi-step prompts graph-first | Moves KRIA toward coworker workflows |
| 5 | Strengthen GUI/browser/IDE semantic substrates | Reduces keyboard-puppeteering behavior |
| 6 | Add capability-aware model routing | Improves local-first intelligence and hardware fit |
| 7 | Add richer HITL/recovery UX | Improves collaboration and trust |
