# KRIA Tool System

## 1. Purpose

KRIA's tool subsystem is the execution surface used by orchestration to perform external work. It provides typed tool contracts, safe dispatch, and policy-gated execution.

Responsibilities:
- Maintain canonical tool definitions (`ToolDef`, `ParamDef`) and handlers.
- Provide a unified runtime dispatch path through `ToolRegistry`.
- Enforce substrate-aware execution via preflight, execution authority, and policy/HITL gates.
- Keep tool execution bounded, cancellable, and observable.

Non-goals:
- Tools do not decide top-level routing or planning authority.
- Tool handlers are not trusted to self-govern risk policy.

Architectural importance:
- The tool system is the bridge between orchestrator decisions and real side effects.

## 2. Architecture Overview

Primary implementation:
- `crates/kria-core/src/tools/registry.rs`
- `crates/kria-core/src/agent/loop_engine/mod.rs`
- `crates/kria-core/src/agent/execution_authority.rs`
- `crates/kria-core/src/safety/policy.rs`

Execution substrates exposed as tools:
1. Native Rust handlers (`tools/*`).
2. MCP-backed handlers (`mcp_*`).
3. OpenClaw handlers (`oc_*`) via containerized substrate.
4. Shell and environment-backed execution tools (`execute_bash`, `execute_python`, `execute_powershell`).

Selection model:
- Orchestration selects mounted schemas per round.
- Semantic tool injection and direct tool matching narrow tool surface.
- Tool schema exposure is bounded per round.

## 3. Runtime Execution Flow

1. Orchestrator selects candidate tool calls from model output.
2. For each call, orchestration applies:
   - preflight validation (`tools/preflight`),
   - execution target authority (`execution_authority`),
   - policy tiering (`PolicyEngine`),
   - HITL approval for Red actions.
3. Execution runs via `run_isolated` with timeout + cancellation token.
4. Result is logged/audited and optionally passed to execution verifier.
5. Result synthesis produces conversational summaries + execution metadata for UI/LLM use.
6. `TurnMemory` updates satisfaction state; loop can terminate early.

Authority boundaries:
- `TurnGate`/`AgentLoop` own orchestration decisions.
- Tool handlers execute only approved calls with typed params.
- Unapproved/ambiguous/dangerous actions are blocked or clarified before execution.

## 4. Core Components

| Component | Location | Contract |
|---|---|---|
| `ToolDef`, `ParamDef` | `tools/registry.rs` | Typed, serializable schema for LLM/runtime |
| `ToolHandler` | `tools/registry.rs` | Async execution trait, context-aware execution path |
| `ToolRegistry` | `tools/registry.rs` | Thread-safe definition/handler registry and lookup |
| Preflight | `tools/preflight.rs` | Deterministic fast block/warn before execution |
| Execution authority | `agent/execution_authority.rs` | Target compatibility and ambiguity handling |
| Policy gate | `safety/policy.rs`, `safety/policy_gate.rs` | Risk tiering and approval/deny policy |
| HITL | `safety/hitl.rs` | Approval request lifecycle with timeout auto-deny |

Invariants:
- No tool call executes without policy evaluation.
- Target-mismatch calls are blocked before side effects.
- Unknown tool calls are rejected.
- Turn cancellation propagates into tool execution context.
- Tool output is synthesized before user-facing rendering or LLM context use.

## 5. Integration Contracts

| Integration | Contract |
|---|---|
| Orchestration | Tool system executes only orchestrator-approved calls |
| Providers | Model providers choose text/tool outputs; they do not execute tools directly |
| Memory | Tool results may become memory inputs only via orchestrator-controlled flow |
| Result synthesis | Tool outputs are normalized into summaries and metadata for UI/LLM use |
| OpenClaw | `oc_*` tools run in sandbox substrate; KRIA remains authority |
| n8n | n8n workflows are treated as substrate calls behind tool/policy gates |
| MCP | `mcp_*` tools are capability-tagged and routed as external substrates |
| Hardware | GPU/CPU-sensitive tools coordinate with lease/orchestrator constraints |
| Safety | Policy/HITL/audit are mandatory execution gates |
| GUI/Browser | Automation tools are last-resort substrates for many capability classes |

## 6. Failure Handling & Recovery

- Unknown tool: immediate structured error.
- Preflight block: execution denied with explicit reason.
- Execution authority block/clarification: no dispatch until resolved.
- HITL timeout/denial: action denied and audited.
- Isolated timeout/failure: structured failure result returned to loop.
- Repeated failures: consecutive-failure guards and dedup reduce loops.
- Goal satisfaction: early termination avoids redundant retries.

Recovery policies:
- Prefer deterministic alternative tool routes over repeating failed identical calls.
- Keep fallbacks explicit and observable.
- Do not bypass policy during fallback.

## 7. Performance & Constraints

Operational constraints:
- Bounded tool rounds per turn (`max_tool_rounds`).
- Tool-specific timeout classes (short/default/long-running).
- Per-turn cancellation and queue limits.

Performance considerations:
- Overly broad tool schema exposure increases model confusion and latency.
- Tool-result payload growth can pressure context budgets.
- External substrates (MCP/OpenClaw/network) dominate tail latency.

Tradeoff:
- Strict safety gates increase latency but reduce unsafe side effects and drift.

## 8. Security & Safety

Trust model:
- Native handlers are trusted code paths under policy.
- MCP/OpenClaw/shell/browser/GUI are lower-trust substrates.

Safety controls:
- Risk tiering (Green/Yellow/Red/Black).
- Parameter-aware escalation (for command/path-sensitive tools).
- HITL required for guarded destructive actions.
- Audit logging for decisions and outcomes.

Execution authority:
- Tool-target mismatches (host/vm/docker/cloud/mcp/browser) are blocked or clarified.
- No implicit target escalation for destructive operations.

## 9. Observability

Primary telemetry surfaces:
- Pipeline step traces in `AgentLoop`.
- Safety audit entries (decision + actor + result + duration).
- HITL pending/response/timeout events.
- Tool failure/timeout distributions by category.
- Satisfaction-driven early-stop metrics.

Evaluation strategy:
- Validate tool routing, policy handling, and fallback behavior via `docs/evaluations/overview.md`.
- Keep regression coverage for routing-critical tools and dangerous operations.

## 10. Future Evolution

1. Unify capability metadata and policy tiering into one typed source.
2. Tighten per-tool idempotency and replay contracts for safer retries.
3. Expand deterministic fallback graphs for substrate outages.
4. Improve per-tool observability baselines (latency, success, denial, timeout).
5. Keep GUI/browser paths as explicit constrained fallback substrates, not default authority routes.
