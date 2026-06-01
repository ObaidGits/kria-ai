# KRIA n8n Production Integration Plan

Date: 2026-05-31
Status: implementation source of truth for the next n8n production phases
Scope: workflow registration, runtime profiles, trigger-specific execution adapters,
output extraction, recovery, dashboard UX, and prompt routing

## 1. Executive Summary

KRIA's n8n integration must support real user workflows without requiring every
workflow to be rewritten with KRIA-specific callback, Code, or HTTP Request
nodes.

The production direction is:

```text
User connects n8n
-> KRIA discovers workflows
-> KRIA analyzes workflow JSON
-> KRIA generates safe metadata with heuristics + opt-in LLM enrichment
-> user reviews and approves
-> KRIA saves a runtime profile and executable registry record
-> user asks KRIA to run or inspect a workflow
-> KRIA chooses the correct adapter from saved metadata
-> KRIA runs, monitors, or safely refuses with clear guidance
-> KRIA extracts result output and shows it in chat, dashboard, logs, and history
```

The key principle:

```text
KRIA does not guess execution behavior at runtime.
KRIA stores the workflow run strategy at registration time and follows it at runtime.
```

This keeps execution deterministic, debuggable, and safe.

## 2. Product Goal

Normal users should be able to:

- connect an n8n instance,
- sync workflows,
- select a workflow,
- let KRIA analyze it,
- let KRIA use the configured LLM to suggest metadata,
- review simple fields in layman language,
- approve safe workflows,
- run supported workflows from chat or dashboard,
- see the actual workflow output in KRIA,
- understand why a workflow cannot run yet,
- fix setup issues without editing TOML.

Users should not have to manually write large TOML workflow configs, schemas,
callback code, HMAC code, or custom n8n adapter nodes for common workflow types.

## 3. Non-Goals

Do not build these as part of the immediate production integration:

- semantic/vector workflow search,
- embedding-based routing,
- autonomous workflow selection,
- AI-generated n8n workflows,
- automatic mutation of existing n8n workflow JSON,
- replacing the n8n editor,
- silently running destructive workflows,
- storing OAuth tokens, API keys, HMAC secrets, or credential values in KRIA
  workflow metadata.

LLM usage is allowed for metadata enrichment and prompt-to-structured-input in
later phases, but deterministic safety rules remain authoritative.

## 4. Current Baseline

The current KRIA codebase already has important pieces in place:

| Area | Current state |
| --- | --- |
| Runtime profiles | `~/.kria/n8n/runtime_profiles.json` |
| Executable workflow registry | `~/.kria/n8n/workflow_registry.json` |
| Legacy TOML cleanup | workflow entries migrated away from TOML source-of-truth |
| Heuristic workflow analysis | implemented in `crates/kria-core/src/n8n/runtime_profiles.rs` |
| LLM metadata enrichment | implemented in `crates/kria-core/src/n8n/metadata_enrichment.rs` and desktop command wiring |
| Dashboard onboarding | Add from n8n flow exists in `N8nWorkflowManagementPanel.tsx` |
| Webhook GET/POST + polling | implemented in `run_n8n_workflow_adapter` for non-callback webhook workflows |
| Callback/HMAC workflows | still supported as advanced callback mode |
| Output extraction | implemented for polling executions, with source node and evidence |
| Progress events | `n8n:workflow_progress` plus final `n8n:chat_result` |
| Chat bounded routing | implemented with suggestion/confirmation flow |
| Fleet SSH foundation | implemented under Fleet/Device Control for future remote runner backend |

The main missing production capabilities are:

- Manual Trigger execution without adding webhook nodes.
- Runner backend selection for local, Docker, and remote server n8n.
- Monitor mode for schedule/event/app-trigger workflows.
- Sub-workflow broker mode for workflows the user chooses to expose as callable.
- Strong prompt-to-structured-input mapping.
- More complete dashboard guidance around what can run, what can only be
  monitored, and what needs setup.

## 5. Source Of Truth

KRIA must separate connection settings, discovered workflow analysis, and
approved executable metadata.

```text
config/default.toml and ~/.kria/config.toml
  -> n8n runtime connection settings only
  -> base URL, dashboard URL, mode, env/file secret sources, timeouts

~/.kria/n8n/runtime_profiles.json
  -> discovered/analyzed n8n workflow profile drafts
  -> trigger detection, output hint, risk estimate, credential names, warnings
  -> not necessarily executable

~/.kria/n8n/workflow_registry.json
  -> KRIA-approved or draft executable workflow records
  -> used by chat, dashboard, routing, execution, and history

~/.kria/n8n/run_events.jsonl
  -> durable execution progress events for polling/runner/monitor modes

callback inbox and governance audit paths
  -> durable callback evidence and governance decisions
```

TOML workflow blocks must not become the production source of truth again.

## 6. Core Concepts

### 6.1 Runtime Profile

A runtime profile is a discovered workflow analysis record. It answers:

```text
What is this workflow?
How does it start?
Can KRIA start it?
How should KRIA read its result?
What risk does it have?
Does it need human review?
Which credentials are involved?
What warnings must the user resolve?
```

Important fields:

```text
profile_id
workflow_id
n8n_workflow_id
display_name
n8n_workflow_name
n8n_workflow_hash
n8n_workflow_updated_at
status
trigger_strategy
webhook_method
result_mode
detected_triggers
input_candidates
output_strategy
credential_requirements
credential_status
category
risk_estimate
irreversibility_estimate
data_scope
hitl_detected
hitl_strategy
confidence
warnings
enrichment provenance
```

### 6.2 Executable Workflow Registry

The executable registry is what KRIA uses at runtime. A workflow is not runnable
because it appears in n8n. It is runnable only when the KRIA registry says it is
approved and the selected adapter can safely execute it.

Important fields:

```text
workflow_id
workflow_version
display_name
n8n_workflow_id
trigger_strategy
result_mode
webhook_method
webhook_path
preferred_output_node
output_strategy
n8n_workflow_hash
status
environment
risk_tier
irreversibility_class
requires_callback
hitl_policy
category
description
example_prompts
tags
aliases
credential_requirements
data_scope
expected_evidence
allowed_actions
execution_timeout_secs
```

### 6.3 Adapter

An adapter is the runtime mechanism that KRIA uses to interact with n8n.

KRIA should support these adapters:

| Adapter | Purpose | Status |
| --- | --- | --- |
| Callback Adapter | Existing KRIA signed callback workflows | Supported |
| Webhook Adapter | Webhook GET/POST workflows with execution polling | Supported |
| Runner Adapter | Manual Trigger workflows via local/Docker/SSH CLI access | Supported |
| Sub-workflow Broker Adapter | Callable workflows through a broker/parent workflow | Supported |
| Monitor Adapter | Schedule/event/app-trigger latest runs/results | Supported |
| Form Trigger Adapter | n8n Form Trigger submission plus polling | Supported |
| Chat Trigger Adapter | public n8n Chat Trigger message submission plus polling | Supported |
| Cloud-safe fallback | Honest message when KRIA cannot start a workflow | Supported |

## 7. Architecture Diagram

```text
┌─────────────────────────────────────────────────────────────────┐
│                         KRIA UI / Chat                          │
│  Dashboard -> n8n          Chat prompt          Run History      │
└───────────────┬────────────────────┬────────────────────────────┘
                │                    │
                v                    v
┌────────────────────────┐  ┌───────────────────────────────┐
│ n8n Onboarding Wizard  │  │ Bounded Workflow Prompt Router │
│ Sync -> Analyze -> LLM │  │ exact metadata matching first  │
│ Review -> Save/Approve│  │ confirmation/safety checks     │
└───────────────┬────────┘  └───────────────┬───────────────┘
                │                           │
                v                           v
┌─────────────────────────────────────────────────────────────────┐
│                  KRIA n8n Workflow Registry                     │
│ runtime profile + executable metadata + drift hash + risk/HITL  │
└───────────────────────────────┬─────────────────────────────────┘
                                │
                                v
┌─────────────────────────────────────────────────────────────────┐
│                    Universal n8n Execution Adapter              │
│ callback | webhook_polling | runner | broker | monitor | fallback│
└─────────────┬──────────────┬────────────┬──────────────┬────────┘
              │              │            │              │
              v              v            v              v
        n8n Webhook     n8n CLI/API   Broker flow   n8n Executions API
              │              │            │              │
              └──────────────┴────────────┴──────────────┘
                                │
                                v
┌─────────────────────────────────────────────────────────────────┐
│             Polling, Output Extraction, Governance              │
│ execution lookup -> full execution detail -> redacted evidence  │
└───────────────────────────────┬─────────────────────────────────┘
                                │
                                v
┌─────────────────────────────────────────────────────────────────┐
│       chat_result + workflow_progress + run_events.jsonl        │
└─────────────────────────────────────────────────────────────────┘
```

## 8. Registration Flow

Registration is where KRIA learns how to handle a workflow.

```text
User opens Dashboard -> n8n -> Add from n8n
-> Sync workflows
-> KRIA calls n8n workflow API
-> KRIA fetches workflow JSON/details
-> KRIA heuristic analyzer detects trigger/result/risk/output/HITL
-> User optionally clicks Generate Metadata
-> KRIA redacts workflow summary
-> active configured LLM enriches metadata
-> deterministic safety merge normalizes suggestions
-> user reviews field-by-field metadata
-> KRIA saves executable draft
-> KRIA auto-approves only if safe
-> otherwise KRIA shows blockers and next steps
```

### 8.1 Registration Diagram

```text
┌──────────────┐
│ Sync n8n     │
└──────┬───────┘
       v
┌────────────────────┐
│ Workflow JSON list │
└──────┬─────────────┘
       v
┌────────────────────────┐
│ Heuristic Analyzer     │
│ trigger/risk/HITL/out  │
└──────┬─────────────────┘
       v
┌────────────────────────┐
│ Runtime Profile Draft  │
└──────┬─────────────────┘
       v
┌────────────────────────┐
│ Optional LLM Enrich    │
│ redacted summary only  │
└──────┬─────────────────┘
       v
┌────────────────────────┐
│ User Review            │
│ layman fields only     │
└──────┬─────────────────┘
       v
┌────────────────────────┐
│ Save Registry Draft    │
└──────┬─────────────────┘
       v
┌────────────────────────┐
│ Safe auto-approve?     │
└──────┬─────────┬───────┘
       │ yes     │ no
       v         v
  Approved   Draft + blockers
```

## 9. Runtime Flow

Runtime must be deterministic.

```text
User prompt or dashboard run
-> KRIA resolves workflow from approved registry
-> KRIA validates confirmation, policy, risk, HITL, and input
-> KRIA performs drift check
-> KRIA chooses adapter from saved trigger_strategy/result_mode
-> adapter starts or monitors workflow
-> KRIA polls or receives callback
-> KRIA extracts output
-> KRIA records governance
-> KRIA emits progress and final chat result
```

### 9.1 Adapter Decision Tree

```text
Workflow approved?
  no -> refuse with setup guidance
  yes
    |
    v
Workflow hash drifted?
  yes -> block or require refresh/re-review
  no
    |
    v
requires_callback=true?
  yes -> Callback Adapter
  no
    |
    v
trigger_strategy=webhook && result_mode=poll_execution?
  yes -> Webhook Adapter
  no
    |
    v
trigger_strategy=manual_api_execute && runner configured?
  yes -> Runner Adapter
  no
    |
    v
trigger_strategy=sub_workflow/callable && broker configured?
  yes -> Sub-workflow Broker Adapter
  no
    |
    v
trigger_strategy=scheduled_monitor/event_monitor?
  yes -> Monitor Adapter
  no
    |
    v
Cloud-safe fallback with exact missing setup
```

## 10. Trigger Strategy Matrix

| Trigger strategy | Can KRIA start it? | Adapter | Result mode | Notes |
| --- | --- | --- | --- | --- |
| Webhook Trigger | Yes | Webhook Adapter | Poll execution or callback | GET/POST method comes from n8n JSON or user review |
| Manual Trigger | Yes, if runner access exists | Runner Adapter | Poll execution | Uses local CLI, Docker exec, or Fleet SSH |
| Execute Workflow Trigger / callable sub-workflow | Yes, if broker is configured | Sub-workflow Broker Adapter | Poll execution | Does not require modifying every target workflow |
| Schedule Trigger | Usually no direct start | Monitor Adapter | Monitor only | KRIA shows latest runs/results and can optionally offer runner if safe |
| App/Event Trigger | No direct start | Monitor Adapter | Monitor only | Example: Gmail event, Slack event, GitHub event |
| Form Trigger | Yes | Form Trigger Adapter | Poll execution | Uses `/form/{webhookId}` and multipart form submission |
| Chat Trigger | Yes, if public | Chat Trigger Adapter | Poll execution | Uses `/webhook/{webhookId}/chat` with `chatInput` and `sessionId` |
| Unknown/unsupported | No | Fallback | Unsupported | User gets specific setup guidance |

## 11. Adapter Details

### 11.1 Callback Adapter

Use when:

```text
requires_callback = true
```

Flow:

```text
KRIA signs invocation
-> POST n8n webhook
-> n8n sends signed callback
-> KRIA verifies HMAC, freshness, schema, ordering
-> state machine ingests callback
-> governance validates evidence
-> chat_result emitted
```

Best for:

- realtime status,
- long-running workflows,
- workflows already designed for KRIA,
- high-assurance callback evidence.

Limitations:

- Requires KRIA-specific callback logic in n8n.
- Not plug-and-play for arbitrary templates.

### 11.2 Webhook Adapter

Current production v1 supports this.

Use when:

```text
trigger_strategy = webhook
result_mode = poll_execution
requires_callback = false
webhook_method = GET or POST
webhook_path is known
n8n API key is present
```

Flow:

```text
KRIA builds payload
-> GET query params or POST JSON body
-> calls n8n production webhook
-> returns accepted quickly
-> background task polls n8n executions API
-> matches execution by correlation ID or workflow/start-time fallback
-> fetches execution detail
-> extracts output
-> emits progress + chat result
```

Important rules:

- Do not auto-probe GET/POST because probing can execute workflows.
- GET payload must stay small; oversized query payload fails clearly.
- Always include correlation metadata where possible:

```json
{
  "kria_correlation_id": "...",
  "kria_execution_id": "...",
  "kria_requested_by": "kria-ui"
}
```

### 11.3 Runner Adapter

This is the next major implementation.

Use when:

```text
trigger_strategy = manual_api_execute
result_mode = poll_execution
runner_backend is configured
n8n_workflow_id is known
n8n API key is present for polling
```

Purpose:

Run Manual Trigger workflows without adding a Webhook node.

The runner starts n8n workflow execution using an environment-specific command
or API capability, then KRIA polls n8n executions API and extracts output.

Highlighted runner backend mapping for later server hardening:

```text
local n8n    -> local CLI
Docker n8n   -> docker exec
remote server -> Fleet/SSH runner path
```

This mapping is the production direction for Runner Adapter deployments. Local
and Docker setups can run directly from KRIA. Remote/server n8n should reuse
KRIA Fleet/SSH instead of adding a second SSH system. A dedicated KRIA n8n
Runner Service can be added later for server deployments, but it must preserve
this same ownership model: KRIA decides the approved workflow, the server-side
runner executes only allowlisted commands near n8n, and KRIA polls/extracts the
result.

Target command shape:

```text
n8n execute --id <n8n_workflow_id>
```

For Docker:

```text
docker exec <container_name> n8n execute --id <n8n_workflow_id>
```

For remote SSH:

```text
ssh target -> n8n execute --id <n8n_workflow_id>
ssh target -> docker exec <container_name> n8n execute --id <n8n_workflow_id>
```

Security rules:

- Never allow arbitrary shell from workflow metadata.
- Build commands from a strict allowlist.
- Only allow approved workflow IDs from KRIA registry.
- Use existing Fleet SSH execution rather than a new SSH subsystem.
- Redact stdout/stderr before UI display.
- Poll n8n API for final output instead of trusting CLI stdout alone.

### 11.4 Sub-workflow Broker Adapter

Use when:

```text
workflow is not directly webhook/manual runnable from KRIA
user is willing to expose it as a callable sub-workflow
```

Concept:

Create one stable KRIA broker workflow in n8n. That broker receives KRIA input,
calls a target workflow through n8n's sub-workflow mechanism, and returns output
in a standard way.

High-level flow:

```text
KRIA
-> Broker Webhook
-> Execute Workflow node
-> Target Workflow
-> Broker collects result
-> KRIA polls broker execution or receives broker callback
```

Why this exists:

- Avoid modifying every individual target workflow.
- Keep one reusable KRIA integration point.
- Useful when direct manual runner is unavailable or undesirable.

Important caveat:

- Target workflows may need to be callable by n8n's Execute Workflow mechanism.
- KRIA must verify the exact n8n version behavior before implementation.
- Broker must never accept arbitrary target IDs from untrusted user input.

### 11.5 Monitor Adapter

Use when:

```text
trigger_strategy = scheduled_monitor or event_monitor
```

Purpose:

Some workflows should not be started by KRIA. They run because a schedule,
email event, Slack event, GitHub event, or other external event happened.

Flow:

```text
KRIA does not start the workflow
-> user asks "show latest result"
-> KRIA lists recent executions for workflow
-> KRIA fetches latest successful/failed execution
-> KRIA extracts output or error summary
-> KRIA shows result in dashboard/chat
```

Examples:

- daily Gmail digest,
- invoice watcher,
- new GitHub issue triage,
- Slack mention watcher,
- nightly database report.

### 11.6 Cloud-safe Fallback

Use when:

```text
n8n is cloud/remote and KRIA has no webhook, no broker, no runner, or no API key
```

Behavior:

KRIA should not pretend it can run the workflow.

User-facing message:

```text
KRIA can monitor this workflow's n8n executions, but cannot start it yet.
To run it from KRIA, configure one of:
- webhook access
- KRIA runner backend
- sub-workflow broker access
```

### 11.7 Form And Chat Trigger Adapter

Use when:

```text
trigger_strategy = form_submit
or
trigger_strategy = chat_trigger
```

Form flow:

```text
KRIA prompt/input
-> structured input payload
-> multipart POST /form/{webhookId}
-> n8n execution starts
-> KRIA polls executions API
-> KRIA extracts final output
-> Run History + chat result
```

Chat flow:

```text
KRIA prompt/input
-> chatInput + sessionId payload
-> JSON POST /webhook/{webhookId}/chat
-> n8n execution starts
-> KRIA polls executions API
-> KRIA extracts final output
-> Run History + chat result
```

Important constraints:

- Form Trigger v1 submits regular fields only; file upload support is later.
- Chat Trigger must be publicly available in n8n for production calls.
- Both adapters require n8n API key because output comes from execution polling.

## 12. Environment-Specific Execution

### 12.1 Local n8n On Same Machine

Requirements:

- n8n API reachable from KRIA.
- API key configured for polling.
- n8n CLI available on same host if using Runner Adapter.

Runner command:

```text
n8n execute --id <workflow_id>
```

Failure handling:

| Failure | KRIA response |
| --- | --- |
| `n8n` command not found | Show setup guidance: install CLI or use webhook/broker |
| API key missing | Block output polling with clear error |
| n8n API offline | Show runtime disconnected |
| execution not found | Timed out with lookup diagnostics |

### 12.2 Managed Docker n8n

Requirements:

- container name known from KRIA managed Docker settings,
- Docker accessible from KRIA host,
- n8n API key configured,
- n8n API base URL reachable.

Runner command:

```text
docker exec <container_name> n8n execute --id <workflow_id>
```

Important:

- `docker exec` starts workflow inside the container.
- KRIA still polls n8n API over HTTP for the actual execution detail.
- Container must be healthy before execution.

### 12.3 n8n On Remote Server

Requirements:

- n8n API reachable from KRIA or through configured network route,
- API key configured,
- remote runner target enrolled in Fleet if CLI/Docker runner is needed,
- SSH key and host identity configured.

Runner path:

```text
KRIA
-> Fleet SSH target
-> n8n execute --id <workflow_id>
-> KRIA polls n8n API
```

Remote Docker path:

```text
KRIA
-> Fleet SSH target
-> docker exec <container_name> n8n execute --id <workflow_id>
-> KRIA polls n8n API
```

Security requirements:

- Reuse Fleet/Device Control.
- Do not create a second SSH layer.
- Use leases, target health, timeout, and command result telemetry.
- Strictly allowlist runner commands.

### 12.4 n8n Inside Docker On Remote Server

This is a common production case.

KRIA needs two connections:

```text
1. HTTP API path:
   KRIA -> https://server:5678/api/v1/...

2. Runner path:
   KRIA -> SSH/Fleet -> docker exec n8n-container n8n execute --id ...
```

If HTTP API is not directly reachable, later phases may add a tunnel or relay,
but v1 should require reachable API for polling/output extraction.

### 12.5 n8n Cloud

n8n Cloud usually does not expose local CLI access to KRIA.

Supported modes:

- Webhook Adapter if the workflow has a webhook.
- Monitor Adapter if API execution history is accessible.
- Sub-workflow Broker if user configures a broker workflow.

Unsupported:

- Runner Adapter without an official remote execution API or CLI access.

KRIA must show cloud-safe fallback instead of failing mysteriously.

## 13. Workflow Drift Handling

KRIA must assume n8n workflows can change after registration.

At registration:

```text
KRIA saves n8n_workflow_hash
```

Before execution:

```text
KRIA fetches current workflow JSON
KRIA computes current hash
KRIA compares saved hash vs current hash
```

Decision table:

| Drift type | Behavior |
| --- | --- |
| Hash unchanged | Run normally |
| Display/name-only change | Warn and allow refresh |
| Trigger changed | Block run until re-analysis |
| Webhook method/path changed | Block run until review |
| Output node/shape changed | Block output extraction or ask review |
| Credential set changed | Mark needs review |
| New write/delete/payment node added | Block or require explicit approval |
| Risk increased | Require re-approval |
| HITL node added | Require review |

User-facing message:

```text
This n8n workflow changed after KRIA approved it.
Refresh analysis before running.
```

Green read-only workflows may offer:

```text
Refresh analysis and continue
```

Yellow/red workflows must require explicit user review again.

## 14. Risk Labeling Criteria

KRIA should label risk from actual node behavior, not broad words alone.

```text
Green  = read-only / lookup / search / fetch / monitor
Yellow = reversible external write or user-visible side effect
Red    = destructive, payment, delete, irreversible, or high-impact action
Review = unclear external effect; user must inspect before approval
```

Examples:

| Risk | Criteria | Examples |
| --- | --- | --- |
| Green | HTTP GET/HEAD/OPTIONS, Gmail read/search, database select, read-only API lookup, weather/movie/search APIs | `Fetch Movies`, inbox digest, CRM lookup |
| Green with warning | HTTP POST that appears read-only by endpoint/name such as search/query/lookup/fetch | GraphQL query, search API using POST |
| Yellow | send/create/update/write/publish/upload/draft/invite/post message, or external POST with clear non-destructive side effect | Slack post, Gmail draft, create ticket |
| Needs review | external HTTP POST/PUT/PATCH whose purpose is not clearly read-only or clearly write-like | generic HTTP call, opaque API action |
| Red | DELETE, payment, charge, refund, drop/truncate, destructive file/database/admin operations | delete rows, charge card, purge bucket |

Important implementation rules:

- Trigger nodes do not increase risk by themselves. A POST Webhook/Form trigger
  is not a write action; downstream nodes decide risk.
- `postgres` must not become Yellow merely because it contains the word `post`.
  The operation matters: select/read is Green, insert/update is Yellow, delete is
  Red.
- Unknown external writes should not be auto-approved as Green.
- LLM metadata may explain risk, but deterministic analysis owns the final
  minimum risk label.

## 15. Output Extraction

KRIA should not require a user to choose output node by default.

Extraction priority:

```text
1. preferred_output_node from registry
2. response-like node
3. final successful non-empty node
4. fields named result/output/response/data/items
5. compact execution summary fallback
```

Normalized evidence:

```json
{
  "result": "Human-readable summary",
  "output": {},
  "output_source": "HTTP Request",
  "n8n_execution_id": "429",
  "phase": "output_extracted",
  "source": "polling",
  "occurred_at_ms": 1780224079573
}
```

UI rules:

- Chat shows the summary and key fields.
- Run History shows output source and structured preview.
- Full output appears only inside collapsed technical details.
- Secret-like keys are redacted.
- Large arrays are summarized and paginated/collapsed.

## 16. Prompt To Structured Input

Current state:

- KRIA can pass basic input payload and source prompt.
- Full production prompt-to-JSON extraction is a later phase.

Target:

```text
User prompt
+ approved workflow input schema
+ example prompts
+ metadata
+ safety policy
-> structured JSON payload
```

Example:

```text
User:
run get_movies workflow and get action movies

KRIA payload:
{
  "genre": "action",
  "limit": 10,
  "source_prompt": "run get_movies workflow and get action movies"
}
```

Rules:

- LLM may extract structured input.
- JSON must validate against the workflow input schema.
- Missing required fields should ask the user a short clarification.
- Never invent credentials or sensitive values.
- Destructive workflows require confirmation after extracted input is shown.
- Payload preview must be shown for Yellow/Red workflows before execution.

## 17. Prompt Routing: When To Use n8n

KRIA should not route every automation-like prompt to n8n.

Routing layers:

```text
1. Manual Tool Mode
   If user selected n8n, prefer n8n workflow matching but still enforce safety.

2. Explicit n8n intent
   "run n8n workflow", "use workflow", exact workflow ID/name/alias.

3. Approved workflow metadata match
   Match workflow_id, display name, aliases, tags, category, examples.

4. Ambiguity handling
   Show top candidates and ask confirmation.

5. Tool family comparison
   If prompt clearly asks browser/file/GitHub/MCP/OpenClaw, do not force n8n.
```

Important:

- n8n routing should be bounded and metadata-based first.
- No auto-run on ambiguous prompt.
- If user says "list workflows", KRIA should call n8n/dashboard registry status,
  not hallucinate that it lacks access.
- If user selects n8n manually, KRIA should bypass broad tool selection but not
  bypass workflow safety, confirmation, drift, or HITL.

### 16.1 Prompt Routing Diagram

```text
User prompt
  |
  v
Manual tool selected?
  | yes: n8n
  v
n8n candidate matching
  |
  +-- exact single safe workflow -> suggest or run depending policy
  +-- multiple candidates -> ask user to choose
  +-- no candidate -> show available n8n workflows / setup hint
  |
  v
Policy and safety
  |
  +-- unapproved -> do not run
  +-- drifted -> refresh analysis
  +-- yellow/red -> confirm/review
  +-- unsupported adapter -> setup guidance
  |
  v
Adapter execution
```

## 18. HITL Ownership

HITL must be handled by KRIA and n8n together.

Cases:

| HITL type | Owner | KRIA behavior |
| --- | --- | --- |
| KRIA pre-run review | KRIA | Show payload/risk, require user confirmation before start |
| n8n Wait/Approval node | n8n + KRIA | Detect waiting state, show resume link/status, monitor execution |
| External approval link | external system | Show link/status, monitor completion |
| Destructive operation | KRIA | Require explicit confirmation and possibly typed confirmation |

KRIA should never mark a HITL workflow verified just because n8n execution
completed. Governance must check whether required user approval evidence exists.

## 19. Security Model

### 18.1 Secrets

KRIA must not store:

- n8n API key literals in workflow registry,
- HMAC signing secrets in workflow metadata,
- OAuth tokens,
- credential values,
- raw Authorization headers,
- raw URLs with sensitive query strings,
- raw LLM prompts/responses containing workflow internals.

Allowed:

- credential kind names, such as `gmailOAuth2`, `slackOAuth2`,
- env/file/keyring references,
- redacted summaries,
- n8n workflow IDs and node names.

### 18.2 LLM Redaction

LLM metadata enrichment receives only:

- node names,
- node types,
- operation names,
- trigger hints,
- output hints,
- credential kind names,
- safe structural summary.

It must not receive:

- credential values,
- secret headers,
- full raw node JSON,
- file contents,
- raw payloads,
- full URL query strings.

### 18.3 Adapter Safety

Adapters must:

- run only approved workflows,
- reject drifted workflows,
- use strict allowlists,
- log correlation IDs,
- redact outputs,
- enforce timeouts,
- persist progress,
- surface actionable errors.

Runner Adapter must never execute arbitrary shell created by LLM or workflow
metadata.

## 20. Observability And Debugging

Every run should be traceable from prompt to final output.

Required identifiers:

```text
execution_id
correlation_id
workflow_id
n8n_workflow_id
n8n_execution_id
adapter
phase
duration
outcome
```

Required log phases:

```text
[N8N][id] Prompt Received
[N8N][id] Workflow Matched
[N8N][id] Drift Check
[N8N][id] Adapter Selected
[N8N][id] n8n Invocation Started
[N8N][id] Execution Lookup Started
[N8N][id] Execution Found
[N8N][id] Poll Status
[N8N][id] Output Extracted
[N8N][id] Governance
[N8N][id] Response Delivery
```

Dashboard Run History should show:

- workflow name,
- adapter,
- current phase,
- n8n execution ID,
- output source,
- result preview,
- elapsed time,
- blocker if failed,
- next action.

## 21. Recovery Behavior

| Problem | Detection | Recovery |
| --- | --- | --- |
| n8n offline | health check/API failure | Show reconnect/start guidance |
| API key missing | secret source check | Block polling and show exact setting |
| webhook inactive | 404 from `/webhook/...` | Tell user to activate workflow or use test URL only for testing |
| wrong webhook method | 405/404 or metadata mismatch | Refresh analysis and ask method review |
| execution not found | polling timeout | Show lookup details and n8n execution page hint |
| output empty | extractor fallback | Ask user to choose output node or show summary |
| workflow drift | hash mismatch | Refresh/re-review before run |
| credential missing | n8n error/API metadata | Show credential setup guidance |
| HITL waiting | execution state/node detection | Show waiting status/resume link if available |
| runner command failed | exit code/stderr | Show sanitized command result and setup checklist |
| remote SSH target unhealthy | Fleet state | Show target health and reconnect guidance |
| destructive workflow | risk estimate/policy | Require explicit review or block |

## 22. Frontend Implementation Plan

The n8n dashboard should be layman-first.

### 21.1 Dashboard Tabs

Normal dashboard:

```text
Ready to Run
Add from n8n
Run History
```

Developer/debug panels should be hidden behind recovery controls or dev mode,
not shown as the normal user flow.

### 21.2 Add From n8n Wizard

Stepper:

```text
1. Connect
2. Sync
3. Analyze
4. Generate Details
5. Review
6. Save
7. Approve or Test Later
```

Each step must show:

- what KRIA is doing,
- why it matters,
- whether it succeeded,
- what the user should do next.

Example messages:

```text
Reading workflows from n8n...
KRIA found 8 workflows.
KRIA detected this workflow starts from a Manual Trigger.
This workflow needs Runner setup before KRIA can run it.
Metadata suggestions are ready. Review the plain-language fields below.
Saved as draft. Approve is blocked because runner backend is not configured.
```

### 21.3 Workflow Card States

Cards should use layman labels:

```text
Ready to run
Needs setup
Changed in n8n
Can only monitor
Needs credentials
Needs review
Unsupported
```

Avoid exposing raw internal terms by default:

- `trigger_strategy`,
- `result_mode`,
- `workflow_hash`,
- raw JSON,
- schema file paths.

Show technical fields only in collapsed details.

### 21.4 Run History

Run History should show the real result first.

Default view:

```text
Fetch Movies
Completed in 2s
Output from: HTTP Request

Guardians of the Galaxy: Vol. 2 (2017)
Genre: Action, Adventure, Comedy
Rating: 7.6
Plot: ...
```

Technical details collapsed:

```text
correlation_id
n8n_execution_id
raw evidence preview
governance decision
polling events
```

### 21.5 Settings

Settings should be grouped:

```text
n8n Connection
  base URL
  dashboard URL
  API key source
  test connection

Run Methods
  webhook: available
  runner: local/docker/remote target
  monitor: available if API key present
  broker: not configured

Security
  signing secret source
  callback URL
  output redaction
```

## 23. Backend Implementation Plan

### Phase 1: Runner Adapter v1

Goal:

Manual Trigger workflows run from KRIA without adding webhook nodes.

Tasks:

1. Add runner backend fields to runtime profile and workflow registry:

```text
runner_backend = local_cli | managed_docker | remote_ssh | remote_docker | none
runner_target_id
runner_container_name
runner_command_mode
runner_available
```

2. Add deterministic runner capability detection:

```text
managed_docker -> Docker container health + n8n CLI present
local_cli -> `n8n --version` check
remote_ssh -> Fleet target ready
remote_docker -> Fleet target ready + docker container configured
```

3. Add `N8nRunnerAdapter`:

```text
run_manual_workflow(workflow, input_payload, correlation_id)
```

4. Strict command builder:

```text
local: n8n execute --id <id>
docker: docker exec <container> n8n execute --id <id>
ssh: fleet.run_shell_command(allowlisted_command)
```

5. Return accepted quickly and poll executions API.

6. Persist runner events in `run_events.jsonl`.

7. Emit `n8n:workflow_progress`.

8. Update governance and chat result with extracted output.

9. Add UI setup for runner backend.

10. Add tests for local, Docker, remote SSH command building and failure modes.

### Phase 2: Cloud-safe Fallback

Goal:

Unsupported workflows produce helpful setup guidance.

Tasks:

1. Add adapter capability report:

```text
can_start
can_monitor
missing_requirements
recommended_setup
```

2. Show fallback in Add from n8n and Run History.

3. Prevent unsupported workflows from being approved as runnable.

4. Allow monitor-only approval when appropriate.

### Phase 3: Monitor Adapter

Goal:

Schedule/event/app-trigger workflows show latest executions/results.

Tasks:

1. Add `monitor_workflow_latest_execution`.
2. List executions by workflow ID.
3. Fetch latest successful/failed execution.
4. Extract output with same extractor.
5. Show "Latest result" in dashboard and chat.
6. Do not present monitor-only workflows as manually runnable.

### Phase 4: Sub-workflow Broker Adapter

Goal:

Support callable workflows through a reusable broker.

Tasks:

1. Define broker workflow contract.
2. Add broker setup detection.
3. Register allowed target workflow IDs.
4. Call broker webhook with target ID and input payload.
5. Poll broker execution output.
6. Do not mutate target workflows automatically.
7. Add security checks against arbitrary target execution.

### Phase 4.5: Form And Chat Trigger Adapter

Goal:

Support user-facing n8n form/chat workflows without adding KRIA callback nodes.

Tasks:

1. Detect Form Trigger and Chat Trigger endpoints from `webhookId`.
2. Register Form workflows as `trigger_strategy=form_submit`.
3. Register public Chat workflows as `trigger_strategy=chat_trigger`.
4. Submit forms using multipart `POST /form/{webhookId}`.
5. Submit chat messages using JSON `POST /webhook/{webhookId}/chat`.
6. Poll n8n execution output using the same output extractor.
7. Show Form/Chat run method, path, progress, and result in dashboard history.

### Phase 5: Prompt-to-Structured-Input

Goal:

Make workflows useful from natural prompts.

Tasks:

1. Generate input schema from workflow/profile where possible.
2. Allow user to review required fields.
3. Use active LLM to extract JSON from prompt.
4. Validate against schema.
5. Ask clarification for missing required values.
6. Show payload preview for Yellow/Red workflows.
7. Persist successful input mappings.

### Phase 6: HITL Integration

Goal:

Make human review states explicit and recoverable.

Tasks:

1. Detect n8n Wait/approval nodes.
2. Poll waiting executions.
3. Surface approval/resume link when available.
4. Require KRIA-side pre-run confirmation for risky workflows.
5. Governance must verify required HITL evidence.

## 24. Backend Code Ownership

Expected areas:

| Area | Files |
| --- | --- |
| Runtime profile types/analyzer | `crates/kria-core/src/n8n/runtime_profiles.rs` |
| Workflow registry | `crates/kria-core/src/n8n/workflow_registry.rs` |
| Workflow contracts | `crates/kria-core/src/n8n/types.rs` |
| Matching/routing | `crates/kria-core/src/n8n/matching.rs` |
| Governance | `crates/kria-core/src/n8n/governance.rs` |
| Desktop n8n commands/adapters | `crates/kria-desktop/src/commands/n8n.rs` |
| Local API chat route | `crates/kria-desktop/src/commands/local_api.rs` |
| Runtime startup/catalog | `crates/kria-desktop/src/commands/runtime.rs` |
| Fleet SSH reuse | `crates/kria-desktop/src/device_control.rs` and `crates/kria-connection-control/src/manager.rs` |
| Frontend n8n store | `ui/src/stores/n8n.ts` |
| Dashboard UI | `ui/src/components/N8nWorkflowHub.tsx`, `N8nWorkflowManagementPanel.tsx`, `N8nRunTimeline.tsx` |

## 25. Approval Rules

A workflow can be approved as runnable only when:

- metadata is complete,
- workflow hash is current,
- trigger strategy is supported by an available adapter,
- result mode is supported,
- credentials are known enough for the selected adapter,
- risk/HITL policy is valid,
- schema references exist or schema validation is intentionally skipped with a
  safe reason,
- output strategy is usable,
- no unresolved destructive warning exists.

Safe auto-approval may happen only when:

```text
risk = green
irreversibility = read_only
hitl = none
credential_status = present or not_required
trigger/result adapter is available
workflow hash is current
metadata validates
no unresolved warnings
```

Otherwise:

```text
save as draft
show blockers
show next action
```

## 26. Testing Strategy

### 25.1 Unit Tests

Add tests for:

- trigger detection,
- runner backend command building,
- adapter selection,
- hash drift,
- output extraction,
- redaction,
- prompt-to-input validation,
- monitor execution selection,
- fallback messages,
- approval blockers.

### 25.2 Desktop Command Tests

Add tests for:

- `invoke_n8n_workflow_from_ui` uses shared adapter,
- local API chat uses shared adapter,
- webhook workflow uses Webhook Adapter,
- manual workflow uses Runner Adapter when configured,
- manual workflow falls back clearly when runner missing,
- schedule workflow is monitor-only,
- drift blocks execution,
- API key missing blocks polling,
- remote SSH runner uses Fleet path only.

### 25.3 Frontend Tests

Add tests for:

- Add from n8n wizard stepper,
- workflow setup status labels,
- runner setup UI,
- monitor-only UI,
- drift warning,
- run method explanation,
- output preview,
- technical details collapsed,
- no dense technical fields for normal users.

### 25.4 Live Smoke Tests

Required smoke workflows:

```text
1. Webhook GET workflow
2. Webhook POST workflow
3. Manual Trigger local CLI workflow
4. Manual Trigger managed Docker workflow
5. Manual Trigger remote SSH workflow
6. Scheduled workflow monitor
7. Event workflow monitor
8. Fallback for n8n Cloud/no runner
```

Each smoke must verify:

- dashboard run/setup state,
- logs,
- run events,
- n8n execution ID,
- output extraction,
- chat result,
- run history.

## 27. Production Readiness Gates

Do not mark production-ready until:

- no normal workflow setup requires editing TOML,
- n8n workflows can be synced and analyzed from UI,
- metadata enrichment is opt-in and redacted,
- approved workflows survive restart,
- drift detection works,
- webhook workflows run and show output,
- manual workflows run through runner backends,
- schedule/event workflows are monitorable,
- unsupported workflows show actionable fallback,
- output extraction is accurate and redacted,
- prompt routing does not hallucinate lack of access,
- chat and dashboard share the same adapter,
- logs are correlation-linked,
- tests and smoke scripts pass.

## 28. Recommended Next Implementation Slice

The next feature should be:

```text
KRIA n8n Runner Adapter v1 + Cloud-safe fallback
```

Reason:

- Webhook Adapter already exists.
- Manual Trigger workflows are the biggest remaining gap.
- Runner Adapter avoids forcing users to add webhook nodes.
- Existing Fleet SSH infrastructure can be reused for remote servers.
- Cloud-safe fallback prevents confusing failures for n8n Cloud or inaccessible
  deployments.

Do not implement Monitor or Broker first unless Runner proves blocked.

## 29. Runner Adapter v1 Detailed Acceptance Criteria

Runner Adapter v1 is complete when:

```text
Manual Trigger workflow discovered
-> profile says Manual Trigger
-> Add from n8n explains runner requirement
-> user configures local/Docker/SSH runner
-> workflow can be approved
-> KRIA runs the workflow without adding webhook nodes
-> KRIA polls n8n execution
-> output appears in chat and Run History
-> logs show command backend and n8n execution ID
-> missing runner produces clear setup fallback
```

## 30. Example End-To-End Flows

### 29.1 Webhook Movie Workflow

```text
User: run fetch_movies
KRIA: workflow fetch_movies matched
KRIA: trigger_strategy=webhook
KRIA: POST /webhook/...
KRIA: polls execution 429
KRIA: extracts output from HTTP Request node
KRIA: shows movie title, genre, plot, rating
```

### 29.2 Manual Gmail Workflow

```text
User: get latest unread Gmail
KRIA: workflow gmail_unread_manual matched
KRIA: trigger_strategy=manual_api_execute
KRIA: runner_backend=managed_docker
KRIA: docker exec kria-n8n n8n execute --id <n8n_workflow_id>
KRIA: polls n8n execution
KRIA: extracts output from Gmail node/final node
KRIA: shows unread email summary
```

### 29.3 Scheduled Digest Workflow

```text
User: show my latest daily digest
KRIA: workflow daily_digest matched
KRIA: trigger_strategy=scheduled_monitor
KRIA: does not start workflow
KRIA: fetches latest execution
KRIA: extracts result
KRIA: shows latest digest and execution timestamp
```

### 29.4 Unsupported Cloud Workflow

```text
User: run my manual cloud workflow
KRIA: workflow matched
KRIA: trigger_strategy=manual_api_execute
KRIA: no runner backend available
KRIA: says workflow can be monitored but not started
KRIA: suggests adding webhook, broker, or runner access
```

## 31. Implementation Discipline

For every new n8n capability:

1. Update runtime profile fields.
2. Update workflow registry fields.
3. Update analyzer and approval blockers.
4. Update shared adapter, not separate dashboard/chat paths.
5. Update UI with layman status and next action.
6. Add logs with correlation ID.
7. Add run event persistence.
8. Add unit and UI tests.
9. Add a live smoke script.
10. Update this document if the architecture changes.

The rule is:

```text
One workflow source of truth.
One shared execution adapter path.
One visible user-facing state.
No silent fallbacks.
No runtime guessing.
```
