# KRIA Runtime Authority

## Purpose

The orchestration subsystem is KRIA's execution authority plane. It decides how
a turn is admitted, routed, constrained, executed, verified, synthesized, and
terminated.

Primary implementation areas:
- `crates/kria-core/src/agent/loop_engine/mod.rs`
- `crates/kria-core/src/agent/turn_context.rs`
- `crates/kria-core/src/agent/turn_gate.rs`
- `crates/kria-core/src/agent/intent_gate.rs`
- `crates/kria-core/src/agent/execution_authority.rs`
- `crates/kria-core/src/safety/policy.rs`
- `crates/kria-core/src/safety/hitl.rs`
- `crates/kria-core/src/tools/registry.rs`

Core invariant:

```text
No external action may bypass admission, routing, preflight, target authority,
policy/HITL, cancellation, isolation, audit, and result synthesis boundaries.
```

## Authority Hierarchy

| Layer | Authority | Must not do |
|---|---|---|
| `TurnAdmission` | Owns active turn, queueing, supersession, stale-turn handling | Execute tools |
| `TurnCancellationTree` | Owns per-turn cancellation tokens for LLM, tools, sidecars, MCP, image work | Decide policy |
| `IntentGate` | Classifies conversational vs execution intent before expensive routing | Call tools or network |
| `TurnGate` | Produces operation/resource plan/tool hints | Execute tools or bypass policy |
| `AgentLoop` | Runs bounded tool rounds and termination logic | Bypass gates for speed |
| `ToolRegistry` | Owns tool definitions, handlers, environment provider, shell state, context creation | Decide top-level intent |
| `ExecutionAuthority` | Resolves and validates execution target | Execute or approve |
| `PolicyEngine` | Owns risk tiering and approval/block decision | Execute |
| `HitlGateway` | Owns approval request/result/timeout lifecycle | Auto-approve guarded actions |
| `run_isolated` | Owns timeout/cancellation wrapper around execution closure | Change policy |
| `ExecutionVerifier` | Validates evidence for non-trivial outcomes when attached | Replan or retry |
| `ResultSynthesizer` | Converts raw results into user-facing summary and metadata | Claim unverified success |
| Integration tools | Perform delegated work after authorization | Become planning or safety authority |

## Turn Lifecycle

Current turn flow:

```text
request
  -> TurnAdmission
  -> cancellation tree
  -> IntentGate
  -> TurnGate
  -> model/tool routing loop
  -> tool execution gates
  -> verifier
  -> result synthesis
  -> TurnMemory satisfaction check
  -> final stream/result
```

Important behavior:
- one active turn per session,
- bounded queueing with stale-turn rejection,
- per-turn child cancellation for tools, sidecars, MCP, and image work,
- bounded tool rounds with early stop when `TurnMemory` detects satisfaction,
- duplicate-success memoization and duplicate-failure guards inside a turn,
- stale turn checks after awaited work.

## Tool Execution Authority Path

For each model-selected or system-injected tool call:

```text
tool call
  -> PolicyEngine evaluate_with_modality_hint
  -> HITL when required
  -> duplicate and budget guards
  -> tools/preflight
  -> execution_authority target validation
  -> ToolRegistry handler lookup
  -> ToolContext creation
  -> run_isolated timeout/cancellation
  -> optional ExecutionVerifier
  -> ResultSynthesizer
  -> TurnMemory update
```

`PolicyEngine` currently runs before preflight and target authority in the loop.
Preflight and execution authority still run before the handler is dispatched, so
blocked or ambiguous calls do not reach the tool closure.

## Execution Target Authority

`execution_authority.rs` resolves the target binding for each call.

Supported targets include:
- host,
- VM,
- Docker,
- Colab,
- MCP,
- cloud provider,
- browser.

Binding source priority:
1. explicit user target,
2. tool-implied target,
3. turn-level inferred target,
4. default host target.

Validation results:
- `Authorized`: target/tool pair may execute,
- `Blocked`: mismatch or unsafe target selection,
- `NeedsClarification`: destructive or ambiguous action requires user choice.

Examples:
- fleet command tools are VM-only,
- Google Workspace tools are cloud-provider tools,
- MCP tools run under MCP/Colab targets,
- browser/navigation/application tools allow browser or host targets,
- destructive shell/file/package/system operations require stronger confidence.

## Policy And HITL

Risk tiers:
- Green: auto-execute,
- Yellow: execute and notify,
- Red: requires HITL approval,
- Black: hard block.

Policy behavior:
- blacklist checks run first,
- shell tools receive command-level classification,
- KRIA-generated bounded code execution can be Green,
- protected-path writes escalate to Red,
- unknown actions default to Red,
- `KRIA_EVAL_MODE` may suppress some Red approval in controlled eval paths,
- debug builds assert that a decision cannot be both blocked and approval-required.

HITL behavior:
- approval request includes action, params, risk level, and request id,
- denial and timeout both prevent execution,
- approved identical tool+args calls may reuse approval within the same turn,
- Red decisions are audited.

## GUI Authority

GUI workflows enter the same authority plane. The GUI path adds semantic and
fidelity contracts before substrate planning:

```text
GuiTaskSpec
  -> SemanticWorkflowAnalysis
  -> ExecutionModeDecision
  -> WorkflowIntentContract check
  -> VerifierAuthorityAssessment
  -> SubstratePlanner
```

This adds workflow-mode and visible-fidelity metadata, but it does not bypass
tool execution gates. GUI/browser/app lifecycle tools still pass through
preflight, target authority, policy/HITL, isolation, verifier, and synthesis.

## OpGraph And GoalTree Authority

`OpGraph` is planning-only. `GoalTree` is the immutable execution contract.
`StageExecutor` executes ordered stages and does not replan or mutate the graph.

Execution authority remains below the stage/action layer: any stage action that
dispatches a tool still uses the same tool registry, policy, HITL, isolation,
and verifier contracts.

## Integration Boundaries

| Surface | Runtime authority rule |
|---|---|
| Model providers | Generate text/tool-call proposals only; they do not execute |
| MCP | External delegated tools; routing and policy remain KRIA-owned |
| Google Workspace | Cloud-provider tool target; send/delete/reply are destructive and approval-gated |
| OpenClaw | External delegated automation surface, not planning authority |
| n8n | Workflow invocation is a tool call behind policy and audit |
| Browser/GUI | Live desktop substrate behind the same gates and readiness checks |
| Shell | Host/VM/Docker target validation plus command classifier |
| Memory/RAG | Persistent memory has its own subsystem; turn-loop memory is turn-scoped |
| Hardware/GPU | Resource lease and provider orchestration coordinate capacity, not safety policy |

## Failure Handling

| Failure | Expected handling |
|---|---|
| Queue full | Reject or report queued turn failure |
| Superseded/stale turn | Stop work and drop stale result |
| Intent uncertainty | Ask or route to clarification path |
| Policy Black | Block and audit, no execution |
| HITL denial/timeout | Do not execute, report failure/denial |
| Preflight block | Return `PREFLIGHT_BLOCKED`, no handler dispatch |
| Target mismatch | Return `EXECUTION_BLOCKED`, no handler dispatch |
| Clarification needed | Return clarification-needed result, no handler dispatch |
| Isolation timeout | Return structured tool failure |
| Verifier failure | Downgrade successful tool result to failure |
| Duplicate failed call | Abort repeated identical failure to prevent loops |

## Observability

Runtime observability includes:
- structured `log_pipeline_step` traces for routing, policy, preflight,
  authority, execution, verification, synthesis, and termination,
- stream events for tool start/end/progress, approvals, task steps, and errors,
- `AuditLogger` records decisions and actors,
- synthesized result metadata is attached to tool-end events,
- verifier evidence/confidence is logged when available.

Operational metrics to track:
- turn admission outcome,
- queue/stale-turn count,
- tool rounds per turn,
- early satisfaction rate,
- policy tier distribution,
- HITL approval/denial/timeout rate,
- execution-authority block/clarification count,
- tool timeout/failure rate,
- verifier failure rate.

## Non-Negotiable Rules

- No tool handler is trusted to self-govern safety.
- No integration backend owns routing or policy authority.
- No blocked, denied, stale, or unverified action may be reported as completed.
- No GUI visible-workflow requirement may be silently downgraded to backend-only
  success.
- Any new execution surface must enter through the same authority path or have a
  documented, audited exception.
