# KRIA n8n Integration

Last updated: 2026-05-27

## Purpose

n8n is an external workflow substrate for bounded automation chains that are better represented as workflow graphs than as local KRIA tools. KRIA remains the authority plane.

n8n may execute delegated workflow steps, but it does not decide:

- whether a workflow should run,
- whether external side effects are allowed,
- whether returned evidence is sufficient,
- whether KRIA should continue, recover, or pause for human review.

## Current Implementation

The active implementation lives in `crates/kria-core/src/n8n` and desktop command/local API glue lives in `crates/kria-desktop/src/commands`.

Implemented capabilities:

- Versioned, allowlisted workflow invocation through `n8n_invoke_workflow`.
- HMAC-signed outbound command envelopes.
- HMAC-signed callback parsing and schema validation.
- In-memory run state with duplicate, out-of-order, and post-terminal callback rejection.
- JSONL callback inbox persistence and startup replay.
- JSONL governance audit records for n8n decisions.
- Dashboard status, discovery, draft import, reconciliation, run state, dead-letter, and HITL response visibility.
- HITL bridge for n8n workflows that need approval before continuation.

Still intentionally bounded:

- n8n is not a planner.
- n8n callbacks are evidence, not completion truth.
- Deep paused GoalTree resume from n8n continuation events is still a future integration layer.

## Runtime Flow

Outbound invocation:

1. `N8nConfig` is loaded from KRIA config.
2. `register_into_tool_registry` exits without registering anything if n8n is disabled.
3. If enabled, `N8nCatalog` validates base URL, signing secret, workflow IDs, versions, endpoint paths, and workflow status.
4. `n8n_invoke_workflow` is registered into `ToolRegistry`.
5. Tool execution parses `N8nToolRequest`.
6. `N8nClient::invoke` resolves the workflow, builds an `N8nCommandEnvelope`, checks `max_payload_bytes`, signs the payload, and POSTs to the configured endpoint.
7. Non-2xx responses fail the tool call. Successful responses are parsed as JSON and returned in `N8nInvocationResult`.

Callback ingestion:

1. n8n POSTs a `kria.n8n.callback.v1` envelope to `/api/n8n/callback`.
2. KRIA verifies `x-kria-signature`.
3. KRIA validates workflow identity and version against the current catalog.
4. `N8nWorkflowStateStore` ingests the event.
5. Duplicate, out-of-order, or post-terminal events are dead-lettered instead of mutating active state.
6. Accepted callbacks are appended to the callback inbox JSONL file.
7. `evaluate_run` converts run state into a governance decision.
8. Governance decisions are retained in memory, appended to JSONL audit, and emitted to the UI.

HITL bridge:

1. A callback with `waiting_for_approval` or missing required evidence produces `PauseForHitl`.
2. KRIA creates a normal approval request through the HITL gateway.
3. The external workflow can poll `/api/n8n/hitl-response?request_id=<id>`.
4. The poll response remains pending until the user approves, denies, or the request expires.

## Core Components

| Component | Location | Runtime contract |
|---|---|---|
| Config | `n8n/config.rs` | Stores enabled flag, base URL, API key, signing secret, payload limit, timeout, and allowlisted workflows. |
| Catalog | `n8n/catalog.rs` | Resolves only known, version-matching, approved workflows. |
| Client | `n8n/client.rs` | Builds and signs command envelopes, enforces payload size, sends HTTP requests. |
| Tool handler | `n8n/tool.rs` | Exposes `n8n_invoke_workflow` through normal `ToolRegistry` execution. |
| Callback parser | `n8n/callback.rs` | Verifies signature, schema version, workflow ID, and workflow version. |
| State store | `n8n/state.rs` | Tracks run status, evidence, side effects, terminal state, and dead letters. |
| Governance | `n8n/governance.rs` | Maps run state to verification status and continuation action. |
| Desktop commands | `commands/n8n.rs` | Provides status, discovery, draft import, and reconciliation commands. |
| Local API | `commands/local_api.rs` | Receives callbacks and HITL polling requests. |

## Schemas And Status

Command schema version:

```text
kria.n8n.command.v1
```

Callback schema version:

```text
kria.n8n.callback.v1
```

Executable workflow status:

```text
Approved
```

Non-executable workflow statuses:

```text
Draft
Test
Deprecated
Disabled
```

Terminal run statuses:

```text
Completed
Partial
Failed
Cancelled
TimedOut
Rejected
```

## Governance Rules

| Run state | KRIA verification | KRIA continuation |
|---|---|---|
| Waiting for approval | Human review required | Pause for HITL |
| Non-terminal | Needs more evidence | Await more events |
| Failed, cancelled, timed out, rejected | Failed | Recover workflow |
| Completed or partial with missing expected evidence | Needs more evidence | Pause for HITL |
| Completed or partial with expected evidence present | Verified | Continue workflow |

Important invariant:

```text
Completed n8n status alone is not enough.
The configured KRIA evidence contract must also be satisfied.
```

## Desktop And UI Surface

Dashboard status is backed by `get_n8n_status` and includes:

- enabled state,
- base URL,
- callback URL,
- configured workflows,
- active catalog workflows,
- current runs,
- dead letters,
- recent governance decisions,
- HITL poll responses,
- callback inbox path,
- governance audit path.

Operator commands:

- `discover_n8n_workflows`: read-only `GET /api/v1/workflows` against n8n.
- `import_n8n_workflow`: imports a workflow as `Draft`, not as executable.
- `reconcile_n8n_run`: reads a known n8n execution by `n8n_run_id` and records a governance view.

## Configuration

`N8nConfig` defaults are conservative:

- `enabled = false`
- no default base URL,
- no default API key,
- no default signing secret,
- `request_timeout_secs = 30`
- `max_payload_bytes = 65536`
- `default_requested_by = local-user`
- no configured workflows.

A workflow must be present in config, version matched, and `Approved` before `n8n_invoke_workflow` can execute it.

## Security Invariants

- n8n is treated as external and untrusted for authority purposes.
- Outbound payloads are HMAC signed.
- Inbound callbacks must provide a valid `x-kria-signature`.
- Workflow identity and version are checked on callback ingestion.
- API discovery is read-only.
- Imported workflows are saved as drafts.
- Callback replay is append-only through JSONL inbox records.
- External evidence cannot bypass KRIA verifier/governance authority.
- HITL approval is handled through KRIA's HITL path, not through trusted callback text.

## Failure Handling

| Failure | Behavior |
|---|---|
| Integration disabled | Tool registration is skipped. |
| Empty base URL or signing secret | Catalog construction fails. |
| Unknown workflow | Invocation/callback validation fails. |
| Version mismatch | Invocation/callback validation fails. |
| Workflow not approved | Invocation is rejected. |
| Oversized command payload | Invocation fails before dispatch. |
| HTTP non-2xx | Tool call returns structured failure. |
| Duplicate callback | Dead-lettered, no active state mutation. |
| Out-of-order callback | Dead-lettered, no active state mutation. |
| Post-terminal callback | Dead-lettered, terminal state preserved. |
| Missing expected evidence | Pause for HITL instead of claiming success. |

## Operational Notes

For a working deployment:

1. Enable n8n in KRIA config.
2. Configure `base_url`, `signing_secret`, and optional `api_key`.
3. Add workflows with explicit `workflow_id`, `workflow_version`, endpoint path, allowed actions, data scope, expected evidence, and `Approved` status.
4. Configure n8n callback nodes to POST to KRIA's displayed callback URL.
5. Include the `x-kria-signature` header on callbacks.
6. Use dashboard dead letters and governance logs when diagnosing callback/order issues.

## Current Limits

- Continuation events are emitted, but deep live GoalTree resume requires more orchestration wiring.
- n8n governance audit is JSONL-based; it is not yet a unified global SQLite audit ledger.
- HITL response delivery is polling-based, not push-based webhook resume.
- External SaaS side effects are governed by expected evidence, but there is not yet full verifier federation for every SaaS target.
