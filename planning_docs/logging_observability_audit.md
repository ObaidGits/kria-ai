# KRIA Logging Observability Audit

Date: 2026-05-30
Status: implemented and verified
Scope: backend logs, frontend logs, n8n logs, GUI cognition logs, MCP startup,
tool execution logs, startup logs, workflow routing, workflow confirmation,
and runtime observability.

## 1. Current Logging Problems

Before this pass, KRIA logs had these practical problems:

| Area | Problem | Impact |
| --- | --- | --- |
| MCP startup | Per-provider and per-tool startup logs were too chatty at `INFO` | Terminal startup became noisy before the user could identify whether startup succeeded |
| n8n execution | Logs existed, but the flow was split across callback, governance, local API, and frontend events | A failed run required source-code knowledge to identify the failing hop |
| GUI cognition | Important transitions were not consistently grouped under one readable trace | GUI automation failures were hard to follow from terminal output |
| Tool execution | Some tool paths logged results, but not every path had a consistent execution ID, duration, and summary | Failures could be hard to correlate with the prompt and execution path |
| Frontend console | Development console logs for n8n/HITL events were noisy | Normal browser logs could obscure actual UI failures |
| Startup | Runtime readiness was visible through many individual lines, but there was no compact startup summary | A user could not quickly answer "what is running?" |
| Log levels | Several developer-detail messages were emitted at `INFO` | Production terminal output mixed milestones with implementation details |

## 2. Noise Sources

| Source | Previous behavior | New behavior |
| --- | --- | --- |
| MCP provider loading | Repeated provider/tool-level lines | One startup summary plus grouped failure reporting |
| MCP client internals | Initialization and tool-listing details at `INFO` | Moved to `DEBUG` or `TRACE` |
| Frontend n8n event handling | Console logs for every received event | Development-only `console.debug` helper |
| HITL workflow session logs | Direct console logging from store transitions | Development-only debug helper |
| n8n unmatched prompts | Potential routing noise from normal non-n8n prompts | Step logs emitted only when the prompt is an n8n prompt, confirmation, or explicit workflow reference |

## 3. Missing Logs

The following gaps were closed:

| Missing item | Implemented evidence |
| --- | --- |
| Startup summary | `startup_summary` target in `crates/kria-desktop/src/commands/runtime.rs` |
| MCP provider batch summary | `mcp_startup` summary in `crates/kria-core/src/mcp/server_manager.rs` |
| n8n step trace | `n8n_execution_trace` in `local_api.rs` and `n8n.rs` |
| n8n confirmation trace | Step 3/9 confirmation logs in local API and Tauri paths |
| n8n callback race visibility | Step 5 logs before awaiting webhook response, with explicit note that callbacks can arrive first |
| GUI cognition lifecycle | `gui_execution_trace` in `loop_engine/mod.rs` and `gui_wiring.rs` |
| Tool execution trace | `tool_execution` logs in ReAct, deterministic dispatch, and GUI policy tool executor paths |

## 4. Readability Problems

Resolved readability issues:

- n8n logs now use a numbered step model: Prompt Received, Workflow Routing,
  Confirmation Check, Webhook Invocation, Callback Waiting, Callback Received,
  Governance, Persistence, Response Delivery.
- Tool logs now include `tool_name`, `execution_id`, `session`, `duration_ms`,
  `success`, `failure_reason`, and redacted/summarized input/output.
- MCP startup now reports loaded providers, enabled/disabled counts, total tool
  count, elapsed startup time, and grouped failures.
- GUI cognition logs now show intent classification, capability resolution,
  plan generation, execution start, coordinator result, and workflow completion.

Remaining readability caveats:

- Some third-party/native stack logs still appear during GUI startup and display
  probing. These are outside the new KRIA targets and should be filtered further
  once the native sidecar boundaries are finalized.
- In this shell, `RUST_LOG=warn` suppresses `INFO` logs unless
  `KRIA_LOG_FILTER` is set explicitly. The verification run used
  `KRIA_LOG_FILTER="info,kria_pipeline=info,mcp_stderr=warn,sidecar_stderr=warn,tower_http=warn,llama-server=warn,ort=warn,ort::logging=warn,xcap=warn"`.

## 5. Startup Logging Review

Implemented in:

- `crates/kria-desktop/src/commands/runtime.rs`

Runtime evidence:

```text
/tmp/kria-logging-final.log:168
KRIA Startup Summary

/tmp/kria-logging-final.log:204
mcp_startup: [MCP] Startup summary: loaded 3/3 enabled provider(s), 125 tool(s), 7379ms
```

The startup summary now reports:

- version,
- GUI cognition status,
- n8n enabled/disabled state,
- MCP enabled/configured provider count,
- local API readiness,
- approved workflow count,
- hardware tier,
- startup time.

## 6. MCP Logging Review

Implemented in:

- `crates/kria-core/src/mcp/server_manager.rs`
- `crates/kria-core/src/mcp/client.rs`

Changes:

- Provider startup is batched.
- Disabled providers are logged once.
- Startup failures are grouped under one warning.
- Per-tool detail moved to `DEBUG`/`TRACE`.
- Client initialization internals moved out of `INFO`.

After example:

```text
[MCP] Loading providers...
[MCP] Startup summary: loaded 3/3 enabled provider(s), 125 tool(s), 7379ms
active=["fs (14 tools)", "gworkspace (110 tools)", "colab-mcp (1 tools)"]
```

This reduces terminal spam while preserving enough information to diagnose
provider availability and startup timing.

## 7. GUI Cognition Logging Review

Implemented in:

- `crates/kria-core/src/agent/loop_engine/mod.rs`
- `crates/kria-core/src/agent/gui_wiring.rs`

Runtime evidence:

```text
/tmp/kria-logging-final.log:250
[GUI] Prompt Received -> Intent Classified -> Execution Mode Selected

/tmp/kria-logging-final.log:256
[GUI] Workflow Plan Generated

/tmp/kria-logging-final.log:258
[GUI] Workflow execution coordinator started

/tmp/kria-logging-final.log:279
[GUI] Workflow Complete ... success=false ... Step 2 timed out after 8000ms
```

The GUI test intentionally exercised a real prompt:

```text
Open gedit and type logging audit probe, then stop
```

The workflow did not fully complete because the app-open step timed out in the
test environment, but the failure is now clearly traceable from terminal logs:

- prompt classified,
- workflow plan generated,
- step execution started,
- file write tool succeeded,
- open-application step timed out,
- workflow completed with error.

## 8. n8n Logging Review

Implemented in:

- `crates/kria-desktop/src/commands/local_api.rs`
- `crates/kria-desktop/src/commands/n8n.rs`

Runtime evidence:

```text
/tmp/kria-logging-final.log:220
[N8N][logging-final-n8n-1780114871374186823] Step 1/9 Prompt Received

/tmp/kria-logging-final.log:221
Step 2/9 Workflow Routing ... candidates=gmail_inbox_digest, gmail_search_messages, gmail_send_draft

/tmp/kria-logging-final.log:222
Step 3/9 Confirmation Check ... result=approved

/tmp/kria-logging-final.log:223
Step 4/9 Webhook Invocation

/tmp/kria-logging-final.log:224
Step 5/9 Callback Waiting

/tmp/kria-logging-final.log:225
Step 6/9 Callback Received ... status=Completed

/tmp/kria-logging-final.log:228
Step 7/9 Governance ... verification=Verified

/tmp/kria-logging-final.log:229
Step 8/9 Persistence ... callback_inbox_written=true

/tmp/kria-logging-final.log:233
Step 9/9 Response Delivery ... terminal chat_result emitted

/tmp/kria-logging-final.log:235
Webhook accepted by n8n ... status_code=200 accepted=true
```

Important implementation detail:

- Step 5 is now logged immediately after dispatching the webhook request and
  before awaiting the n8n HTTP response.
- This is intentional because n8n can send the callback before the webhook HTTP
  response has returned to KRIA.
- The later "Webhook accepted by n8n" line is logged as a milestone, not as a
  numbered lifecycle step.

## 9. Tool Execution Logging Review

Implemented in:

- `crates/kria-core/src/agent/loop_engine/mod.rs`
- `crates/kria-core/src/agent/gui_wiring.rs`

Runtime evidence:

```text
/tmp/kria-logging-final.log:267
tool_execution: Tool execution started ... tool_name=write_file ... input_summary=...

/tmp/kria-logging-final.log:268
tool_execution: Tool execution completed ... duration_ms=1 success=true
```

The log contract now includes:

- tool name,
- execution ID,
- session,
- input summary,
- duration,
- success flag,
- failure reason,
- result summary.

Payloads are summarized rather than dumped. The implementation uses the
existing JSON sanitization helper before logging inputs and results.

## 10. Correlation And Tracing Review

Current traceability is materially better after this pass:

| Flow | Correlation evidence |
| --- | --- |
| n8n prompt | `correlation_id` appears in every step line |
| n8n callback | callback trace, governance, persistence, and response delivery share the same correlation ID |
| GUI workflow | session and workflow ID appear at plan, execution, coordinator, and completion logs |
| Tool execution | execution ID and session are present at start and completion |
| MCP startup | provider counts, tool counts, active providers, and elapsed time are reported together |

Remaining traceability improvements to consider later:

- Add a single exported log bundle command for a correlation ID.
- Add a terminal-friendly "last run summary" command for support/debugging.
- Standardize every target on `execution_id` where older paths still use only
  session/workflow IDs.

## 11. Recommended Improvements

Completed now:

1. Batch MCP startup logs.
2. Add compact startup summary.
3. Add n8n step-based execution trace.
4. Add GUI cognition lifecycle trace.
5. Add consistent tool execution start/completion logs.
6. Reduce frontend console noise.
7. Move MCP client internals to lower log levels.

Recommended later:

1. Add file rotation and retention policy for runtime logs.
2. Add a user-accessible "export diagnostic bundle" action.
3. Add a redaction test suite that asserts secret-like fields never appear in
   `INFO` logs.
4. Add structured JSON log output mode for CI/eval runs.
5. Add per-run log bundle links into eval reports.

## 12. Implemented Improvements

| Improvement | Files |
| --- | --- |
| MCP startup batching and grouped failures | `server_manager.rs`, `client.rs` |
| Startup summary | `runtime.rs` |
| n8n step logs and callback ordering fix | `local_api.rs`, `n8n.rs` |
| n8n Tauri suggestion/invocation trace | `n8n.rs` |
| GUI cognition lifecycle logs | `loop_engine/mod.rs`, `gui_wiring.rs` |
| Tool execution start/completion logs | `loop_engine/mod.rs`, `gui_wiring.rs` |
| Frontend debug-only event logs | `ui/src/stores/app.ts`, `ui/src/stores/workflowSession.ts` |

## 13. Before Vs After Examples

### MCP Startup

Before:

```text
MCP provider A loaded
MCP provider B loaded
MCP provider C loaded
...
tool foo loaded
tool bar loaded
...
```

After:

```text
[MCP] Loading providers...
[MCP] Startup summary: loaded 3/3 enabled provider(s), 125 tool(s), 7379ms
Active:
- fs (14 tools)
- gworkspace (110 tools)
- colab-mcp (1 tools)
```

### n8n Execution

Before:

```text
n8n callback accepted
governance complete
chat_result emitted
```

After:

```text
[N8N][logging-final-n8n-1780114871374186823] Step 1/9 Prompt Received
[N8N][logging-final-n8n-1780114871374186823] Step 2/9 Workflow Routing
[N8N][logging-final-n8n-1780114871374186823] Step 3/9 Confirmation Check
[N8N][logging-final-n8n-1780114871374186823] Step 4/9 Webhook Invocation
[N8N][logging-final-n8n-1780114871374186823] Step 5/9 Callback Waiting
[N8N][logging-final-n8n-1780114871374186823] Step 6/9 Callback Received
[N8N][logging-final-n8n-1780114871374186823] Step 7/9 Governance
[N8N][logging-final-n8n-1780114871374186823] Step 8/9 Persistence
[N8N][logging-final-n8n-1780114871374186823] Step 9/9 Response Delivery
```

### GUI Workflow

Before:

```text
workflow failed
```

After:

```text
[GUI] Prompt Received -> Intent Classified -> Execution Mode Selected
[GUI] Capability Resolution complete; preparing workflow plan
[GUI] Workflow Plan Generated
[GUI] Workflow execution coordinator started
tool_execution: Tool execution started ... tool_name=write_file
tool_execution: Tool execution completed ... success=true
[GUI] Workflow Complete ... success=false ... Step 2 timed out after 8000ms
```

## 14. Verification Results

Static and unit checks:

| Command | Result |
| --- | --- |
| `cargo check -p kria-core` | PASS |
| `cargo check -p kria-desktop` | PASS |
| `cargo test -p kria-core n8n --lib` | PASS, 42 passed |
| `cargo test -p kria-desktop n8n` | PASS, 9 passed |
| `cd ui && npm run check` | PASS |
| `git diff --check` | PASS |

n8n evals:

| Command | Result |
| --- | --- |
| `./scripts/run_n8n_evals.sh` | PASS, 11/11 |
| `./scripts/run_n8n_live_e2e.sh` | PASS, 11/11 |
| `./scripts/run_n8n_reliability_tests.sh` | PASS, 17/17 |
| `./scripts/run_n8n_stage3_routing_eval.sh` | READY, 60 evaluated, 0 failures, 0% false auto-run |
| `./scripts/run_n8n_full_capability_eval.sh` | PASS, 23 passed, 0 failed, 18 skipped |

Runtime checks:

| Check | Result |
| --- | --- |
| KRIA local API health | PASS |
| n8n Docker health | PASS |
| MCP startup summary captured | PASS |
| n8n workflow trace captured | PASS |
| GUI workflow trace captured | PASS |
| Tool execution trace captured | PASS |

## 15. Production Readiness Verdict

Verdict: PASS for the logging/observability improvement phase.

The terminal output is now significantly more useful as a debugging dashboard:

- startup has a compact system summary,
- MCP startup is batched,
- n8n execution is traceable by numbered steps and correlation ID,
- GUI cognition has a readable lifecycle,
- tool executions expose duration and outcomes,
- frontend event logs are quieter by default.

Known remaining work is mostly operational polish: log retention, diagnostic
bundle export, and further filtering of third-party/native stack logs.
