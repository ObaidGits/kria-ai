# KRIA Orchestration Runtime Authority

## 1. Purpose

The orchestration subsystem is KRIA's execution authority plane. It decides how a turn is admitted, routed, constrained, executed, and terminated under policy.

Responsibilities:
- Enforce single-turn authority per session with explicit queue/supersession behavior.
- Classify intent and compute class (`TurnGate`, `IntentGate`) before tool execution.
- Enforce execution boundaries (preflight, target validation, policy/HITL, isolation).
- Coordinate execution and cancellation across tools, sidecars, MCP, image, and model calls.
- Terminate loops when goals are satisfied or risk/confidence rules require clarification/refusal.

Non-goals:
- It is not an autonomous planner outside bounded contracts.
- It does not bypass safety for speed.
- It does not let integration backends (MCP/OpenClaw/cloud APIs) become routing authority.

Architectural importance:
- This layer preserves KRIA's core invariant: deterministic, bounded, policy-governed execution.

## 2. Architecture Overview

Runtime structure (implemented in `crates/kria-core/src/agent/loop_engine/mod.rs` and related modules):

1. **Admission and cancellation boundary**
   - `TurnAdmission` manages one active turn per session and optional queueing.
   - `TurnCancellationTree` creates per-turn child tokens (`l0`, `l1`, `tools`, `sidecar`, `mcp`, `image`).

2. **Intent and resource boundary**
   - `IntentGate` classifies conversational vs execution intent early (deterministic, no LLM/network).
   - `TurnGate` compiles `IntentEnvelope` + `ResourcePlan` and tool hints.
   - Optional classifier stack: deterministic guards, semantic router, optional ONNX classifier, optional new intent classifier.

3. **Execution boundary**
   - Tool schemas are mounted/routed per round, then executed through `ToolRegistry`.
   - Every call passes:
     - preflight validation (`tools/preflight`)
     - execution target authority (`agent/execution_authority`)
     - policy evaluation (`safety/policy`)
     - HITL approval for required Red actions (`safety/hitl`)
     - isolated runtime wrapper with timeout/cancellation (`infra/isolation::run_isolated`)

4. **Verification and loop boundary**
   - Optional post-execution verifier validates non-trivial outcomes (`execution_verifier`).
   - Result synthesis normalizes tool output into conversational summaries plus execution metadata.
   - `TurnMemory` tracks action completion + memoization + goal satisfaction.
   - Round loop breaks early on satisfaction to avoid redundant tool rounds.

## 3. Runtime Execution Flow

### 3.1 Turn lifecycle

1. A new turn is admitted (`TurnAdmission::admit_or_enqueue_turn`).
2. If busy and queueing is requested, turn is queued; otherwise existing active turn is superseded.
3. Root cancellation tree is created and bound to session+turn.
4. `IntentGate` runs first (conversation-first suppression for non-execution turns).
5. `TurnGate::plan_turn` produces operation + `ResourcePlan` + tool hints.
6. Loop executes up to `max_tool_rounds` (default 10), but exits early on satisfaction.
7. On completion or cancellation, turn is finalized and next queued turn may be promoted.

### 3.2 Tool execution lifecycle (authoritative path)

For each selected tool call:
1. **Preflight**: deterministic block/warn check before process/tool dispatch.
2. **Execution authority**:
   - `check_execution_authority(tool, user_text, turn_target)`
   - resolves target binding (host/vm/docker/colab/mcp/cloud/browser)
   - returns `Authorized`, `Blocked`, or `NeedsClarification`
3. **Policy/HITL**:
   - `PolicyEngine::evaluate(action, params)` assigns risk and approval requirement.
   - Red actions request HITL approval; timeout auto-denies.
4. **Isolated execution**:
   - `run_isolated` applies timeout + cancellation token + execution closure.
5. **Audit and optional verification**:
   - policy/HITL decisions and outcomes are logged (`AuditLogger`).
   - optional verifier validates result evidence (no replanning/retry authority).
6. **Result synthesis**:
   - synthesized summaries and execution metadata are emitted for UI/LLM use.

### 3.3 Authority boundaries

- `TurnGate`/`IntentGate` decide route intent; executors cannot silently override with hidden planners.
- `ExecutionAuthority` decides target validity before dispatch (`Authorized` / `Blocked` / `NeedsClarification`).
- `PolicyEngine`/HITL decide whether action may run.
- Integrations execute work but do not own route/policy/memory authority.

## 4. Core Components

| Component | Module | Responsibility | Key invariants |
|---|---|---|---|
| Turn admission | `agent/turn_context.rs` | Active-turn registry, queueing, promotion, supersession | One active turn per session; stale turns are dropped |
| Cancellation tree | `agent/turn_context.rs` | Hierarchical cancellation across execution planes | Root cancel propagates to all children |
| Intent gate | `agent/intent_gate.rs` | Deterministic conversation-first guard | No LLM/network/embedding dependency |
| Turn gate | `agent/turn_gate.rs` | Intent envelope + compute/resource plan + tool hints | Stable typed boundary for top-level planning |
| Agent loop | `agent/loop_engine/mod.rs` | Round orchestration, schema routing, execution loop, synthesis | Bounded rounds; satisfaction-aware early stop |
| Turn memory | `agent/turn_memory.rs` | Per-turn memo, completed actions, satisfaction detection | Turn-scoped only, no cross-turn persistence |
| Execution authority | `agent/execution_authority.rs` | Target binding + compatibility checks | Block/clarify before execution on ambiguity/mismatch |
| Tool registry | `tools/registry.rs` | Tool definition/handler authority, context creation | Unified handler dispatch and schema surface |
| Safety policy | `safety/policy.rs` | Risk tiering and approval requirements | Fail-safe defaults; command-level shell tiering |
| HITL gateway | `safety/hitl.rs` | Approval request/response with timeout semantics | Timeout => deny for guarded actions |
| Audit logger | `safety/audit.rs` | Structured audit logging with hash-chain fields | Append-only decision trail |
| Result synthesizer | `agent/result_synthesizer.rs` | Tool output normalization for UI/LLM | Conversational summary + execution metadata |
| GPU lease manager | `resource/gpu_lease.rs` | GPU ownership arbitration and recovery/degraded states | No conflicting owners; recovery/degraded explicit |
| Provider registry | `llm/provider/registry.rs` | Active provider lifecycle + orchestrator notification | Switches are explicit and location-aware |
| MCP capability registry | `mcp/capability_registry.rs` | Capability metadata + execution mode preference | GUI-last policy support and deterministic capability mapping |

## 5. Integration Contracts

This subsystem is the coordination authority for all major KRIA surfaces:

- **Orchestration core**: `TurnAdmission` + `TurnGate` + `AgentLoop` define runtime sequencing.
- **Providers**: provider selection occurs via `ProviderRegistry`; orchestration treats provider as execution backend and preserves policy/cancellation contracts.
- **Tools**: all tool execution enters through `ToolRegistry` and policy gates.
- **Memory**: orchestration uses `TurnMemory` for turn-scoped control; persistent memory authority remains in memory subsystem.
- **Hardware**: compute pressure and model paths interact with `GpuLeaseManager` and orchestrator telemetry-driven behavior (`docs/operations/hardware.md`).
- **OpenClaw**: treated as tool-capability execution surface, not planning authority.
- **n8n**: workflow invocation must remain typed, auditable, and policy-gated under this authority layer.
- **GUI execution**: GUI workflow execution must pass the same preflight/authority/policy/cancellation contracts.
- **MCPs**: MCP tools are routed through tool and capability registries; execution mode metadata informs selection order.
- **Voice**: voice turns enter same turn authority/cancellation model; voice cannot bypass policy/tool governance.
- **Safety**: `PolicyEngine`, HITL, audit, rollback hooks are mandatory execution guards.

## 6. Failure Handling & Recovery

### Failure classes and handling

1. **Admission failures**
   - Queue full => explicit rejection for the turn.
   - Stale/superseded turn => dropped with done/cancel signal.

2. **Intent uncertainty**
   - `IntentGate`/`TurnGate` low confidence => clarification path.
   - `ExecutionAuthority::NeedsClarification` blocks ambiguous destructive routing.

3. **Preflight or authority block**
   - `PREFLIGHT_BLOCKED` and execution-authority block return explicit tool error.
   - No subprocess/tool side effect is started in blocked path.

4. **Policy/HITL denial**
   - Red action denied or timed out => tool call refused, audited, loop continues or terminates by strategy.

5. **Tool/runtime failures**
   - Isolation timeout or tool error produces structured failure result.
   - Consecutive failure guards and per-call dedup reduce loops.

6. **Post-execution verification failure**
   - Verifier logs confidence/evidence outcomes.
   - Verifier is non-authoritative for replanning; it does not execute retries itself.

7. **GPU recovery/degradation**
   - Lease manager enters recovering/degraded state when reconciliation fails or recovery stalls.
   - Degraded state is explicit and blocks unsafe lease grants.

## 7. Performance & Constraints

### Runtime constraints

- Default maximum tool rounds: **10**.
- Timeout policy is tool-specific (examples in loop engine):
  - package/fleet/image operations can extend to 300s,
  - shell execution around 120s,
  - default short operations around 30s.
- Turn queue limit is bounded (`TurnAdmission` default queue limit per session is 1).

### Bottlenecks

- Large message history and tool result payload growth (managed via context compaction and token budgets).
- Long-running external tools and network dependencies.
- Vision and GPU-heavy operations under constrained VRAM.
- Multi-provider streaming differences normalized at provider layer.

### Tradeoffs

- Deterministic safety checks add latency but prevent unsafe execution drift.
- Strict bounded loops reduce runaway behavior at cost of some aggressive autonomy.
- Clarification-before-destruction favors safety over speculative execution.

## 8. Security & Safety

Trust boundaries:
- Internal Rust orchestrator is trusted authority.
- External execution surfaces (shell, MCP, OpenClaw, cloud APIs, sidecars) are untrusted/less-trusted execution backends.

Safety controls:
- Preflight command/tool validation before execution.
- Target binding validation (cross-target mismatch blocking).
- Policy tiering (Green/Yellow/Red/Black) with command-level classification for shell tools.
- HITL approval with timeout auto-deny.
- Structured audit logging with decision provenance.

Execution authority discipline:
- Unknown or ambiguous high-risk operations fail safe (block/clarify), not guess-and-run.
- Eval mode exception paths are explicit and scoped (e.g., controlled auto-approval behavior for evaluation runs).

## 9. Observability

Runtime observability is built around structured pipeline tracing and audit records:

- **Pipeline steps**: loop engine emits stepwise structured trace events (`log_pipeline_step`) for intent, routing, policy, execution, verification, and termination decisions.
- **Synthesis telemetry**: synthesized summaries and execution metadata are emitted with tool end events.
- **Safety audit**: `AuditLogger` records action, risk, decision, actor, result, duration, and hash-chain fields.
- **HITL telemetry**: approval requests/responses/timeouts are explicit gateway events.
- **Resource telemetry**: GPU lease state, recovery, and degraded transitions are visible.
- **Evaluation linkage**: orchestration behavior is validated via `docs/evaluations/overview.md` and subsystem runbooks in `docs/evaluations/`.

Recommended core metrics:
- turn admission outcomes (admitted/queued/rejected),
- stale-turn drop count,
- tool round count and early-satisfaction termination rate,
- policy/HITL decision distribution,
- execution-authority block/clarification counts,
- timeout/failure rate by tool family,
- verification confidence distribution.

## 10. Future Evolution

Realistic near-term roadmap for this subsystem:

1. **Tighten typed contracts**
   - reduce stringly-typed routing hints in favor of explicit typed routing/capability descriptors.

2. **Stronger verifier integration policy**
   - formalize which tool classes require mandatory verifier pass/fail gating versus advisory logging.

3. **Policy and capability unification**
   - align `PolicyEngine` and capability registries so routing preferences and risk tiers are derived from one canonical capability model.

4. **Cross-plane budget controls**
   - enforce shared turn budgets across tokens, tool rounds, and long-running external executions with consistent rejection semantics.

5. **Operational hardening**
   - expand deterministic regression suites for target binding, satisfaction termination, and degraded-mode recovery behavior.
