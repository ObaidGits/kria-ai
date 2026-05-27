# KRIA OpenClaw Integration

## 1. Purpose

OpenClaw is an execution substrate integrated into KRIA for bounded external task execution. KRIA remains orchestration authority; OpenClaw provides specialized containerized execution capabilities exposed as tools.

Responsibilities:
- Boot OpenClaw subsystem and persistent state.
- Register OpenClaw-backed tools (`oc_*`) into `ToolRegistry`.
- Track OpenClaw sessions/tasks/audit metadata.
- Route OpenClaw calls through standard safety and orchestration gates.

Non-goals:
- OpenClaw does not perform global planning or orchestration authority decisions.

## 2. Architecture Overview

Primary implementation:
- `crates/kria-core/src/openclaw/mod.rs`
- `crates/kria-core/src/openclaw/init.rs`
- `crates/kria-core/src/openclaw/types.rs`

Architecture:
1. `OpenClawSubsystem::boot` initializes DB tables and runtime state.
2. `register_tools` mounts `oc_*` tools into the shared registry.
3. Tool calls enter normal loop execution path (authority + policy + isolation).
4. Results are returned as tool output and can feed orchestration state.

## 3. Runtime Execution Flow

1. Orchestrator selects an `oc_*` tool for a turn.
2. Preflight + execution authority validate target and context.
3. Policy/HITL gates approve or deny high-risk operations.
4. OpenClaw adapter executes substrate action and returns structured result.
5. Loop verifier/satisfaction handling determines continuation or stop.

Authority boundaries:
- KRIA controls when OpenClaw is called.
- OpenClaw executes only requested tasks under KRIA-issued call contract.

## 4. Core Components

| Component | Location | Contract |
|---|---|---|
| `OpenClawSubsystem` | `openclaw/mod.rs` | Lifecycle owner (boot/register) |
| Init/bootstrap | `openclaw/init.rs` | Durable storage and startup wiring |
| Types/contracts | `openclaw/types.rs` | Typed task/session/result representation |
| Tool registration | `openclaw/mod.rs` | Binds `oc_*` calls into global tool surface |

Invariants:
- OpenClaw tools execute through the same global safety pipeline as other tools.
- OpenClaw state initialization is deterministic at boot.
- OpenClaw results are treated as substrate outputs, not orchestration decisions.

## 5. Integration Contracts

| Integration | Contract |
|---|---|
| Orchestration | OpenClaw calls require explicit orchestrator selection |
| Providers | Providers may suggest calls; they never execute OpenClaw directly |
| Tools | OpenClaw capabilities are exposed as first-class tools |
| Memory | OpenClaw outcomes can be persisted via memory pipeline |
| n8n/MCP | OpenClaw can coexist with other substrates; no authority transfer |
| Hardware | Containerized workloads still constrained by host capacity/policy |
| Safety | Policy + HITL always gate risky OpenClaw operations |
| GUI/Browser | OpenClaw is parallel substrate, not replacement authority |

## 6. Failure Handling & Recovery

- Startup failure: OpenClaw subsystem remains unavailable; orchestrator degrades to other substrates.
- Tool execution failure: return structured error; retry only when idempotent and policy-safe.
- Timeout/cancellation: isolated execution boundaries prevent hanging the turn.
- Partial substrate outage: fallback to MCP/native/browser/shell tools where appropriate.

Recovery principle:
- Prefer deterministic substrate fallback over repeated failing OpenClaw retries.

## 7. Performance & Constraints

Constraints:
- Container startup and bridge overhead add latency.
- Long-running OpenClaw tasks can consume turn/tool budgets.
- Heavy workloads are bounded by host CPU/RAM/IO and policy limits.

Tradeoff:
- OpenClaw increases capability isolation, but not at zero latency cost.

## 8. Security & Safety

Trust boundaries:
- OpenClaw runtime is an external substrate boundary relative to KRIA core.

Controls:
- Dangerous actions require policy escalation and possible HITL.
- Execution authority prevents ambiguous destructive targeting.
- Audit logs capture request/decision/outcome lifecycle.

## 9. Observability

Required telemetry:
- OpenClaw call count, success/failure, timeout rate.
- Policy denials and HITL outcomes for `oc_*` tools.
- Substrate availability and boot health.
- Latency distribution by OpenClaw capability class.

Evaluation:
- Include OpenClaw routing and failure scenarios in `docs/evaluations/overview.md`.

## 10. Future Evolution

1. Harden capability metadata to improve deterministic substrate selection.
2. Expand per-capability SLO metrics and error taxonomy.
3. Improve recovery profiles when OpenClaw is partially unavailable.
4. Keep authority centralized in KRIA orchestration as integration depth grows.
