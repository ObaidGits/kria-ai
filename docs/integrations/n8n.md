# KRIA n8n Integration

## 1. Purpose

n8n integration provides workflow-execution substrate connectivity for automation chains that are better expressed as external workflow graphs. KRIA remains orchestration authority and treats n8n as bounded execution substrate.

Responsibilities:
- Define contract boundaries for invoking n8n workflows.
- Ensure n8n invocation is policy-governed and observable.
- Preserve deterministic authority: KRIA decides when workflows run.

Non-goals:
- n8n is not a planner or authority layer for KRIA runtime.
- n8n does not bypass tool/policy/HITL controls.

## 2. Architecture Overview

Current implementation reality:
- KRIA core now exposes a first outbound n8n substrate module in `crates/kria-core/src/n8n`.
- The current slice supports versioned allowlisted workflow invocation through the `n8n_invoke_workflow` tool.
- KRIA core also exposes signed callback parsing and in-memory workflow state ingestion primitives.
- The desktop local API exposes `POST /api/n8n/callback` for signed callback ingestion.
- Callback events are persisted to a JSONL inbox and replayed during runtime startup.
- The KRIA dashboard now has an n8n tab for configured workflows, callback URL, run state, dead letters, and read-only n8n discovery.
- KRIA evaluates terminal callback evidence against configured `expected_evidence` before emitting a continuation decision.
- n8n workflows can request KRIA HITL through callback evidence and poll `/api/n8n/hitl-response`.
- Operators can reconcile a known n8n run against the n8n execution API from the dashboard.
- n8n execution is still treated as an external integration substrate reached through controlled tooling paths.

Architecture contract:
1. Orchestrator chooses workflow invocation explicitly.
2. Invocation passes through standard safety and execution gating.
3. Workflow results return either synchronously from the invocation response or asynchronously through a signed callback envelope.
4. Callback evidence is ingested as workflow state, not treated as final truth by itself.

## 3. Runtime Execution Flow

1. Intent analysis determines workflow substrate is appropriate.
2. Orchestrator selects integration call path and prepares bounded input payload.
3. Policy/HITL enforces risk controls for external side effects.
4. Workflow executes externally; result is ingested as tool/integration response or callback event.
5. Callback state rejects duplicate, stale, and post-terminal events.
6. Verifier/satisfaction logic decides continuation, fallback, or termination.

Authority boundaries:
- KRIA controls initiation, retries, cancellation, and final decisioning.
- n8n executes delegated steps only.

## 4. Core Components

| Component | Contract |
|---|---|
| Orchestrator loop | Chooses if/when n8n substrate is used |
| `n8n` catalog/client | Validates workflow allowlist, version, status, payload size, and signed dispatch |
| `n8n` callback parser | Validates callback signature, schema version, workflow identity, and workflow version |
| `n8n` workflow state store | Tracks callback sequence, evidence log, side effects, terminal state, and dead letters |
| Local API bridge | Receives signed n8n callbacks at `/api/n8n/callback` |
| Durable callback inbox | Appends callback records to JSONL for restart replay |
| Governance evaluator | Converts callback state into verification and continuation decisions |
| HITL bridge | Creates KRIA approval requests for n8n approval callbacks and exposes pollable responses |
| Reconciliation command | Reads n8n execution state by run ID for operator recovery/debugging |
| Dashboard n8n tab | Shows workflow catalog, callback URL, run state, governance decisions, discovery output, and dead letters |
| `n8n_invoke_workflow` tool | Serializes request + parses response through ToolRegistry |
| Safety policy/HITL | Governs dangerous external side-effect operations |
| Audit pipeline | Records invocation decisions/outcomes |

Invariants:
- No workflow runs without orchestrator-triggered call.
- External workflow output is advisory input to KRIA, not authority output.
- Asynchronous callback evidence never bypasses verifier authority.
- Duplicate/stale callback events are dead-lettered instead of mutating active workflow state.
- A completed n8n run without required evidence becomes `pause_for_hitl`, not silent success.
- HITL responses are explicit and pollable by request ID.

## 5. Integration Contracts

| Integration | Contract |
|---|---|
| Orchestration | KRIA remains single source of execution authority |
| Providers | Provider output can suggest workflows; does not execute directly |
| Tools | n8n invocation must be represented through controlled execution interfaces |
| Memory | Workflow outputs may be persisted through memory contracts only |
| OpenClaw/MCP | n8n is one substrate among peers, selected by capability and policy |
| Hardware | Workflow-triggered local actions still constrained by hardware policies |
| Safety | Risk-tier and HITL rules apply before external side effects |
| GUI/Browser | n8n may complement but not supersede GUI/browser substrate controls |

## 6. Failure Handling & Recovery

- Workflow endpoint unavailable: mark substrate failure and route to alternate strategy.
- Execution timeout: cancel/abort and return structured failure.
- Partial completion: classify side effects and continue with explicit compensating logic if defined.
- Repeated failures: backoff and avoid tight retry loops.
- Duplicate callbacks: ignore state mutation and record a dead letter.
- Out-of-order callbacks: preserve current state and record a dead letter.
- Post-terminal callbacks: preserve terminal state and record a dead letter.

Recovery strategy:
- Prefer deterministic fallback substrate paths over opaque repeated n8n retries.

## 7. Performance & Constraints

Constraints:
- Network and remote runtime latency dominate.
- Workflow queueing and external service limits affect tail latency.
- Payload size and serialization overhead impact responsiveness.

Operational tradeoff:
- n8n improves complex workflow composability but adds external dependency surface.

## 8. Security & Safety

Trust boundaries:
- n8n is external and untrusted from KRIA core authority perspective.

Controls:
- Inputs must be bounded and validated before dispatch.
- High-risk actions require policy escalation and possible HITL.
- Credentials and endpoints are managed as deployment configuration, not runtime authority.
- Discovery is read-only; imported workflows are saved as draft and are not executable until explicitly approved in KRIA config.

## 9. UI + Callback Usage

Dashboard:
- Open `Dashboard -> n8n`.
- Copy the displayed callback URL into n8n workflow callback nodes.
- Use `Discover` to inspect n8n workflows through the configured n8n API.

Callback contract:
- Method: `POST`
- Path: `/api/n8n/callback`
- Header: `x-kria-signature: sha256=<hmac>`
- Body schema: `kria.n8n.callback.v1`

The callback body must include:
- `correlation_id`
- `causation_id`
- `event_id`
- `sequence_number`
- `workflow_id`
- `workflow_version`
- `n8n_run_id`
- `status`
- `evidence`
- `occurred_at_ms`

HITL bridge:
- n8n sends callback status `waiting_for_approval` or a callback whose evidence requires human review.
- KRIA creates a normal HITL approval request.
- n8n polls `GET /api/n8n/hitl-response?request_id=<id>`.
- Response is `pending` until the user approves, denies, or the request times out.

Continuation behavior:
- `completed` + all expected evidence present -> `continue_workflow`
- `completed` + missing expected evidence -> `pause_for_hitl`
- `failed` / `cancelled` / `timed_out` / `rejected` -> `recover_workflow`
- non-terminal statuses -> `await_more_events`

## 10. Observability

Capture:
- Invocation latency, success/failure, timeout, retry counts.
- Callback correlation ID, event ID, sequence number, run status, and dead-letter reason.
- Policy denials and approval paths for workflow calls.
- External endpoint health and error taxonomy.
- Correlation IDs linking turn, workflow run, and resulting side effects.

Evaluation:
- n8n scenarios belong in integration regressions under `docs/evaluations/overview.md`.

## 11. Still Not Authority-Complete

Implemented now:
1. Outbound allowlisted invocation.
2. Signed async callback ingestion.
3. Durable callback inbox replay.
4. Dashboard visibility.
5. Read-only discovery and draft import command surface.
6. Governance decisions for continuation, recovery, HITL pause, and missing evidence.
7. HITL response bridge for n8n approval workflows.
8. n8n execution reconciliation command.

Still future:
1. Deep GoalTree executor resume from continuation events where the paused session is known.
2. Formal verifier federation over external SaaS side effects.
3. Rich global SQLite audit-ledger integration beyond n8n JSONL governance records.
4. Push-based n8n resume webhooks instead of polling HITL responses.
