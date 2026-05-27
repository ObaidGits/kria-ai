# KRIA Tool System

## Purpose

The tool system is KRIA's side-effect boundary. It exposes typed tool schemas,
stores handlers, creates execution context, and lets the orchestrator dispatch
work only after authority gates have passed.

Primary implementation areas:
- `crates/kria-core/src/tools/registry.rs`
- `crates/kria-core/src/tools/preflight.rs`
- `crates/kria-core/src/agent/execution_authority.rs`
- `crates/kria-core/src/safety/policy.rs`
- `crates/kria-core/src/agent/loop_engine/mod.rs`

Tools do not own top-level routing, safety policy, user approval, or final
truth. They execute authorized work and return structured results.

## Registry Contract

`ToolRegistry` stores:
- `ToolDef`: name, description, category, parameter schema, default risk tier,
  minimum hardware tier,
- `ParamDef`: typed function-call parameter definition,
- `ToolHandler`: async handler trait,
- `ToolResumeCapability`: restart/resume classification,
- environment provider,
- shared shell state.

The registry is thread-safe and supports dynamic registration. This is required
for background MCP/integration startup and for desktop runtimes that re-register
live app/browser handlers.

`ToolContext` is created per execution and carries:
- environment provider,
- shared shell state,
- cancellation token.

Handlers should implement `execute_with_context` when they need environment,
cwd/env state, or cancellation. Legacy `execute` still exists as a fallback.

## Registry Construction

Current builders:
- `build_default_registry()`
- `build_registry_with_store()`
- `build_registry_full()`
- `build_registry_full_with_psdg()`
- `build_registry_full_with_psdg_wcr()`

Common registered categories include:
- system information,
- file operations,
- app lifecycle,
- shell,
- internet/browser,
- knowledge/memory,
- system config and power,
- process and disk,
- documents and communication,
- packages and scheduler,
- desktop and GUI automation,
- AT-SPI tools,
- cognition tools,
- vision and vision automation,
- i18n,
- Google Workspace,
- RAG and proactive tools when configured,
- fleet stubs when fleet runtime is not connected.

Google Workspace tools remain visible even before the MCP account/client is
connected so the desktop runtime can rewire live client state later.

## Execution Flow

Tool execution happens through `AgentLoop`.

```text
tool proposal
  -> PolicyEngine evaluate_with_modality_hint
  -> HITL approval when required
  -> duplicate/budget guards
  -> preflight validation
  -> execution target authority
  -> ToolRegistry handler lookup
  -> ToolContext creation
  -> run_isolated
  -> optional verifier
  -> result synthesis
  -> TurnMemory update
```

Policy is evaluated before dispatch. Preflight and target authority are still
before handler execution, so a blocked call does not reach the tool closure.

## Preflight

`tools/preflight.rs` is deterministic, synchronous, and does not call I/O,
network, tools, or LLMs.

It can block:
- empty shell commands,
- recursive removal of critical/root paths,
- destructive disk commands such as `dd` to device paths,
- filesystem operations against critical paths,
- blocked network metadata endpoints.

It can warn on:
- shell expansion,
- `sudo`,
- curl/wget piped to shell,
- writes under sensitive system locations,
- dotfile writes.

Preflight is fast defense-in-depth. Policy and execution authority remain
separate gates.

## Execution Authority

Execution authority validates target compatibility before a handler runs.

Targets include:
- host,
- VM,
- Docker,
- Colab,
- MCP,
- cloud provider,
- browser.

Policy examples:
- shell can run on host, VM, or Docker but destructive ambiguity blocks,
- fleet commands are VM-targeted,
- `mcp_*` tools are MCP/Colab-targeted,
- Google Workspace tools are cloud-provider-targeted,
- browser/app lifecycle tools allow browser or host targets,
- destructive file/package/system tools require stronger confidence.

Results:
- `Authorized`: dispatch may continue,
- `Blocked`: no dispatch, explicit error,
- `NeedsClarification`: no dispatch until user target is resolved.

## Resume Capability

`ToolResumeCapability` describes whether an interrupted tool can be resumed or
reconstructed safely.

| Capability | Meaning | Typical tools |
|---|---|---|
| `DeterministicLocal` | Local, replayable/reconstructable operation | file writes, directory creation, local shell/python execution |
| `RequiresLiveGui` | Depends on current live desktop/browser state | GUI automation, vision automation, desktop, browser search/open URL |
| `ExternalDelegated` | Delegated to external service/substrate | fleet, Google Workspace, MCP, n8n, OpenClaw |
| `Unsupported` | No safe resume contract known | unknown or uncategorized tools |

New tools should register explicit resume capability. The registry keeps a
compatibility shim for older tools that lack metadata.

## Tool Families

| Family | Examples | Notes |
|---|---|---|
| Local deterministic | `write_file`, `create_directory`, `execute_bash`, `execute_python` | Still policy/preflight/authority gated |
| Desktop/app lifecycle | `open_application`, `focus_window`, `open_url`, `browser_search` | Live environment dependent |
| GUI automation | click/type/focus/window tools | Requires display readiness and verifier discipline |
| Vision automation | screenshot/OCR/image interaction tools | May require OCR/vision sidecar or desktop capture |
| MCP/Colab | `mcp_*`, `mcp_colab*` | External delegated, target-specific |
| Google Workspace | `gw_*` | Cloud-provider target; send/delete/reply are high-risk |
| n8n | workflow invocation tools | External delegated workflow substrate |
| OpenClaw | external browser/automation substrate | Delegated execution, not KRIA authority |
| Memory/RAG/proactive | knowledge and automation tools | Registered only with backing runtime where required |

## Safety Invariants

- Unknown tools return a structured error.
- A handler must not execute when preflight blocks.
- A handler must not execute when target authority blocks or asks for
  clarification.
- Red actions require HITL unless an explicit eval-only exception applies.
- HITL denial or timeout means the operation did not happen.
- Tool results must be synthesized before user-facing rendering.
- Verifier failure downgrades a claimed-success result to failure where verifier
  evidence is attached.
- Tool handlers must not silently create a new top-level plan.

## Cancellability And Isolation

`run_isolated` wraps handler execution with:
- isolation name,
- timeout,
- cancellation token,
- async execution closure.

Timeout classes are selected by tool name:
- package/fleet/image operations can use long timeouts,
- shell/python/powershell use medium timeouts,
- normal tools use shorter defaults.

The `ToolContext` cancellation token lets context-aware handlers stop when a
turn is cancelled or superseded.

## Observability

Tool execution emits:
- tool start/end/progress stream events,
- approval required/result events,
- preflight block traces,
- execution-authority traces,
- policy evaluation traces,
- verifier evidence/confidence traces,
- result-synthesis metadata,
- audit records for safety decisions.

These events are part of the production contract. Do not hide fallback or
partial-completion behavior inside handler logs only.

## Current Limits

- Some registered tools are stubs when their external runtime is not connected.
- Resume capability is partly explicit and partly compatibility-shim based.
- GUI/browser tools depend on live display/session readiness.
- External delegated tools can dominate latency and failure behavior.
- Tool categories remain partly string-based; new code should prefer typed
  capability metadata where practical.
