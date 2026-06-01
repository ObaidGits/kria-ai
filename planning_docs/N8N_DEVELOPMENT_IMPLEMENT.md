# KRIA n8n Development Implementation Plan

Date: 2026-05-29
Status: Phase 0 through Phase 2 implementation verified
Related roadmap: `planning_docs/N8N_DEVELOPMENT.md`
Related audits:
- `planning_docs/n8n_audit.md`
- `planning_docs/n8n_callback_to_ui_trace.md`
- `planning_docs/n8n_capability_evidence_report.md`
- `planning_docs/n8n_reliability_audit.md`
- `planning_docs/n8n_roundtrip_evidence.md`

## 1. Purpose

`N8N_DEVELOPMENT.md` is a good long-term roadmap, but it is not specific enough
to execute safely. This document converts that roadmap into an implementation
plan with concrete phases, file-level work, verification commands, acceptance
criteria, and stop conditions.

Use this document as the execution source of truth for the next n8n work.

The goal is not to build a large intelligent automation system immediately. The
goal is to make KRIA's existing n8n integration stable, observable, native in
the UI, and easy to validate end to end.

## 2. Current Baseline

This baseline is based on the current repository state and the latest n8n
status note captured on 2026-05-29.

### 2.1 Verified Working

The following pieces are already implemented or evidence-backed:

| Area | Status | Evidence |
| --- | --- | --- |
| n8n feature flag | Enabled in `config/default.toml` | `[n8n] enabled = true` |
| Signing secret source | Resolved from config/env/file | `crates/kria-core/src/n8n/config.rs` |
| Secret in default config | Removed from `config/default.toml` | `signing_secret = ""` |
| Outbound invocation | HMAC-signed POST to n8n webhook | `crates/kria-core/src/n8n/client.rs` |
| Retry/backoff | Present in outbound client | 3 attempts in `client.rs` |
| Workflow catalog | Version-pinned allowlist | `catalog.rs` |
| Tool registration | Registered as `n8n_invoke_workflow` | `runtime.rs`, `tool.rs` |
| Deterministic dispatch | Uses `input_payload` | `loop_engine/mod.rs` |
| Callback endpoint | `POST /api/n8n/callback` | `local_api.rs` |
| Callback auth model | Bearer bypass + HMAC verification | `api_auth.rs`, `callback.rs` |
| Body limit | 128 KB Axum body limit | `local_api.rs` |
| Callback state machine | Dedup, ordering, terminal protection | `state.rs` |
| Governance decision | Verifies evidence and continuation action | `governance.rs` |
| Persistence | JSONL callback inbox and governance audit | `local_api.rs`, `runtime.rs` |
| Terminal chat event | Emits `n8n:chat_result` for terminal callbacks | `local_api.rs` |
| Frontend chat listener | Injects terminal n8n result into chat | `ui/src/stores/app.ts` |
| Dashboard status | Shows configured workflows, runs, governance | `N8nDashboard.tsx` |
| Live E2E report | 10 passed / 0 failed | `~/.kria/eval_reports/n8n_live_e2e_20260529_212445.txt` |
| Phase 0 contract report | 4 passed / 0 failed | `~/.kria/eval_reports/n8n_phase0_contract_20260529_202647.txt` |
| Runtime-mode report | 5 passed / 0 failed | `~/.kria/eval_reports/n8n_runtime_modes_20260529_202648.txt` |
| Phase 2 UI report | 5 passed / 0 failed | `~/.kria/eval_reports/n8n_phase2_ui_20260529_212255.txt` |
| Phase 3 progress report | 5 passed / 0 failed | `~/.kria/eval_reports/n8n_phase3_progress_20260529_212255.txt` |
| Phase 4 management report | 5 passed / 0 failed | `~/.kria/eval_reports/n8n_phase4_management_20260529_212255.txt` |
| Phase 5 invocation report | 5 passed / 0 failed | `~/.kria/eval_reports/n8n_phase5_invocation_20260529_212255.txt` |
| Reliability report | 17 passed / 0 failed | `~/.kria/eval_reports/n8n_reliability_20260529_202535.txt` |
| Basic n8n eval report | 10 passed / 0 failed | `~/.kria/eval_reports/n8n_eval_20260529_212445.txt` |
| Capability eval report | 25 passed / 0 failed / 16 skipped | `~/.kria/eval_reports/n8n_capability_20260529_212512.txt` |

### 2.2 Known Caveats

These must be treated as active implementation constraints:

1. Terminal chat injection only happens for terminal callback statuses:
   `completed`, `partial`, `failed`, `cancelled`, `timed_out`, or `rejected`.
   A workflow that only sends `running` will not produce a final chat message.

2. The full KRIA desktop/local API was re-executed on 2026-05-29. If KRIA or
   n8n is restarted, rerun `scripts/run_n8n_live_e2e.sh` before moving to the
   next phase.

3. `config/n8n_test_workflow.json` is a diagnostic workflow, not production
   workflow logic. It now uses the current callback schema and contains no
   literal HMAC secret, but its Code node requires the n8n runtime env flags
   `N8N_BLOCK_ENV_ACCESS_IN_NODE=false` and `NODE_FUNCTION_ALLOW_BUILTIN=crypto`
   so it can read `KRIA_N8N_SIGNING_SECRET` and compute the callback HMAC.

4. `N8nDashboard.tsx` is still mostly an admin/debug surface. It is not yet a
   polished native workflow experience.

5. `N8nWorkflowBrowser.tsx` exists, but it must be validated for integration,
   command compatibility, styling, and end-to-end behavior before treating it
   as a finished user feature.

6. Some older audit findings are already fixed in code. Do not implement an
   audit item blindly without checking current files first.

## 3. Execution Rules

### 3.1 Priorities

Work must proceed in this order:

1. Stabilize current contracts and tests.
2. Make workflow execution visibly reliable in the chat and workflow UI.
3. Add a native workflow browser/management experience.
4. Add minimal progress visibility.
5. Only then start intelligent routing or input extraction.

### 3.2 Executable Scope Boundary

This document has two scopes:

| Scope | Included stages/phases | Status |
| --- | --- | --- |
| `IMPLEMENT_NOW` | Phase 0, Phase 1, Phase 1.5, Phase 2, Phase 3 | Current execution source of truth |
| `IMPLEMENT_NEXT` | Phase 4, Phase 4.5, Phase 5, Phase 6 | Allowed only after `IMPLEMENT_NOW` gates pass |
| `FUTURE_RFC` | Roadmap Stage 7, Stage 8, Stage 9, Stage 10 | Planning only, not executable in near-term work |

Agents must not implement `FUTURE_RFC` work from this document. Those stages are
kept only so the near-term design does not block future evolution.

### 3.3 Non-Goals For The Next Sprint

Do not implement these yet:

- Semantic workflow routing.
- Embedding-based workflow search.
- Autonomous workflow chaining.
- AI-generated n8n workflows.
- Workflow memory or habit learning.
- Multi-workflow orchestration.
- "KRIA learns your behavior" features.
- Full n8n node editor replacement.

These are later-stage roadmap items. Implementing them before the core UX and
verification gates are stable will increase debugging cost and reduce reliability.

### 3.4 Engineering Constraints

- Use existing Rust modules under `crates/kria-core/src/n8n` before adding new
  backend abstractions.
- Use existing Tauri command patterns in `crates/kria-desktop/src/commands`.
- Use SolidJS patterns already present in `ui/src/stores/app.ts` and existing
  components.
- Keep user-facing UI free of raw tool names, raw n8n JSON, and internal
  implementation details.
- Every phase must end with verification commands and an explicit pass/fail
  summary.
- If a callback test fails, trace it hop by hop using
  `planning_docs/n8n_callback_to_ui_trace.md`.

## 4. Architecture Map

This section lists the relevant files and the role they play.

### 4.1 Core Rust n8n Module

| File | Role |
| --- | --- |
| `crates/kria-core/src/n8n/config.rs` | Loads n8n config and resolves signing secret |
| `crates/kria-core/src/n8n/catalog.rs` | Allowlisted workflow catalog, approval and version validation |
| `crates/kria-core/src/n8n/types.rs` | Command/callback/workflow/run data contracts |
| `crates/kria-core/src/n8n/client.rs` | Outbound HMAC-signed HTTP invocation and retry logic |
| `crates/kria-core/src/n8n/callback.rs` | Inbound callback signature and schema verification |
| `crates/kria-core/src/n8n/state.rs` | Run state, deduplication, ordering, timeout, dead letters |
| `crates/kria-core/src/n8n/governance.rs` | Verification and continuation decisions |
| `crates/kria-core/src/n8n/tool.rs` | `n8n_invoke_workflow` ToolRegistry handler |
| `crates/kria-core/src/n8n/mod.rs` | Module exports and unit tests |

### 4.2 Agent Integration

| File | Role |
| --- | --- |
| `crates/kria-core/src/agent/loop_engine/mod.rs` | Deterministic workflow listing, invocation, output/error formatting |
| `crates/kria-core/src/agent/router.rs` | Tool routing hints for n8n workflow prompts |
| `crates/kria-core/src/agent/turn_gate.rs` | Turn gating recognition for tool-like commands |

### 4.3 Desktop API And Runtime

| File | Role |
| --- | --- |
| `crates/kria-desktop/src/commands/runtime.rs` | Registers tool, builds catalog, replays inbox, starts maintenance |
| `crates/kria-desktop/src/commands/local_api.rs` | Local HTTP routes, callback handling, SSE, HITL bridge |
| `crates/kria-desktop/src/commands/api_auth.rs` | Bearer auth and n8n callback exemption |
| `crates/kria-desktop/src/commands/n8n.rs` | Tauri commands for dashboard/discovery/CRUD/executions |
| `crates/kria-desktop/src/commands/app_state.rs` | App state fields for n8n catalog, runs, paths, logs |
| `crates/kria-desktop/src/main.rs` | Tauri command registration |

### 4.4 Frontend

| File | Role |
| --- | --- |
| `ui/src/stores/app.ts` | Chat state and `n8n:chat_result` listener |
| `ui/src/components/N8nDashboard.tsx` | Current admin/status n8n panel |
| `ui/src/components/N8nWorkflowBrowser.tsx` | Candidate workflow card browser |
| `ui/src/components/WorkflowProgress.tsx` | Existing progress UI pattern to consider/reuse |
| `ui/src/styles/global.css` | Shared styling entry point |

### 4.5 Config And Scripts

| File | Role |
| --- | --- |
| `config/default.toml` | n8n enablement and workflow allowlist |
| `config/n8n_test_workflow.json` | Demo n8n workflow export, currently needs cleanup |
| `scripts/run_n8n_evals.sh` | Basic prompt/API evaluation |
| `scripts/run_n8n_full_capability_eval.sh` | Capability matrix evaluation |
| `scripts/run_n8n_reliability_tests.sh` | Production reliability callback suite |

### 4.6 Required Backend Change Inventory

This table is the backend work inventory. If a later implementation PR touches
n8n, it should map to one or more rows here.

| Area | Required change | Primary files | Stage |
| --- | --- | --- | --- |
| Config model | Add explicit n8n runtime mode: managed Docker vs external | `config.rs`, `config/default.toml`, config UI commands | Stage 1 |
| Secret model | Keep HMAC/API secrets out of tracked config; support env/file/credential references | `config.rs`, settings UI, docs | Stage 1 |
| Runtime management | Start/stop/status/open dashboard for KRIA-managed n8n | new `commands/n8n_runtime.rs` or `commands/n8n.rs` | Stage 1 |
| External connection | Save/test external n8n base URL, dashboard URL, API key source, callback URL | `commands/n8n.rs`, settings UI | Stage 1 |
| Health checks | Verify n8n HTTP health, API auth, webhook reachability, callback URL reachability | `commands/n8n.rs`, eval scripts | Stage 1 |
| Invocation | Keep signed, version-pinned workflow invocation through `N8nClient` | `client.rs`, `tool.rs` | Stage 1 |
| Callback processing | Keep HMAC, schema, version, body-size validation | `callback.rs`, `local_api.rs` | Stage 1 |
| State machine | Preserve dedup, ordering, terminal protection, timeout | `state.rs` | Stage 1 |
| Observability | Structured logs for every hop and per-run correlation IDs | `client.rs`, `tool.rs`, `local_api.rs`, `runtime.rs` | Stage 1 |
| Realtime events | Emit backend/Tauri/SSE events for start, accepted, callback, governance, terminal | `local_api.rs`, optional store module | Stage 2/5 |
| Workflow registry CRUD | Import draft, approve, disable, delete, list executions | `commands/n8n.rs`, config persistence | Stage 6 |
| Workflow authoring | Create/update workflows only after JSON/schema/graph/API validation | new validator module, `commands/n8n.rs` | Stage 6/9 |
| Backup/rollback | Snapshot existing n8n workflow before update | new workflow authoring module | Stage 6/9 |
| Eval API | Add local test hooks so prompt workflows can be tested without clicking UI | `local_api.rs`, eval scripts | Stage 1/2 |

### 4.7 Required Frontend Change Inventory

This table is the frontend work inventory. The user-facing goal is that n8n
feels like part of KRIA, not a separate service bolted onto the side.

| Area | Required change | Primary files | Stage |
| --- | --- | --- | --- |
| Settings page | Add n8n section under KRIA settings/configuration | settings components, store | Stage 1 |
| Runtime mode UI | Toggle between KRIA-managed n8n and external n8n | settings components | Stage 1 |
| Connection test UI | Show health, API auth, callback URL, dashboard URL, webhook status | settings components | Stage 1 |
| Dashboard launch | Button to open n8n dashboard URL from KRIA | settings/workflow hub | Stage 1 |
| Workflow hub | Native cards/list/search/filter/run/history surface | `N8nWorkflowHub.tsx` | Stage 2 |
| Shared store | Central n8n store for status/runs/governance/events | `ui/src/stores/n8n.ts` | Stage 2 |
| Progress UI | Triggered, accepted, waiting, terminal, timeout states | workflow hub/chat components | Stage 2/5 |
| Chat integration | Render workflow status/result inside chat, no raw JSON | `app.ts`, chat components | Stage 2 |
| Evidence view | Collapsible evidence/governance/dead-letter details | `N8nEvidenceViewer.tsx` | Stage 2 |
| CRUD UI | Import draft, approve, disable, delete registry workflows | workflow hub/admin panel | Stage 6 |
| Authoring UI | Prompt-to-workflow draft preview with validation report and diff | future authoring components | Stage 9 |
| Eval dashboard | Show latest eval/reliability report status | diagnostics panel | Stage 1/2 |

Native UX is considered successful only when a normal user can:

- configure n8n without editing TOML,
- see whether KRIA uses managed Docker or external n8n,
- run an approved workflow from KRIA,
- see current status and final result in KRIA,
- open n8n dashboard from KRIA only when needed,
- avoid raw n8n JSON in normal mode,
- complete the happy path without reading terminal logs.

### 4.8 Settings And Configuration Contract

The current `[n8n]` config is enough for basic invocation, but not enough for the
experience requested here. The target config must explicitly support two modes:

1. `managed_docker`: KRIA owns the n8n container lifecycle.
2. `external`: the user provides an existing n8n URL/API key and KRIA does not
   manage the process.

Proposed target config shape:

```toml
[n8n]
config_version = 2
enabled = true
mode = "managed_docker" # managed_docker | external
base_url = "http://127.0.0.1:5678"
dashboard_url = "http://127.0.0.1:5678"
api_key = "" # deprecated: migrate literal values to api_key_file or keyring
api_key_env = "KRIA_N8N_API_KEY"
api_key_file = "~/.kria/secrets/n8n_api_key"
api_key_keyring = "kria/n8n/api_key"
signing_secret = "" # deprecated: migrate literal values to signing_secret_file or keyring
signing_secret_env = "KRIA_N8N_SIGNING_SECRET"
signing_secret_file = "~/.kria/secrets/n8n.key"
signing_secret_keyring = "kria/n8n/signing_secret"
callback_base_url = "http://127.0.0.1:3001"
callback_path = "/api/n8n/callback"
request_timeout_secs = 30
max_payload_bytes = 65536
default_requested_by = "local-user"
auto_start = false # first-run wizard must require explicit consent before Docker launch
open_dashboard_on_start = false
open_dashboard_from_settings = true
healthcheck_timeout_secs = 5
healthcheck_interval_secs = 30
execution_poll_interval_secs = 5
event_stream_enabled = true
callback_freshness_window_secs = 300
future_callback_skew_secs = 30

[n8n.managed_docker]
container_name = "kria-n8n"
image = "n8nio/n8n:pin-tested-version-or-digest"
image_digest = "sha256:<pinned-digest>"
bind_host = "127.0.0.1"
host_port = 5678
container_port = 5678
data_dir = "~/.kria/n8n/docker"
network = "kria"
restart_policy = "unless-stopped"
pull_policy = "if_missing"
host_gateway_name = "host.docker.internal"
privileged = false
user = "1000:1000"
volume_mode = "rw"
port_collision_policy = "fail_with_guidance" # fail_with_guidance | choose_free_port
healthcheck_path = "/healthz"
n8n_encryption_key_file = "~/.kria/secrets/n8n_encryption_key"
dashboard_auth_required = true
basic_auth_user_env = "KRIA_N8N_BASIC_AUTH_USER"
basic_auth_password_file = "~/.kria/secrets/n8n_basic_auth_password"

[n8n.external]
base_url = "http://127.0.0.1:5678"
dashboard_url = "http://127.0.0.1:5678"
manage_lifecycle = false
require_connection_test_before_enable = true
```

Implementation notes:

- Do not store real API keys or HMAC secrets in tracked config.
- Literal `api_key` and `signing_secret` config fields are deprecated. They
  exist only for backward compatibility and must be migrated to file/env/keyring.
- The settings UI should show the resolved source, not the secret value.
- Managed Docker must never start on first run without explicit user consent.
- `callback_base_url` must be configurable because Docker callbacks may need
  `http://host.docker.internal:3001` while local external n8n may use
  `http://127.0.0.1:3001`.
- `dashboard_url` is separate from `base_url` because reverse proxy or hosted
  n8n deployments can expose UI and API differently.
- The settings UI must have a "Test connection" action that reports health,
  API auth, callback reachability guidance, and workflow discovery status.

#### Config Migration Plan

`N8nConfig` must support backward-compatible loading from the current config
shape before any new fields are made required.

Migration rules:

1. If `config_version` is missing, treat config as version 1.
2. For version 1, default `mode = "external"` when `base_url` is already set.
3. For version 1, default `auto_start = false`.
4. If a literal `signing_secret` exists, write it to
   `~/.kria/secrets/n8n.key`, chmod `0600`, replace the config value with `""`,
   and record a redacted migration notice.
5. If a literal `api_key` exists, write it to
   `~/.kria/secrets/n8n_api_key`, chmod `0600`, replace the config value with
   `""`, and record a redacted migration notice.
6. Unknown future fields must be ignored or preserved without crashing.
7. Migration must have tests for old `config/default.toml`, missing fields, and
   literal secret migration.

Per-workflow settings should eventually include:

```toml
[[n8n.workflows]]
workflow_id = "test_workflow"
workflow_version = "v1"
display_name = "Test Workflow"
description = "Safe callback test workflow"
owner = "local-user"
endpoint_path = "/webhook/..."
status = "approved"
environment = "dev"
risk_tier = "Green"
irreversibility_class = "read_only"
timeout_class = "interactive"
requires_callback = true
input_schema_ref = "schemas/n8n/test_workflow.input.json"
output_schema_ref = "schemas/n8n/test_workflow.output.json"
expected_evidence = ["result"]
allowed_actions = []
data_scope = []
external_data_transfer = false
credential_requirements = []
hitl_policy = "none" # none | on_sensitive_data | always
tags = ["test", "diagnostic"]
aliases = ["test workflow"]
retry_attempts = 3
retry_backoff_ms = [500, 1500, 3000]
```

These workflow-level settings are what let KRIA execute tasks fluently without
guessing how n8n behaves. A workflow should be approved only when its timeout,
callback requirement, risk tier, schemas, and evidence expectations are clear.

### 4.9 Managed Docker Vs External n8n Runtime

KRIA must support both operating models.

#### Managed Docker Mode

KRIA responsibilities:

- Detect Docker availability.
- Pull the configured n8n image if needed.
- Create/start the n8n container.
- Mount persistent n8n data under KRIA data directory.
- Inject required env vars, including the callback URL and secret references.
- Inject `N8N_BLOCK_ENV_ACCESS_IN_NODE=false` and
  `NODE_FUNCTION_ALLOW_BUILTIN=crypto` when using KRIA's signed diagnostic
  callback workflow.
- Confirm health before enabling workflow run buttons.
- Offer "Open n8n Dashboard" from KRIA.
- Offer stop/restart actions in developer/settings mode.

Managed Docker security requirements:

- Bind n8n to `127.0.0.1` by default, not `0.0.0.0`.
- Use a pinned image version or digest; do not use `latest`.
- Do not run the container as privileged.
- Use a dedicated persistent data directory with owner-only permissions.
- Detect port collision before starting and fail with clear guidance unless the
  user explicitly allows choosing a free port.
- Generate or require `N8N_ENCRYPTION_KEY` before storing credentials.
- Require dashboard authentication before exposing/opening the dashboard.
- Never pass secrets as visible CLI args when env file or Docker secret style
  injection is available.
- Log container status and health, but never secret values.

Dashboard/admin setup requirements:

- First-run managed mode must include an admin/auth setup step.
- Settings must show whether dashboard auth is configured.
- KRIA may open the dashboard URL only after the URL is local/trusted and auth
  status has been checked.
- External mode must clearly say that dashboard authentication is the user's
  responsibility.

Required backend commands:

| Command | Purpose |
| --- | --- |
| `get_n8n_runtime_status` | Return mode, process/container status, health, URLs |
| `start_managed_n8n` | Start or create the managed container |
| `stop_managed_n8n` | Stop the managed container |
| `restart_managed_n8n` | Restart and re-check health |
| `open_n8n_dashboard` | Open configured dashboard URL |
| `test_n8n_connection` | Validate base URL, API auth, callback guidance |
| `save_n8n_settings` | Persist settings and rebuild catalog/client |

Expected user-facing states:

```text
n8n managed by KRIA: Running
n8n managed by KRIA: Starting...
n8n managed by KRIA: Docker unavailable
n8n external: Connected
n8n external: API key missing
n8n external: Cannot reach base URL
```

#### External Mode

KRIA responsibilities:

- Do not start or stop n8n.
- Validate base URL and API key if provided.
- Show the dashboard URL button.
- Show callback setup instructions.
- Still own KRIA workflow registry, governance, callbacks, and chat rendering.

External mode must be first-class, not treated as an error. Some users will
already have an n8n instance and will not want KRIA to manage it.

### 4.10 Logging And Debugging Contract

The integration must make failures obvious from terminal logs, durable logs, and
frontend diagnostics. Every run must be traceable by `correlation_id`.

#### Required Backend Log Targets

| Target | Required fields | Purpose |
| --- | --- | --- |
| `n8n_runtime` | mode, base_url, dashboard_url, container_name, health | Docker/external runtime status |
| `n8n_config` | mode, enabled, secret_source, api_key_source, workflow_count | Config load/reload debugging |
| `n8n_client` | workflow_id, correlation_id, attempt, status_code, retryable | Outbound invocation debugging |
| `n8n_tool` | workflow_id, correlation_id, cancellation, result_class | ToolRegistry debugging |
| `n8n_callback_trace` | correlation_id, event_id, sequence_number, workflow_id, status | Callback hop-by-hop tracing |
| `n8n_governance` | correlation_id, verification_status, continuation_action | Governance reasoning |
| `n8n_persistence` | path, record_type, bytes, success/error | JSONL/audit persistence |
| `n8n_events` | event_type, correlation_id, frontend_emit_status | Tauri/SSE event debugging |
| `n8n_authoring` | draft_id, workflow_id, validation_stage, pass/fail | Workflow create/update safety |
| `n8n_eval` | scenario_id, prompt, expected, actual, pass/fail | Automated eval diagnostics |

#### Required Callback Hop Logs

The callback route must make these hops visible:

```text
HOP-0 auth/hmac started
HOP-0 auth/hmac passed or failed
HOP-1 callback parsed
HOP-2 state machine decision
HOP-3 governance decision
HOP-4 persistence written
HOP-5 terminal gate evaluated
HOP-6 Tauri/SSE event emitted
HOP-7 frontend received event
HOP-8 UI state updated
```

Current logs already include several hops. The implementation should normalize
the hop labels so a terminal run can be followed from terminal output alone.

#### Durable Debug Artifacts

Write or preserve:

- `~/.kria/n8n/callback_inbox.jsonl`
- `~/.kria/n8n/governance_audit.jsonl`
- `~/.kria/n8n/runtime_status.json`
- `~/.kria/n8n/last_connection_test.json`
- `~/.kria/eval_reports/n8n_*.txt`

Do not log secret values. It is acceptable to log `secret_source=file`,
`secret_present=true`, and a short non-reversible fingerprint if needed.

#### Log Redaction And Retention

Default logs must be safe for normal users to share.

Rules:

- Default terminal logs may include IDs, statuses, timings, and error classes.
- Default terminal logs must not include full workflow inputs, evidence bodies,
  credential names, tokens, API keys, HMAC secrets, file contents, or user PII.
- Full payload logging requires an explicit debug toggle and must write to a
  local-only debug file with a short retention period.
- Durable debug files should rotate or cap size; default retention target is
  14 days or 50 MB, whichever is hit first.
- Eval reports should include redacted responses by default and link to full
  debug payloads only when the user opted into debug capture.
- Redaction must preserve enough shape to debug: field names, value types,
  lengths, and hashes are allowed; raw sensitive values are not.

### 4.11 Realtime Automated Evals

The user should not have to test each prompt manually in the UI. Build automated
realtime evals that drive the same API/event path the UI uses.

#### Required Eval Types

| Eval | Purpose | Required output |
| --- | --- | --- |
| Prompt run eval | Send workflow prompts through KRIA chat/local API | Final response, correlation ID, pass/fail |
| Callback eval | Send signed callbacks for accepted/running/completed/failed | Callback response and emitted event check |
| Event stream eval | Subscribe to `/api/n8n/events` and verify event sequence | Ordered event transcript |
| Tauri/UI eval | Headless UI or Playwright run button/status smoke | Screenshot/report |
| Runtime mode eval | Managed Docker and external connection checks | Runtime status report |
| Workflow CRUD eval | Import draft, approve, disable, delete | Config/catalog assertions |
| Authoring validation eval | Feed good/bad workflow JSON drafts | Validation report |
| Full capability eval | Run full matrix with pass/fail/skips | `~/.kria/eval_reports/n8n_*.txt` |

#### Proposed Scripts

Add or extend:

```bash
scripts/run_n8n_phase0_contract.sh
scripts/run_n8n_live_e2e.sh
scripts/run_n8n_runtime_modes.sh
scripts/run_n8n_phase2_ui.sh
scripts/run_n8n_workflow_authoring_validation.sh
scripts/run_n8n_ui_smoke.sh
scripts/run_n8n_full_capability_eval.sh
scripts/run_n8n_reliability_tests.sh
```

These scripts are `to-create` unless they already exist in the repository. A
phase gate may not depend on a script until the script exists, is executable,
and writes the required report.

Each eval report must include:

- timestamp,
- KRIA commit hash if available,
- n8n mode,
- n8n base/dashboard URL,
- KRIA local API URL,
- prompt/scenario ID,
- expected event sequence,
- actual event sequence,
- final user-visible response,
- pass/fail,
- log file pointers.

Required JSON report shape for every new eval script:

```json
{
  "schema_version": "kria.n8n.eval_report.v1",
  "suite": "n8n_realtime_e2e",
  "started_at_unix_ms": 1780000000000,
  "finished_at_unix_ms": 1780000001000,
  "kria_commit": "unknown",
  "kria_api_url": "http://127.0.0.1:3001",
  "n8n_mode": "managed_docker",
  "n8n_base_url": "http://127.0.0.1:5678",
  "summary": {
    "passed": 0,
    "failed": 0,
    "skipped": 0
  },
  "scenarios": [
    {
      "id": "run-approved-workflow",
      "status": "passed",
      "prompt": "Run test_workflow",
      "correlation_id": "019...",
      "expected_events": ["workflow_invocation_started", "chat_result"],
      "actual_events": ["workflow_invocation_started", "chat_result"],
      "expected_response_contains": "completed",
      "actual_response": "Workflow completed",
      "logs": ["~/.kria/eval_reports/..."]
    }
  ]
}
```

#### Expected Event Sequence For A Successful Prompt

```text
prompt_sent
workflow_invocation_started
workflow_invocation_accepted
workflow_waiting_for_callback
callback_received_running_or_completed
governance_verified
chat_result_emitted
chat_result_visible
```

The eval should fail if a terminal callback is accepted but no terminal UI/chat
event is observed.

### 4.12 Backend-To-Frontend Streaming Contract

The backend must provide enough events for a responsive frontend. This does not
require full n8n node-level streaming on day one.

#### Required Event Types

| Event | Channel | When |
| --- | --- | --- |
| `n8n:runtime_status` | Tauri + optional SSE | n8n starts/stops/health changes |
| `n8n:workflow_invocation_started` | Tauri + optional SSE | KRIA begins outbound invocation |
| `n8n:workflow_invocation_accepted` | Tauri + optional SSE | n8n webhook returns success |
| `n8n:workflow_invocation_failed` | Tauri + optional SSE | outbound invocation fails |
| `n8n:callback` | Tauri + SSE | any accepted or rejected callback |
| `n8n:governance` | Tauri + SSE | governance decision produced |
| `n8n:chat_result` | Tauri | terminal user-visible result |
| `n8n:workflow_timeout` | Tauri + optional SSE | KRIA marks run timed out |

#### Event Payload Contract

Every run event must include:

```json
{
  "event_type": "n8n:callback",
  "workflow_id": "test_workflow",
  "workflow_version": "v1",
  "correlation_id": "019...",
  "status": "completed",
  "sequence_number": 1,
  "timestamp_ms": 1780000000000,
  "user_visible_summary": "Workflow completed",
  "debug": {
    "n8n_run_id": "exec-123",
    "event_id": "evt-123"
  }
}
```

Frontend state must key runs by `correlation_id`, not only workflow ID. Multiple
runs of the same workflow can be active.

### 4.13 Workflow Create/Update Validation Pipeline

KRIA may eventually create or update n8n workflows from prompts, but only behind
a validation and rollback pipeline. This applies to Stage 6 CRUD and Stage 9 AI
workflow generation.

#### Required Pipeline

```text
prompt or edit request
-> draft workflow JSON
-> static JSON parse
-> n8n workflow schema validation
-> graph integrity validation
-> KRIA callback contract validation
-> secret/reference validation
-> dry-run/import validation against n8n test instance
-> diff against existing workflow
-> backup existing workflow
-> save as draft
-> user confirmation
-> activate/test
-> approve in KRIA registry
```

#### Static Validation Rules

The validator must reject workflow JSON when:

- JSON is invalid.
- Required top-level fields are missing.
- Node IDs are duplicated.
- Connections point to missing nodes.
- Webhook node is missing for KRIA-invoked workflows.
- Callback HTTP Request node is missing for async workflows.
- Callback body does not include `correlation_id`, `event_id`,
  `sequence_number`, `workflow_id`, `workflow_version`, `n8n_run_id`, `status`,
  and `occurred_at_ms`.
- Callback signature is computed from a different body than the one sent.
- A real secret literal appears in JSON.
- The workflow would overwrite an existing workflow without a backup.
- The workflow uses disabled or disallowed node types for the current risk tier.

#### Dynamic Validation Rules

Before updating an existing workflow:

- Export and store the current n8n workflow JSON as a backup.
- Import/update the generated workflow as inactive or draft if supported by the
  installed n8n version.
- Trigger a test execution with a safe payload.
- Require the terminal callback to pass KRIA verification.
- Require governance to produce `verified` or a documented expected result.
- Only then allow activation/approval.

#### n8n Version Compatibility

Workflow validation must be version-aware.

Rules:

- Managed Docker mode must pin the supported n8n image version or digest.
- External mode must query and record the installed n8n version before workflow
  create/update operations.
- A workflow JSON draft must declare the n8n version it was generated for.
- Validation must reject drafts generated for unsupported major versions unless
  the user explicitly runs a compatibility check.
- Before update, KRIA must perform a dry import/export roundtrip against the
  actual target n8n instance and compare the normalized result.
- If n8n modifies or drops fields during roundtrip, the update requires manual
  review before activation.

#### Authoring UI Requirements

The user must see:

- Draft workflow name and purpose.
- Risk tier and irreversibility class.
- Nodes/actions summary.
- Required external credentials.
- Validation pass/fail list.
- Diff from existing workflow if updating.
- Backup identifier.
- Test execution result.
- Approve/deny controls.

No prompt-generated workflow should silently overwrite an existing n8n workflow.

## 5. Implementation Phases

The roadmap stages are too broad for direct execution. Use the phases below.

### 5.0 Roadmap Stage Checkpoints

The implementation must still follow the stages from `N8N_DEVELOPMENT.md`.
Each stage below has a checkpoint gate. Do not move to the next stage until the
current stage has implementation, automated tests, manual smoke, expected
responses, and a pass/fail report.

#### Roadmap-To-Phase Crosswalk

| Roadmap stage | Implementation phases | Required gate |
| --- | --- | --- |
| Stage 1: Basic Stable Integration | Phase 0, Phase 1, Phase 1.5 | Contract cleanup, live callback proof, settings/runtime mode proof |
| Stage 2: Native KRIA Experience | Phase 2, Phase 3 | Workflow hub, shared store, visible progress, realtime E2E |
| Stage 3: Intelligent Workflow Routing | Phase 5, Phase 6 | Deterministic matching first, no auto-run on ambiguity |
| Stage 4: AI Input Extraction | Future extension after Phase 6 | Input schemas, confirmation, missing-input evals |
| Stage 5: Realtime Streaming Experience | Phase 3 plus future node-streaming extension | Event ordering, timeout handling, UI update without refresh |
| Stage 6: Workflow CRUD Layer | Phase 4, Phase 4.5 | Safe registry CRUD, workflow validation, backup/rollback |
| Stage 7-10 | `FUTURE_RFC` only | New RFC and explicit approval required |

#### Stage 1: Basic Stable Integration

Scope:

- Config model.
- Managed/external runtime selection.
- Signed workflow invocation.
- Signed callback verification.
- Reliability tests.
- Connection settings and diagnostics.

Automated tests:

- `cargo check -p kria-core`
- `cargo check -p kria-desktop`
- `cd ui && npm run check`
- `./scripts/run_n8n_reliability_tests.sh`
- `./scripts/run_n8n_runtime_modes.sh`

Manual test:

- Start KRIA.
- Use settings to select managed Docker or external n8n.
- Test connection.
- Run `test_workflow`.
- Confirm terminal callback appears in chat.

Expected user response:

```text
Workflow "Test Workflow" triggered. Waiting for n8n callback.
Workflow "Test Workflow" completed: <result>
```

Gate:

- No raw n8n JSON in normal response.
- Reliability suite passes.
- Runtime settings are visible and understandable.

#### Stage 2: Native KRIA Experience

Scope:

- Workflow hub/cards.
- Search/filter.
- Recent runs/history.
- Evidence viewer.
- Status/progress in chat and workflow hub.
- n8n dashboard open button.

Automated tests:

- UI typecheck.
- Store tests for run state updates.
- Realtime E2E script verifies event sequence.
- UI smoke if Playwright is available.

Manual test:

- Run workflow from workflow card.
- Run workflow from chat.
- Open n8n dashboard from KRIA.
- Confirm both surfaces show consistent run state.

Expected user response:

```text
Workflow "Name" is running.
Waiting for n8n callback.
Workflow "Name" completed successfully.
```

Gate:

- User can operate common n8n tasks without opening raw config.
- Opening n8n dashboard is optional, not required for basic use.

#### Stage 3: Intelligent Workflow Routing

Scope:

- Deterministic metadata-based matching first.
- Later semantic ranking only after Stage 2 is stable.
- Confirmation before running ambiguous matches.

Automated tests:

- Prompt matching evals.
- Ambiguity evals.
- Unknown workflow evals.

Manual test:

- Ask for a workflow by display name/alias.
- Ask an ambiguous prompt.
- Confirm KRIA asks before running.

Expected user response:

```text
I found 2 matching workflows. Choose one before I run it.
```

Gate:

- No vague prompt may auto-run a workflow without confirmation.

#### Stage 4: AI Input Extraction

Scope:

- Extract workflow parameters from user text.
- Ask for missing required inputs.
- Handle attachments only after safe contracts exist.

Automated tests:

- Parameter extraction evals with expected JSON.
- Missing input evals.
- Payload size and schema tests.

Manual test:

- Prompt with complete inputs.
- Prompt with missing inputs.
- Confirm KRIA asks targeted clarification.

Expected user response:

```text
I need the recipient email before running this workflow.
```

Gate:

- Parameter extraction must be visible and confirmable for non-read-only workflows.

#### Stage 5: Realtime Streaming Experience

Scope:

- Backend-to-frontend event contract.
- Workflow progress UI.
- Optional n8n node/log streaming when available.

Automated tests:

- Event stream ordering test.
- Callback-to-UI event test.
- Timeout test.

Manual test:

- Run a workflow that emits `running` then `completed`.
- Confirm UI updates without refresh.

Expected user response:

```text
Triggered -> Waiting for callback -> Completed
```

Gate:

- User never sees a permanently stuck "Running..." state without timeout or diagnostic path.

#### Stage 6: Workflow CRUD Layer

Scope:

- Import/export.
- Approve/disable/delete.
- Create/update draft workflow with validation.
- Backup before update.

Automated tests:

- CRUD command tests.
- Workflow JSON validation tests.
- Backup/rollback tests.

Manual test:

- Import a workflow as draft.
- Approve it.
- Disable it.
- Delete registry entry.
- Attempt bad JSON update and confirm rejection.

Expected user response:

```text
Workflow draft created. Validation passed. Approval required before execution.
```

Gate:

- Bad workflow JSON cannot corrupt existing n8n workflows.

#### Stage 7: Hybrid KRIA + n8n Cognition (`FUTURE_RFC` only)

Scope:

- Local GUI data collection plus n8n cloud/API execution.
- Explicit context handoff.
- User-visible evidence.

Gate:

- Stage 1 through Stage 6 must be stable.
- No hidden local-to-cloud data transfer.
- A separate RFC must define data-scope enforcement, consent, audit, and HITL.

#### Stage 8: Teach KRIA (`FUTURE_RFC` only)

Scope:

- Workflow usage history.
- Recommendations based on explicit, inspectable patterns.

Gate:

- Requires privacy and retention settings.
- Must be opt-in.
- Requires a separate memory/privacy RFC before implementation.

#### Stage 9: AI Workflow Generation (`FUTURE_RFC` only)

Scope:

- Generate workflow drafts from prompts.
- Validate JSON/schema/graph/callback.
- Test before activation.

Gate:

- Workflow authoring validation pipeline must already exist.
- Generated workflows remain draft until user approval.
- Requires a separate authoring RFC and fixture suite before implementation.

#### Stage 10: Full Agentic Orchestration (`FUTURE_RFC` only)

Scope:

- Workflow chaining.
- Adaptive retries.
- Reasoning over outputs.

Gate:

- Requires explicit user policy, audit, rollback, and safety contracts.
- Not part of the near-term implementation.

### Phase 0: Baseline Lock And Contract Cleanup

Goal: make the current integration contracts clean before adding UX.

#### Tasks

| ID | Task | Files |
| --- | --- | --- |
| P0.1 | Confirm `config/default.toml` has empty `signing_secret` | `config/default.toml` |
| P0.2 | Remove hardcoded secret from test workflow export | `config/n8n_test_workflow.json` |
| P0.3 | Update test workflow export to use `input_payload`, not `payload` | `config/n8n_test_workflow.json` |
| P0.4 | Update test workflow callback body to match `N8nCallbackEnvelope` | `config/n8n_test_workflow.json`, `types.rs` |
| P0.5 | Ensure n8n callback signing signs the exact callback JSON body | `config/n8n_test_workflow.json` |
| P0.6 | Document required n8n env/credential secret setup | this doc or a dedicated setup doc |
| P0.7 | Re-run static checks | Rust and UI commands below |
| P0.8 | Add callback timestamp freshness validation and future-skew rejection | `callback.rs`, `types.rs`, tests |
| P0.9 | Add literal secret migration/redaction tests | `config.rs`, config tests |

#### Callback Contract

Every terminal callback from n8n to KRIA must include this shape:

```json
{
  "schema_version": "kria.n8n.callback.v1",
  "correlation_id": "same-correlation-id-from-command",
  "causation_id": "same-correlation-id-or-parent-id",
  "event_id": "unique-event-id",
  "sequence_number": 1,
  "workflow_id": "test_workflow",
  "workflow_version": "v1",
  "n8n_run_id": "n8n-execution-or-generated-id",
  "status": "completed",
  "evidence": {
    "result": "human readable result",
    "occurred_at_ms": 1780000000000
  },
  "side_effects": [],
  "occurred_at_ms": 1780000000000
}
```

Allowed `status` values are defined in `N8nRunStatus`:

- `accepted`
- `running`
- `waiting_for_approval`
- `completed`
- `partial`
- `failed`
- `cancelled`
- `timed_out`
- `rejected`

Only terminal statuses should produce final chat results:

- `completed`
- `partial`
- `failed`
- `cancelled`
- `timed_out`
- `rejected`

#### n8n Workflow Export Requirements

`config/n8n_test_workflow.json` must not contain real secrets. Use one of:

1. n8n credential field for the HMAC secret.
2. n8n environment variable such as `KRIA_N8N_SIGNING_SECRET`.
3. A local-only placeholder string with instructions, never a real secret.

The HTTP Request node must sign the exact byte/string body it sends. If the node
computes a signature from `$json` but sends a different JSON body, KRIA will
reject the callback with `n8n callback signature is invalid`.

For the bundled test workflow, configure the same HMAC secret on both sides:

```bash
mkdir -p ~/.kria/secrets
printf '%s\n' "$KRIA_N8N_SIGNING_SECRET" > ~/.kria/secrets/n8n.key
chmod 600 ~/.kria/secrets/n8n.key
docker run --env KRIA_N8N_SIGNING_SECRET="$KRIA_N8N_SIGNING_SECRET" ...
```

Do not paste the literal secret into `config/default.toml` or the exported n8n
workflow JSON. If an old config contains a literal `signing_secret`, KRIA should
migrate it to `~/.kria/secrets/n8n.key` and leave the config value empty.

#### Acceptance Criteria

Phase 0 is complete only when:

- `rg -n "bdb01293|signing_secret = \"[^\"]+\"" config` finds no real secret.
- `config/n8n_test_workflow.json` contains `correlation_id`, `causation_id`,
  `event_id`, `sequence_number`, `n8n_run_id`, and `occurred_at_ms`.
- The test workflow export reads `input_payload`.
- Callback freshness rejects stale callbacks older than the configured window.
- Callback freshness rejects callbacks too far in the future.
- Literal config secrets are migrated to local secret files or rejected.
- `cargo check -p kria-core` passes.
- `npm run check` passes in `ui/`.

#### Verification Commands

```bash
rg -n "bdb01293|signing_secret = \"[^\"]+\"" config
cargo check -p kria-core
cd ui && npm run check
```

### Phase 1: Live End-To-End Callback Verification

Goal: prove that the live app performs the full loop, not only static checks or
manual callback simulation.

#### Flow To Verify

```text
User chat prompt
-> deterministic dispatch
-> n8n_invoke_workflow
-> signed webhook POST to n8n
-> n8n executes workflow
-> n8n sends signed terminal callback
-> KRIA verifies callback
-> state machine accepts event
-> governance verifies or escalates
-> JSONL inbox and audit are written
-> n8n:chat_result is emitted
-> UI injects assistant message into chat
```

#### Tasks

| ID | Task | Files/Commands |
| --- | --- | --- |
| P1.1 | Start n8n and confirm health | `docker start n8n`, `curl 127.0.0.1:5678/healthz` |
| P1.2 | Start KRIA desktop app/local API | `cargo tauri dev` |
| P1.3 | Confirm local API health | `curl http://127.0.0.1:3001/api/health` |
| P1.4 | Trigger `Run test_workflow` from chat | UI |
| P1.5 | Confirm n8n receives webhook | n8n execution view |
| P1.6 | Confirm callback accepted | KRIA logs and JSONL |
| P1.7 | Confirm chat receives final message | UI and console |
| P1.8 | Run live E2E script | `scripts/run_n8n_live_e2e.sh` |
| P1.9 | Run reliability suite after restart | `scripts/run_n8n_reliability_tests.sh` |

#### Required Logs

Backend logs should show:

- `HOP-1: State machine ingest complete`
- `HOP-2: Governance evaluation complete`
- `HOP-3: Preparing n8n:chat_result event for frontend`
- `HOP-4: n8n:chat_result emitted successfully`

Frontend console should show:

- `[n8n:chat_result] HOP-5`
- `[n8n:chat_result] HOP-6`
- `[n8n:chat_result] HOP-7`

#### Acceptance Criteria

Phase 1 is complete only when:

- `Run test_workflow` triggers n8n.
- n8n sends a signed terminal callback.
- KRIA returns callback response with `decision: accepted`.
- `~/.kria/n8n/callback_inbox.jsonl` receives an entry.
- `~/.kria/n8n/governance_audit.jsonl` receives an entry.
- Chat shows a final workflow result without raw n8n JSON.
- Reliability suite passes 17/17.

#### Automated Live E2E

Use the live script to avoid manually clicking through the UI for every run:

```bash
./scripts/run_n8n_live_e2e.sh
```

The script checks KRIA health, n8n health, the test webhook, chat-triggered
workflow invocation, a new accepted terminal callback in
`~/.kria/n8n/callback_inbox.jsonl`, matching governance audit persistence, and
the `/api/n8n/events` stream.

If n8n is running in Docker, the container must expose the same secret KRIA uses,
for example:

```bash
docker run -e KRIA_N8N_SIGNING_SECRET="$(cat ~/.kria/secrets/n8n.key)" ...
```

The script fails preflight when the local `n8n` container is missing this env
var, because the callback HMAC cannot match KRIA otherwise.

#### Manual Callback Test

Use this only to isolate KRIA callback behavior. It does not replace the real n8n
workflow callback test.

```bash
SECRET=$(cat ~/.kria/secrets/n8n.key)
NOW_MS=$(date +%s%3N)
PAYLOAD='{"schema_version":"kria.n8n.callback.v1","correlation_id":"manual-live-test","causation_id":"manual-live-test","event_id":"evt-manual-live-test-1","sequence_number":1,"workflow_id":"test_workflow","workflow_version":"v1","n8n_run_id":"manual-run-1","status":"completed","evidence":{"result":"Manual terminal callback accepted","occurred_at_ms":'"$NOW_MS"'},"side_effects":[],"occurred_at_ms":'"$NOW_MS"'}'
SIG=$(printf '%s' "$PAYLOAD" | openssl dgst -sha256 -hmac "$SECRET" -binary | xxd -p | tr -d '\n')
curl -sS -X POST http://127.0.0.1:3001/api/n8n/callback \
  -H "Content-Type: application/json" \
  -H "x-kria-signature: sha256=$SIG" \
  -d "$PAYLOAD"
```

### Phase 1.5: n8n Settings And Runtime Management

Goal: make n8n setup manageable from KRIA configuration, with two explicit
runtime modes: KRIA-managed Docker and user-managed external n8n.

#### Backend Tasks

| ID | Task | Files |
| --- | --- | --- |
| P1.5.1 | Extend `N8nConfig` with runtime mode, dashboard URL, secret source fields, runtime health fields | `crates/kria-core/src/n8n/config.rs`, `config/default.toml` |
| P1.5.2 | Add runtime status command | `crates/kria-desktop/src/commands/n8n.rs` or new `n8n_runtime.rs` |
| P1.5.3 | Add start/stop/restart commands for managed Docker mode | runtime command module |
| P1.5.4 | Add external connection test command | runtime command module |
| P1.5.5 | Add open-dashboard command | runtime command module |
| P1.5.6 | Persist settings and rebuild n8n catalog/client without restart where possible | config/runtime code |
| P1.5.7 | Add structured logs under `n8n_runtime` and `n8n_config` | runtime command module |

#### Frontend Tasks

| ID | Task | Files |
| --- | --- | --- |
| P1.5.8 | Add n8n settings section under KRIA settings | settings components |
| P1.5.9 | Add mode selector: managed Docker vs external | settings components |
| P1.5.10 | Add connection test results panel | settings components |
| P1.5.11 | Add "Open n8n Dashboard" button | settings/workflow hub |
| P1.5.12 | Add runtime status badges | settings/workflow hub |
| P1.5.13 | Hide secret values but show source and presence | settings components |

#### Frontend Integration Targets

Use these concrete targets unless the current UI structure has changed:

| UI piece | Target file/path |
| --- | --- |
| Shared n8n settings/store state | `ui/src/stores/n8n.ts` |
| Settings panel section | existing settings component or `ui/src/components/N8nSettings.tsx` |
| Workflow hub entry component | `ui/src/components/N8nWorkflowHub.tsx` |
| Workflow cards | `ui/src/components/N8nWorkflowCard.tsx` |
| Evidence/details viewer | `ui/src/components/N8nEvidenceViewer.tsx` |
| Diagnostics/runtime panel | `ui/src/components/N8nDiagnosticsPanel.tsx` |
| Global styling | `ui/src/styles/global.css` or the existing imported component stylesheet |
| Navigation entry | the existing app navigation/router surface that currently exposes admin pages |

The implementation must choose one navigation entry and document it in the stage
report. Do not scatter n8n settings across unrelated panels.

#### Settings UI Requirements

The settings UI must include:

- Enable/disable n8n integration.
- Runtime mode selector.
- Base URL.
- Dashboard URL.
- API key source: env/file/manual masked.
- HMAC secret source: env/file/manual masked.
- Callback URL preview.
- Managed Docker container name.
- Managed Docker data directory.
- Auto-start toggle.
- Open dashboard toggle/button.
- Test connection button.
- Last connection test result.

#### Acceptance Criteria

Phase 1.5 is complete only when:

- User can configure managed Docker mode from KRIA.
- User can configure external n8n mode from KRIA.
- User can test connection without editing TOML manually.
- User can open n8n dashboard from KRIA.
- Runtime logs clearly show why n8n is not available when it fails.
- Secrets are not displayed or written into tracked files.

#### Verification Commands

```bash
cargo check -p kria-core
cargo check -p kria-desktop
cd ui && npm run check
./scripts/run_n8n_runtime_modes.sh
```

### Mandatory Security Gate Before Phase 2

Do not begin the native workflow hub implementation until this gate passes:

- No literal API key or signing secret remains in tracked config/workflow exports.
- Managed Docker does not auto-start without user consent.
- Managed Docker binds to `127.0.0.1` by default.
- Managed Docker uses a pinned image version/digest.
- Dashboard auth/encryption key status is checked.
- Callback freshness and future-skew rejection are implemented and tested.
- Default logs redact payload/evidence/PII.
- Realtime eval script artifacts are either implemented or explicitly marked
  `to-create` and not used as blocking gates yet.

### Phase 2: Native Workflow Store And UI Hub

Goal: make n8n workflows feel like a native KRIA capability, not an admin panel.

This phase corresponds to Stage 2 from the roadmap, but it must remain
deterministic. Do not add semantic routing or embeddings in this phase.

#### UX Requirements

The workflow UI must support:

- Workflow cards.
- Search by workflow ID/display name/tags.
- Filter by status/risk/environment.
- Run approved workflows.
- Show recent runs.
- Show terminal result.
- Show governance status.
- Show dead-letter count with drilldown.
- Show callback URL and setup health in a compact diagnostics area.

#### Suggested Store

Create a dedicated SolidJS store:

```text
ui/src/stores/n8n.ts
```

Store responsibilities:

- Load status via `get_n8n_status`.
- Keep `configured_workflows`, `runs`, `dead_letters`, `governance_log`.
- Subscribe to `n8n:callback`.
- Subscribe to `n8n:governance`.
- Subscribe to `n8n:chat_result` only if needed for workflow-specific UI.
- Provide `runWorkflow(workflow_id)`.
- Provide `refresh()`.
- Provide derived selectors:
  - `approvedWorkflows`
  - `runningRuns`
  - `terminalRuns`
  - `runsByWorkflowId`
  - `latestRunForWorkflow`
  - `deadLettersByWorkflowId`

Do not keep all workflow state local inside `N8nDashboard.tsx`.

#### Component Plan

| Component | Purpose |
| --- | --- |
| `N8nWorkflowHub.tsx` | Main user-facing workflow surface |
| `N8nWorkflowCard.tsx` | Individual workflow card with status and run button |
| `N8nRunTimeline.tsx` | Recent events for a selected run |
| `N8nEvidenceViewer.tsx` | Expandable evidence and governance details |
| `N8nDiagnosticsPanel.tsx` | Admin/debug details, collapsed by default |

`N8nDashboard.tsx` can remain as an admin/debug view, but the default user
surface should become the workflow hub.

#### Tauri Command Requirements

Confirm these commands are registered in `main.rs` and usable:

- `get_n8n_status`
- `discover_n8n_workflows`
- `import_n8n_workflow`
- `approve_n8n_workflow`
- `disable_n8n_workflow`
- `delete_n8n_workflow`
- `reconcile_n8n_run`
- `list_n8n_executions`

If a frontend component calls a command that does not exist, either register it
or change the component to call an existing command. For example, verify whether
`send_chat_message` is a valid Tauri command before relying on it from
`N8nWorkflowBrowser.tsx`.

#### Run Button Behavior

The run button should not expose `n8n_invoke_workflow` to the user.

Acceptable implementation options:

1. Call the existing chat command with `Run <workflow_id>` if that command is
   already registered and returns a usable result.
2. Add a dedicated Tauri command such as `invoke_n8n_workflow_from_ui`.

Preferred long-term option: add a dedicated Tauri command. It avoids coupling
workflow cards to chat prompt parsing.

Proposed command:

```rust
#[tauri::command]
pub async fn invoke_n8n_workflow_from_ui(
    state: State<'_, AppStateCell>,
    workflow_id: String,
    input_payload: serde_json::Value,
) -> Result<serde_json::Value, String>
```

The command should:

- Resolve app state.
- Validate n8n is enabled.
- Use the existing `N8nClient`.
- Return `workflow_id`, `correlation_id`, `accepted`, and user-safe message.
- Never return the raw n8n webhook acknowledgement as the primary UI result.

#### UI Acceptance Criteria

Phase 2 is complete only when:

- User can see all configured workflows as cards.
- Approved workflows have an enabled run control.
- Draft/disabled/deprecated workflows cannot be run from normal mode.
- User can search/filter workflows.
- Running a workflow shows immediate "triggered/waiting" state.
- Terminal callback updates the workflow card/run timeline.
- Governance result is visible without raw JSON by default.
- Admin/debug JSON is hidden behind an explicit details control.
- UI works on desktop and narrow viewport without overlapping text.

#### Verification Commands

```bash
cd ui && npm run check
cargo check -p kria-desktop
./scripts/run_n8n_phase2_ui.sh
```

If Playwright is available in the repo, add or run a screenshot smoke for:

- Workflow hub desktop viewport.
- Workflow hub mobile/narrow viewport.
- Workflow card run state.
- Evidence details expanded.

### Phase 3: Minimal Progress Visibility

Goal: give users a trustworthy execution state even before full node-level
streaming exists.

This phase pulls forward the smallest useful part of roadmap Stage 5.

#### State Model

Use a simple UI lifecycle:

```text
idle
-> triggering
-> accepted
-> waiting_for_callback
-> completed | failed | partial | timed_out | rejected
```

Do not attempt node-by-node n8n streaming yet unless n8n is explicitly sending
node events.

#### Event Sources

| Source | Meaning |
| --- | --- |
| Immediate invocation result | Workflow was accepted by n8n webhook |
| `n8n:callback` event | State changed in KRIA state machine |
| `n8n:governance` event | Verification/continuation changed |
| `n8n:chat_result` event | Terminal user-visible result |
| `/api/n8n/events` | Optional SSE stream for external/local clients |

#### UI Requirements

For each running workflow show:

- Workflow name.
- Correlation ID, shortened.
- Current status.
- Last evidence timestamp.
- Time since trigger.
- Waiting-for-callback warning if no terminal callback arrives.
- Final result or failure summary.

#### Timeout Behavior

Timeout classes are defined in `N8nTimeoutClass`:

- `interactive`: 60 seconds in current code.
- `background`: 5 minutes.
- `long_running`: 1 hour.

If a run times out, the UI must show a clear timeout state and a recovery hint.

#### Acceptance Criteria

Phase 3 is complete only when:

- A user never sees only "Running..." with no further status.
- Non-terminal callbacks update state but do not pretend the workflow finished.
- Terminal callbacks visibly complete the run.
- Timeout is visible as a terminal failure/recovery state.
- Governance failure is visible as "needs review" or "failed", not raw JSON.

### Phase 4: Workflow Management Hardening

Goal: safely manage workflow registry entries from KRIA.

This is a bounded subset of roadmap Stage 6. It is not a full n8n visual editor.

#### Supported Normal-Mode Actions

- View workflows.
- Import discovered workflow as draft.
- Approve workflow only after required metadata is present.
- Disable workflow.
- Delete KRIA registry entry.
- View execution history.

#### Developer-Mode Actions

Developer mode may show:

- Raw workflow endpoint path.
- Raw n8n execution payload.
- Raw JSON import preview.
- Signature/callback diagnostics.

Normal mode must not show raw n8n node complexity by default.

#### Required Metadata For Approval

A workflow should not be approved unless these fields are present:

- `workflow_id`
- `workflow_version`
- `display_name`
- `endpoint_path`
- `risk_tier`
- `irreversibility_class`
- `timeout_class`
- `environment`
- `owner`
- `requires_callback`
- `input_schema_ref`
- `output_schema_ref`
- `expected_evidence`
- `credential_requirements`
- `data_scope`
- `hitl_policy`

Recommended optional fields:

- `allowed_actions`
- `tags` if added later
- `description` if added later

#### Backend Tasks

| ID | Task | Files |
| --- | --- | --- |
| P4.1 | Validate approve/disable/delete command behavior | `crates/kria-desktop/src/commands/n8n.rs` |
| P4.2 | Ensure catalog is rebuilt with resolved secret after changes | `n8n.rs` |
| P4.3 | Persist config updates safely | config handling code |
| P4.4 | Return user-safe errors from all commands | `n8n.rs`, frontend |
| P4.5 | Add tests for approve/disable/delete if missing | command tests |

#### Acceptance Criteria

Phase 4 is complete only when:

- Import creates draft, not approved.
- Draft workflow cannot run.
- Approve requires safe metadata.
- Disable immediately prevents execution.
- Delete removes registry entry and refreshes UI.
- All actions refresh the shared n8n store.

### Phase 4.5: Workflow Authoring And Update Validation

Goal: allow KRIA to create or update n8n workflows safely, without corrupting
workflow JSON or overwriting existing workflows by accident.

This phase is required before any prompt-generated workflow feature. It can be
implemented as a manual/developer workflow validator first, then reused by Stage
9 AI workflow generation later.

#### Backend Tasks

| ID | Task | Files |
| --- | --- | --- |
| P4.5.1 | Add workflow draft type and validation report type | `crates/kria-core/src/n8n/types.rs` or new module |
| P4.5.2 | Add static JSON parser and schema validator | new `workflow_validation.rs` |
| P4.5.3 | Add graph integrity validator | new `workflow_validation.rs` |
| P4.5.4 | Add callback contract validator | new `workflow_validation.rs` |
| P4.5.5 | Add secret leak detector | new `workflow_validation.rs` |
| P4.5.6 | Add backup/export-before-update path | `commands/n8n.rs` |
| P4.5.7 | Add validate-only Tauri command | `commands/n8n.rs` |
| P4.5.8 | Add create/update-as-draft command | `commands/n8n.rs` |
| P4.5.9 | Add test-execution command for draft workflow | `commands/n8n.rs` |
| P4.5.10 | Add audit entries for every create/update attempt | local audit path |

#### Frontend Tasks

| ID | Task | Files |
| --- | --- | --- |
| P4.5.11 | Add workflow draft preview UI | future authoring component |
| P4.5.12 | Add validation results panel | future authoring component |
| P4.5.13 | Add existing workflow diff view | future authoring component |
| P4.5.14 | Add backup ID display before update | future authoring component |
| P4.5.15 | Add approve/deny controls after validation | future authoring component |

#### Validation Report Shape

Every validation command should return a structured report:

```json
{
  "status": "failed",
  "workflow_id": "draft_email_sender",
  "checks": [
    {
      "id": "json_parse",
      "status": "passed",
      "message": "Workflow JSON parsed"
    },
    {
      "id": "callback_contract",
      "status": "failed",
      "message": "Callback body is missing correlation_id"
    }
  ],
  "safe_to_import": false,
  "safe_to_activate": false,
  "backup_id": null
}
```

#### Acceptance Criteria

Phase 4.5 is complete only when:

- Invalid JSON is rejected before any n8n API call.
- Graph errors are reported with actionable messages.
- Missing callback fields are rejected for KRIA-invoked async workflows.
- Real secrets in JSON are rejected.
- Existing workflow update always creates a backup first.
- Create/update produces a draft, not an auto-approved workflow.
- A safe test execution is required before approval.

#### Verification Commands

The test names below are `to-create`. Phase 4.5 cannot be marked complete until
these commands exist and pass.

```bash
cargo test -p kria-core n8n_workflow_validation
cargo test -p kria-desktop n8n_workflow_authoring
./scripts/run_n8n_workflow_authoring_validation.sh
```

### Phase 5: Bounded Deterministic Invocation Enhancements

Goal: make invocation more natural without adding semantic AI routing.

This phase is still deterministic.

#### Allowed Enhancements

- Match exact `workflow_id`.
- Match exact `display_name`.
- Match explicit aliases if added to config.
- Match tags/categories if added.
- Show clarification when multiple deterministic matches exist.
- Show "available workflows" when no match exists.

#### Disallowed Enhancements

- Embedding search.
- Model-based routing.
- Automatic workflow selection from vague prompts.
- Recommendation engine.

#### Suggested Data Model Extension

Optional additions to `N8nWorkflowConfig`:

```rust
pub description: Option<String>,
pub tags: Vec<String>,
pub aliases: Vec<String>,
```

If this is added:

- Update serde defaults.
- Update config examples.
- Update workflow cards.
- Update deterministic dispatch.
- Add tests.

#### Acceptance Criteria

Phase 5 is complete only when:

- `Run test_workflow` still works.
- `Run Test Workflow` works by display name.
- Alias matching is exact and deterministic.
- Ambiguous matches ask the user to choose.
- No model/embedding dependency is introduced.

### Phase 6: Intelligence Readiness Gate

Goal: define when Stage 3 from the roadmap may begin.

Do not start semantic routing until all conditions below are true.

#### Required Gates

- Phase 0 through Phase 5 complete.
- Reliability suite passes 17/17 on a running app.
- At least three real workflows are registered with good metadata.
- Workflow cards and history are stable.
- Terminal callback path is verified with real n8n, not only manual curl.
- Unknown workflow, disabled workflow, bad signature, and timeout are all
  user-visible and tested.
- There is a clear eval set for workflow selection prompts.

#### Stage 3 First Slice

When the gates are satisfied, the first intelligent routing slice should be:

1. Rank workflows using existing metadata only.
2. Return top 3 suggestions.
3. Ask user to confirm.
4. Do not auto-run.

This prevents the first AI routing implementation from creating unsafe hidden
execution behavior.

## 6. Test Plan

### 6.1 Static Checks

Run after every meaningful backend or frontend change:

```bash
cargo check -p kria-core
cargo check -p kria-desktop
cd ui && npm run check
git diff --check
```

### 6.2 Unit And Integration Checks

Run targeted tests when touching n8n core:

```bash
cargo test -p kria-core n8n
```

Run desktop command tests if command behavior changes:

```bash
cargo test -p kria-desktop n8n
```

### 6.3 Reliability Suite

Run when KRIA local API is running:

```bash
./scripts/run_n8n_reliability_tests.sh
```

Required result:

```text
17 passed / 0 failed / 17 total
```

### 6.4 Capability Suite

Run when n8n and KRIA are both running:

```bash
./scripts/run_n8n_full_capability_eval.sh
```

Expected behavior:

- Core callable capabilities pass.
- Skips are acceptable only when explicitly documented.
- Failures require triage before moving phases.

### 6.5 Manual UI Smoke

Run:

```bash
cd ui && npm run check
cargo tauri dev
```

Manual checks:

1. Open KRIA UI.
2. Open workflow surface.
3. Confirm workflow cards render.
4. Run `test_workflow`.
5. Confirm immediate waiting state.
6. Confirm terminal callback result in chat.
7. Confirm workflow history updates.
8. Confirm no raw n8n JSON is shown in normal mode.
9. Narrow the window and confirm no layout overlap.

### 6.6 Realtime E2E Suite

This suite is required so prompts do not need to be individually tested through
the UI every time.

Proposed command:

```bash
./scripts/run_n8n_realtime_e2e.sh
```

Required scenarios:

| Scenario | Prompt/input | Expected event sequence | Expected final response |
| --- | --- | --- | --- |
| run approved workflow | `Run test_workflow` | started, accepted, callback, governance, chat_result | workflow completed |
| retry workflow | `Retry test_workflow` | started, accepted, callback, governance, chat_result | workflow completed |
| unknown workflow | `Run missing_workflow` | error only | workflow not found |
| disabled workflow | configured disabled workflow | error only | workflow not approved/disabled |
| running callback | signed `running` callback | callback, governance await | still waiting |
| terminal callback | signed `completed` callback | callback, governance, chat_result | completed |
| failed callback | signed `failed` callback | callback, governance recover, chat_result | failed/recovery |
| bad signature | callback with bad HMAC | rejection | signature invalid |
| missing signature | callback without HMAC | rejection | signature missing |
| timeout | no terminal callback before deadline | timeout event | timed out |

The script must fail if:

- a terminal callback does not produce a terminal chat/result event,
- raw webhook JSON appears in the normal response,
- correlation IDs are missing from debug output,
- event ordering is broken,
- the same terminal event creates duplicate chat messages.

### 6.7 Runtime Mode Eval

Proposed command:

```bash
./scripts/run_n8n_runtime_modes.sh
```

Required checks:

- managed Docker mode detects Docker availability,
- managed Docker mode starts or reports a clear failure,
- managed Docker mode reports dashboard URL,
- external mode validates a supplied base URL,
- external mode does not start/stop containers,
- missing API key produces a warning but still allows webhook-only workflows
  when applicable,
- callback URL guidance changes correctly for Docker vs external mode.

### 6.8 Workflow Authoring Eval

Proposed command:

```bash
./scripts/run_n8n_workflow_authoring_validation.sh
```

Required fixtures:

- valid minimal KRIA callback workflow,
- invalid JSON,
- duplicate node IDs,
- broken connection target,
- missing callback node,
- missing callback `correlation_id`,
- hardcoded secret literal,
- update existing workflow without backup request,
- safe update with backup and validation report.

### 6.9 Full Eval Gate

Before a stage is marked complete, produce a single report bundle:

```text
~/.kria/eval_reports/n8n_stage_<stage>_<timestamp>.txt
```

The report must include:

- static check results,
- backend tests,
- frontend tests,
- realtime E2E result,
- reliability result,
- runtime mode result if settings changed,
- workflow authoring result if CRUD/authoring changed,
- manual smoke checklist,
- known skips with reason.

## 7. User-Facing Text Rules

### 7.1 Good Messages

Use messages like:

```text
Workflow "Test Workflow" triggered. Waiting for n8n callback.
Workflow "Test Workflow" completed: Hello from n8n.
Workflow "Test Workflow" failed. Check recovery details.
Workflow "missing_workflow" was not found. Open workflows to see available options.
```

### 7.2 Bad Messages

Avoid messages like:

```text
Tool 'n8n_invoke_workflow' completed with error...
{"received": true}
n8n workflow invocation failed: ...
Tracking ID: ...
```

Internal identifiers can appear in developer/debug details, not in the primary
user-facing response.

## 8. Security Requirements

### 8.1 Secrets

- No real HMAC secret in tracked config or workflow exports.
- Secret source priority remains:
  1. OS keyring entry if configured,
  2. environment variable such as `KRIA_N8N_SIGNING_SECRET`,
  3. local secret file such as `~/.kria/secrets/n8n.key`.
- Literal config secret fields are deprecated and must be migrated/redacted.
- Prefer local secret file or n8n credential/env variable for development.

### 8.2 Callback Authentication

- `/api/n8n/callback` must remain Bearer-auth exempt only because it has its
  own HMAC authentication.
- Missing signature must fail closed.
- Invalid signature must fail closed.
- Wrong workflow version must fail closed.
- Unknown workflow ID must fail closed.

### 8.3 Payload Limits

- Keep callback payload limit at 128 KB unless there is a clear workflow need.
- Keep command payload bound through `max_payload_bytes`.

### 8.4 Replay And Freshness

The system must deduplicate by `event_id`, reject stale sequence numbers, and
reject callbacks outside the configured freshness window.

Required rules:

- Reject callbacks older than `callback_freshness_window_secs` unless a workflow
  is explicitly marked long-running and uses a separate freshness policy.
- Reject callbacks with timestamps more than `future_callback_skew_secs` in the
  future.
- Keep a bounded TTL cache of accepted/rejected event IDs to prevent replay.
- Include freshness rejection in reliability and realtime E2E tests.

### 8.5 Data Scope And External Handoff

n8n can connect KRIA to cloud/external systems, so data transfer must be
explicit.

Required rules:

- Every workflow must declare `data_scope`.
- External/cloud workflows must declare `external_data_transfer = true`.
- Sensitive `data_scope` values require HITL or explicit user confirmation.
- Governance audit must record transferred data classes, not raw data.
- Stage 7 local-to-cloud workflows are blocked until these controls exist.

## 9. Observability Requirements

### 9.1 Backend Events

Keep or replace the current `n8n_callback_trace` logs with structured logs that
make these hops visible:

- Auth passed or HMAC failed.
- Callback parsed.
- State machine decision.
- Governance decision.
- Persistence success/failure.
- Tauri event emitted.
- HITL bridge started if applicable.

### 9.2 Frontend Events

The frontend should log only development-useful diagnostics:

- Received terminal result event.
- Failed to process event.
- Failed command invocation.

Do not leave noisy logs in production builds if they interfere with normal use.

### 9.3 Durable Files

These files are part of verification and support:

- `~/.kria/n8n/callback_inbox.jsonl`
- `~/.kria/n8n/governance_audit.jsonl`
- `~/.kria/eval_reports/n8n_*.txt`

## 10. Risk Register

| Risk | Impact | Mitigation |
| --- | --- | --- |
| n8n workflow sends no terminal callback | User waits forever | UI waiting state + timeout + setup docs |
| Callback signs different body than it sends | KRIA rejects callback | Build body once, sign exact body |
| Old signed callback is replayed | False completion or duplicate side effect | Timestamp freshness, event TTL cache, sequence rejection |
| Secret committed to repo | Security breach | Use env/credential/file only |
| Workflow UI calls nonexistent command | Runtime failure | Verify Tauri command registration |
| Raw JSON leaks to user | Poor UX | Normal/debug mode split |
| Semantic routing added too early | Wrong workflow execution | Freeze Stage 3 until gates pass |
| Running workflow state lost on restart | Confusing status | JSONL replay + status reconciliation |
| Duplicate callback creates duplicate UI message | Bad UX | Dedup by event/run in store |
| Timeout class mismatch | Unexpected failure or long wait | Surface timeout class on workflow card |
| Managed Docker fails silently | User cannot run workflows | Runtime status command + `n8n_runtime` logs |
| External n8n misconfigured | Invocation/callback fails | Connection test with actionable setup guidance |
| Event stream missing terminal event | User sees stale state | Realtime E2E event-sequence eval |
| Prompt-generated workflow has invalid JSON | Corrupt n8n workflow | Validation pipeline before import/update |
| Generated update overwrites existing workflow | Data loss | Backup and diff before update |
| Secret leaks into workflow export | Security breach | Secret leak detector and credential references |
| UI settings drift from config | Confusing behavior | Save/test/reload settings through one store/command path |
| Local data sent to external workflow without consent | Privacy breach | data_scope enforcement, external transfer flag, HITL for sensitive scopes |

## 11. Phase Completion Checklist

Use this checklist before moving to the next phase.

### Phase 0 Complete

- [x] Test workflow export has no real secret.
- [x] Test workflow export uses current callback schema.
- [x] Test workflow export signs exact callback body.
- [x] Literal config secrets are migrated/redacted.
- [x] Callback freshness and future-skew rejection are tested.
- [x] Static checks pass.

Phase 0 implementation note, 2026-05-29:

- Added `scripts/run_n8n_phase0_contract.sh` as the static Phase 0 gate.
- Updated `config/n8n_test_workflow.json` to use the current callback envelope,
  `callback_body`, `callback_signature`, and runtime-sourced
  `KRIA_N8N_SIGNING_SECRET` instead of a literal secret.
- Verification passed:
  `./scripts/run_n8n_phase0_contract.sh`,
  `cargo test -p kria-core --lib n8n`,
  `git diff --check`,
  and the n8n secret scan.

### Phase 1 Complete

- [x] n8n is running and healthy.
- [x] KRIA local API is running.
- [x] Real workflow sends terminal callback.
- [x] Callback accepted by KRIA.
- [x] Governance persisted.
- [x] Chat-visible terminal event path is wired.
- [x] Reliability suite passes 17/17.

Phase 1 implementation note, 2026-05-29:

- Added an explicit local API chat bridge for prompts like `Run test_workflow`
  so the eval path invokes the allowlisted n8n workflow through KRIA instead of
  relying on raw JSON or manual UI clicks.
- Added a callback-only Docker bridge listener on `172.17.0.1:3001` while
  keeping the normal local API on `127.0.0.1:3001`. The bridge exposes only
  `POST /api/n8n/callback`, which still requires HMAC verification.
- Updated managed n8n startup to include the env access and `crypto` builtin
  flags required by KRIA's signed diagnostic workflow.
- Repaired the local n8n runtime by replacing the stale `n8n` container,
  preserving the previous container as `n8n-pre-kria-phase1-20260529200509`,
  importing/publishing the current test workflow, and restarting n8n.
- Verification passed:
  `./scripts/run_n8n_live_e2e.sh`
  (`~/.kria/eval_reports/n8n_live_e2e_20260529_203627.txt`) and
  `./scripts/run_n8n_reliability_tests.sh`
  (`~/.kria/eval_reports/n8n_reliability_20260529_202535.txt`).

### Phase 1.5 Complete

- [x] n8n settings are available in KRIA configuration UI.
- [x] Managed Docker mode can be selected and tested.
- [x] External n8n mode can be selected and tested.
- [x] Dashboard URL can be opened from KRIA.
- [x] Connection test reports health/API/callback guidance.
- [x] Secrets are masked and stored by env/file/reference, not tracked config.
- [x] Managed Docker uses local bind, pinned image, auth/encryption key, and non-privileged container.
- [x] Runtime logs identify startup or connection failure causes.

Phase 1.5 implementation note, 2026-05-29:

- Backend added `N8nConfig` v2 runtime-mode fields, managed Docker settings,
  env/file secret source fields, dashboard URL, callback preview fields, and
  last connection status.
- Desktop added runtime status, save settings, connection test, dashboard open,
  managed Docker start/stop/restart, and structured `n8n_runtime` /
  `n8n_config` logs.
- Settings UI added an `n8n` integration tab with external/managed mode,
  connection test, dashboard open, secret source visibility, Docker settings,
  auth/encryption settings, and runtime badges.
- Managed Docker start refuses privileged containers, unpinned images, missing
  encryption key file, and missing dashboard auth credentials when auth is
  required.
- Managed Docker start injects the n8n env-access and `crypto` builtin flags
  required for the signed diagnostic callback workflow.
- Verification passed:
  `cargo check -p kria-core`,
  `cargo check -p kria-desktop`,
  `cargo test -p kria-core --lib n8n`,
  `cd ui && npm run check`,
  `./scripts/run_n8n_runtime_modes.sh`,
  `git diff --check`,
  and the n8n secret scan.

### Phase 2 Complete

- [x] Workflow cards integrated into app.
- [x] Shared n8n store exists or current state handling is clearly justified.
- [x] Search/filter works.
- [x] Run button works for approved workflows.
- [x] Draft/disabled workflows cannot be run.
- [x] Recent runs visible.
- [x] Normal UI hides raw JSON.
- [x] Normal user can complete happy path without editing TOML.
- [x] Narrow viewport checked.

Phase 2 implementation note, 2026-05-29:

- Added dedicated `invoke_n8n_workflow_from_ui` Tauri command so workflow cards
  do not depend on chat prompt parsing or expose `n8n_invoke_workflow`.
- Added shared Solid n8n store with status loading, callback/governance event
  subscriptions, derived workflow/run selectors, optimistic accepted runs, and
  workflow invocation.
- Replaced the n8n dashboard default surface with a native workflow hub:
  workflow cards, search, status/risk/environment filters, guarded run button,
  recent runs, selected evidence/governance summary, and compact diagnostics.
- Extended diagnostics to show setup health, runtime mode, container health,
  dashboard URL, base URL, and callback URL from the shared n8n store.
- Moved raw run/governance JSON behind explicit `Technical details` expanders.
- Added responsive n8n styling and a Phase 2 contract gate script.
- Verification passed:
  `cargo check -p kria-core`,
  `cargo check -p kria-desktop`,
  `cargo test -p kria-core --lib n8n`,
  `cd ui && npm run check`,
  `cd ui && npm run test:run`,
  `cd ui && npm run build`,
  `./scripts/run_n8n_phase2_ui.sh`,
  `./scripts/run_n8n_runtime_modes.sh`,
  `./scripts/run_n8n_evals.sh`,
  `./scripts/run_n8n_full_capability_eval.sh`,
  `git diff --check`,
  and the n8n secret scan.

### Phase 3 Complete

- [x] Triggered/waiting/completed/failed states visible.
- [x] Non-terminal callbacks update status.
- [x] Terminal callbacks finalize status.
- [x] Timeout visible.
- [x] Governance failure visible.

Phase 3 implementation note, 2026-05-29:

- Added a reusable n8n progress model with explicit lifecycle states:
  `idle`, `triggering`, `accepted`, `waiting_for_callback`, terminal states,
  and `needs_review`.
- Added local optimistic `triggering`, `accepted`, and `rejected` run states in
  the shared n8n store so users never see an unqualified "Running..." state.
- Added `N8nRunProgress` for workflow cards and selected runs, showing current
  state, shortened correlation ID, elapsed time, last evidence timestamp,
  n8n run ID, waiting warnings, final summaries, and recovery hints.
- Updated the run timeline to show elapsed time, evidence time, waiting
  warnings, and reconcile controls per run.
- Updated selected-run handling to track `correlation_id` instead of a stale run
  object, so callback/governance refreshes update the visible selected result.
- Updated evidence/governance display to show "Verified", "Waiting for
  evidence", "Needs review", or "Failed" without requiring raw JSON.
- Added `scripts/run_n8n_phase3_progress.sh` and `ui/src/lib/n8nProgress.test.ts`.
- Verification passed:
  `./scripts/run_n8n_phase3_progress.sh`,
  `cd ui && npm run test:run -- n8nProgress`,
  `cd ui && npm run check`,
  `cd ui && npm run test:run`,
  `cd ui && npm run build`,
  `./scripts/run_n8n_phase2_ui.sh`,
  and `./scripts/run_n8n_live_e2e.sh`.

### Phase 4 Complete

- [x] Import as draft works.
- [x] Approve validates metadata.
- [x] Disable prevents execution.
- [x] Delete removes registry entry.
- [x] History and dashboard refresh after actions.

Phase 4 implementation note, 2026-05-29:

- Extended `N8nWorkflowConfig` with approval metadata required by Phase 4:
  `owner`, `requires_callback`, `input_schema_ref`, `output_schema_ref`,
  `credential_requirements`, `data_scope`, `expected_evidence`, and
  `hitl_policy`.
- Added reusable approval metadata validation. Approval now fails closed with a
  user-safe missing-field list instead of silently promoting incomplete draft
  workflows.
- Hardened import/approve/disable/delete commands with workflow ID/path
  validation, user-safe n8n API errors, registry logging under
  `n8n_workflow_registry`, safe config persistence, and catalog rebuild after
  every registry update.
- Added native workflow management UI inside the workflow hub: discover, import
  draft, approve, disable, delete, metadata readiness, and execution-history
  refresh.
- Updated default and local runtime `test_workflow` metadata so the existing
  approved diagnostic workflow remains compatible with the Phase 4 contract.
- Added `scripts/run_n8n_phase4_management.sh` plus Rust unit coverage for
  metadata validation, disabled workflow rejection, and unsafe endpoint
  rejection.
- Verification passed:
  `cargo test -p kria-core n8n --lib`,
  `cargo test -p kria-desktop n8n`,
  `cd ui && npm run check`,
  `cd ui && npm run test:run -- n8nProgress`,
  `cd ui && npm run test:run`,
  `cd ui && npm run build`,
  `./scripts/run_n8n_phase2_ui.sh`,
  `./scripts/run_n8n_phase3_progress.sh`,
  `./scripts/run_n8n_phase4_management.sh`,
  `./scripts/run_n8n_live_e2e.sh`,
  and `git diff --check`.

### Phase 4.5 Complete

- [x] Workflow JSON validator exists.
- [x] Callback contract validator exists.
- [x] Secret leak detector exists.
- [x] Existing workflow backup occurs before update.
- [x] Bad workflow JSON cannot be imported.
- [x] n8n version compatibility and non-mutating dry-run validation are checked.
- [x] Generated/updated workflows remain draft until approved.
- [x] Authoring validation eval passes.

Phase 4.5 implementation note, 2026-05-29:

- Added `crates/kria-core/src/n8n/workflow_validation.rs` with JSON parse,
  graph integrity, callback contract, secret leak, webhook endpoint inference,
  and n8n version compatibility checks.
- Added validate-only, dry-run, backup, rollback, and create/update-as-draft
  Tauri commands. The dry-run path reports validation and `mutated_n8n=false`.
- Existing registry updates create an automatic pre-update backup; draft JSON is
  also written as a separate local backup artifact.
- Added a destructive-safe CRUD fixture test that imports a temporary draft,
  approves it, verifies catalog execution, disables it, verifies execution is
  blocked, and deletes it from the temporary registry.
- Added `scripts/run_n8n_workflow_authoring_validation.sh`.
- Verification passed:
  `cargo test -p kria-core n8n_workflow_validation --lib`,
  `cargo test -p kria-desktop n8n_workflow_authoring`,
  `cargo test -p kria-desktop n8n_destructive_safe_crud_fixture`,
  and `./scripts/run_n8n_workflow_authoring_validation.sh`.

### Phase 5 Complete

- [x] Exact workflow ID matching works.
- [x] Display-name matching works.
- [x] Alias/tag matching, if added, is deterministic.
- [x] Ambiguous matches ask for clarification.
- [x] No embedding/model routing was introduced.

Phase 5 implementation note, 2026-05-29:

- Added a shared deterministic workflow matcher in `crates/kria-core/src/n8n/matching.rs`.
  It resolves exact `workflow_id`, exact `display_name`, exact aliases, and exact
  tags only. It does not use embeddings, model routing, semantic ranking, or
  recommendations.
- Extended `N8nWorkflowConfig` with `description`, `tags`, and `aliases`, with
  serde defaults for backward compatibility.
- Updated local API chat routing to extract the full workflow reference instead
  of truncating to the first token, so `Run Test Workflow` and exact aliases work.
- Added clarification responses when multiple deterministic matches exist, and
  available-workflow guidance when no deterministic match exists.
- Updated the agent deterministic dispatch path to use the same matcher before
  calling `n8n_invoke_workflow`.
- Updated workflow cards and the import-draft panel to expose tags and aliases.
- Updated evals with display-name and exact-alias prompt coverage.
- Added `scripts/run_n8n_phase5_invocation.sh`.
- Verification passed:
  `cargo test -p kria-core n8n --lib`,
  `cargo test -p kria-desktop n8n`,
  `cd ui && npm run check`,
  `cd ui && npm run test:run`,
  `cd ui && npm run build`,
  `./scripts/run_n8n_phase2_ui.sh`,
  `./scripts/run_n8n_phase3_progress.sh`,
  `./scripts/run_n8n_phase4_management.sh`,
  `./scripts/run_n8n_phase5_invocation.sh`,
  `./scripts/run_n8n_live_e2e.sh`,
  `./scripts/run_n8n_evals.sh`,
  and `./scripts/run_n8n_full_capability_eval.sh`.

### Phase 6 Complete

- [x] Stage 3 readiness is represented as a concrete gate instead of prose only.
- [x] The gate requires Phase 0 through Phase 5 evidence before intelligence can start.
- [x] The gate requires the reliability suite to pass 17/17 on a running app.
- [x] The gate requires at least three approved workflows with routing-quality metadata.
- [x] Workflow cards/history stability, live terminal callback evidence, unknown workflow,
  disabled workflow, bad signature, timeout, and selection eval coverage are checked.
- [x] Stage 3 first slice is constrained to metadata ranking, top-3 suggestions,
  user confirmation, and no auto-run.
- [x] No semantic, embedding, recommendation, or model-based n8n routing was introduced.

Phase 6 implementation note, 2026-05-29:

- Added `crates/kria-core/src/n8n/readiness.rs` with a reusable Stage 3
  readiness report. It emits `ready` only when every gate is true; otherwise it
  returns `blocked` with specific missing gates.
- Added a hard workflow metadata threshold:
  `N8N_STAGE3_REQUIRED_WORKFLOW_COUNT = 3`. A workflow counts only when it is
  approved and has approval metadata, display name, description, and tags or
  aliases.
- Added desktop readiness evidence collection from the latest n8n eval reports
  and exposed the result as `stage3_readiness` from `get_n8n_status`.
- Extended the native diagnostics panel to show Stage 3 readiness, blocked
  gates, and the first allowed intelligence slice when ready.
- Added a disabled-workflow execution rejection test so the Phase 6
  negative-path gate has direct coverage.
- Added `scripts/run_n8n_phase6_readiness_gate.sh` to verify the gate wiring,
  report evidence discovery, negative-path coverage, selection eval coverage,
  and that n8n semantic/model routing has not started.
- Updated reliability eval callback fixtures to include the configured
  `result` evidence key, matching the Phase 4 workflow contract.
- Current readiness expectation: Stage 3 should remain blocked until at least
  three real approved workflows are registered with good metadata.
- Verification passed:
  `cargo test -p kria-core n8n --lib`,
  `cargo test -p kria-desktop n8n`,
  `cd ui && npm run check`,
  `cd ui && npm run test:run`,
  `cd ui && npm run build`,
  `./scripts/run_n8n_phase2_ui.sh`,
  `./scripts/run_n8n_phase3_progress.sh`,
  `./scripts/run_n8n_phase4_management.sh`,
  `./scripts/run_n8n_phase5_invocation.sh`,
  `./scripts/run_n8n_reliability_tests.sh`,
  `./scripts/run_n8n_live_e2e.sh`,
  `./scripts/run_n8n_evals.sh`,
  `./scripts/run_n8n_full_capability_eval.sh`,
  and `./scripts/run_n8n_phase6_readiness_gate.sh`.
- Latest Phase 6 report:
  `/home/obaid/.kria/eval_reports/n8n_phase6_readiness_20260529_214913.txt`.
- Latest readiness result:
  `BLOCKED (only 1/3 approved workflows have routing-quality metadata)`.

### Stage Gate Complete

- [ ] The stage has an implementation summary.
- [ ] Automated tests are listed and passed.
- [ ] Manual smoke steps are listed and passed.
- [ ] Expected user responses are documented.
- [ ] Known skips are justified.
- [ ] A stage eval report exists under `~/.kria/eval_reports/`.

## 12. Recommended Next Implementation Slice

The best immediate slice is:

```text
Phase 0 + Phase 1 + the security subset of Phase 1.5:
Clean the test workflow contract, migrate/redact secrets, add callback
freshness, verify real live terminal callback, define runtime mode settings,
and re-run reliability tests.
```

Why this first:

- It closes the most important current caveats before UX work.
- It gives a trustworthy base for the UI work.
- It prevents building polished UX on top of a stale demo workflow export.
- It is low-risk and directly verifiable.

After that, implement:

```text
Phase 2:
Native workflow hub with cards, run button, recent runs, and evidence details.
```

Do not start Stage 3 intelligence until Phase 0 through Phase 5 are complete.
