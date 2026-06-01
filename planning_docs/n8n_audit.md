# KRIA ↔ n8n Integration — Deep Production Audit

**Date:** 2026-05-29
**Status:** Comprehensive audit with prioritized enhancement roadmap
**Scope:** Backend, Frontend, Runtime, Security, Streaming, CRUD, UX, DX

---

## 1. Executive Summary

KRIA's n8n integration is **architecturally sound but operationally incomplete**.

The backend substrate (9 Rust modules, ~1100 lines) implements a secure,
governance-aware invocation layer with HMAC signing, versioned allowlists,
callback state machines, dead-letter queues, and HITL bridges. This is
genuinely production-grade infrastructure.

**However, the integration currently feels like "KRIA calling an external
automation tool" rather than "KRIA natively owns workflow capabilities."**

Critical gaps that prevent native-feeling integration:

| Gap | Impact |
|-----|--------|
| Fire-and-forget execution (no streaming) | User sees "accepted" then silence |
| No chat-turn correlation for callbacks | Async results never appear in chat |
| No workflow CRUD beyond import-as-draft | Manual TOML editing required |
| No retry/recovery on invocation failure | Single-shot HTTP, no resilience |
| Dashboard-only UI (not conversational) | Workflows feel administrative |
| No execution progress streaming | Opaque black-box execution |
| Signing secret committed to VCS | Security vulnerability |

**Production Readiness Score: 5/10** (infrastructure solid, experience poor)

**Target Score: 9/10** achievable in 3 implementation waves.


---

## 2. Current Architecture Overview

```text
┌─────────────────────────────────────────────────────────────┐
│                    KRIA Desktop App                           │
├─────────────────────────────────────────────────────────────┤
│  Chat UI                    │  N8nDashboard (admin panel)    │
│  - "Run test_workflow"      │  - Status / Runs / Governance  │
│  - Deterministic dispatch   │  - Discovery / Import / Recon  │
│  - ToolEnd shown once       │  - 5s polling refresh          │
├─────────────────────────────┴────────────────────────────────┤
│  AgentLoop (loop_engine)           │  Tauri Commands (n8n.rs) │
│  - Deterministic n8n pattern       │  - get_n8n_status        │
│  - n8n_invoke_workflow tool call   │  - discover_n8n_workflows│
│  - StreamEvent::ToolEnd emitted    │  - import_n8n_workflow   │
│                                    │  - reconcile_n8n_run     │
├────────────────────────────────────┴─────────────────────────┤
│  kria-core/n8n module (9 files)                               │
│  ┌─────────┐ ┌──────────┐ ┌────────┐ ┌──────────────┐       │
│  │ config  │ │ catalog  │ │ client │ │    tool      │       │
│  │ N8nConf │ │ Allowlist│ │ HMAC+  │ │ ToolHandler  │       │
│  │ igurati │ │ Version  │ │ HTTP   │ │ Registration │       │
│  └─────────┘ └──────────┘ └────────┘ └──────────────┘       │
│  ┌──────────┐ ┌──────────────┐ ┌────────────────────┐       │
│  │ callback │ │    state     │ │    governance      │       │
│  │ HMAC+    │ │ RunState FSM │ │ Verify+Continue    │       │
│  │ Schema   │ │ Dead Letters │ │ Evidence Mapping   │       │
│  └──────────┘ └──────────────┘ └────────────────────┘       │
├──────────────────────────────────────────────────────────────┤
│  Local API (Axum)                                             │
│  POST /api/n8n/callback  ← n8n sends signed callbacks        │
│  GET  /api/n8n/hitl-response ← n8n polls for HITL decisions  │
└──────────────────────────────────────────────────────────────┘
          │                              ▲
          │ POST (HMAC-signed)           │ POST callback (HMAC-signed)
          ▼                              │
┌─────────────────────────────────────────────────────────────┐
│                    n8n (Docker)                                │
│  Webhook Node → Process → [optional callback to KRIA]         │
│  Port 5678                                                    │
└─────────────────────────────────────────────────────────────┘
```

### Data Flow (Current)

**Outbound (KRIA → n8n):**
1. User: "Run test_workflow" → deterministic dispatch → `n8n_invoke_workflow`
2. Catalog resolves workflow → validates approved + version match
3. Client builds `N8nCommandEnvelope`, signs with HMAC-SHA256
4. POST to `{base_url}{endpoint_path}` with signed headers
5. Immediate webhook response returned as `N8nInvocationResult`
6. `StreamEvent::ToolEnd` emitted to UI — **turn ends here**

**Inbound (n8n → KRIA, asynchronous):**
7. n8n workflow completes → POSTs callback to `/api/n8n/callback`
8. KRIA verifies HMAC + schema + workflow version
9. State machine ingests event (dedup, sequence check)
10. Governance evaluates run → continuation decision
11. If HITL needed → bridge spawns approval flow
12. Tauri event `n8n:callback` emitted → Dashboard refreshes

**The critical disconnect:** Steps 1-6 (synchronous turn) and steps 7-12
(asynchronous callback) are **completely unconnected**. The chat UI never
shows the eventual workflow completion/failure.


---

## 3. Backend Audit

### 3.1 What Works Well

| Component | Assessment | Evidence |
|-----------|-----------|----------|
| HMAC signing (outbound + inbound) | Production-grade | `client.rs:93-110`, `callback.rs:52-66`, constant-time compare |
| Catalog allowlist + version pinning | Robust | `catalog.rs:38-100` — rejects unknown, unapproved, mismatched |
| State machine (dedup + sequence) | Correct | `state.rs:79-110` — handles duplicate, out-of-order, terminal |
| Dead-letter queue | Good observability | `state.rs:121-135` — rejected events preserved |
| Governance engine | Well-designed | `governance.rs:37-95` — evidence-based verification |
| JSONL persistence + replay | Durable | `local_api.rs:697-715`, `runtime.rs:1975-2007` |
| Tool registration through ToolRegistry | Clean integration | `tool.rs:69-117` — standard KRIA tool |
| HITL bridge for n8n approvals | Complete flow | `local_api.rs:767-885` |

### 3.2 Critical Backend Weaknesses

| ID | Weakness | File:Line | Severity |
|----|----------|-----------|----------|
| B1 | **No retry/backoff on HTTP failure** — single-shot POST | `client.rs:108` | HIGH |
| B2 | **No cancellation propagation** — ctx.cancellation checked only pre-dispatch | `tool.rs:24-26` | MEDIUM |
| B3 | **Fire-and-forget** — tool returns immediately after webhook ack, no await on completion | `tool.rs:40-43` | CRITICAL |
| B4 | **No SSE/WS for n8n events** — Dashboard polls every 5s | entire architecture | HIGH |
| B5 | **Auth header inconsistency** — client uses Bearer, commands use X-N8N-API-KEY | `client.rs:106` vs `n8n.rs:103` | MEDIUM |
| B6 | **Deterministic dispatch sends `payload` not `input_payload`** | `loop_engine/mod.rs:168` | BUG |
| B7 | **HITL bridge hardcodes RiskLevel::Red** regardless of workflow risk_tier | `local_api.rs:817,845` | LOW |
| B8 | **Inbound body size unbounded** — no Axum body limit on callback endpoint | `local_api.rs:164` | MEDIUM |
| B9 | **No replay protection** — HMAC validated but no timestamp/nonce window | `callback.rs:52-66` | MEDIUM |
| B10 | **Signing secret in VCS** — `config/default.toml:289` | committed | HIGH |
| B11 | **No execution polling** — KRIA never proactively checks n8n execution status | architecture | MEDIUM |
| B12 | **Governance log cap hardcoded in 2 places** — 100 entries, magic number | `n8n.rs:127`, `local_api.rs:751` | LOW |

### 3.3 Backend Recommendations (Priority Order)

**P0 — Must Fix:**
1. **B6 (BUG):** Change `"payload"` → `"input_payload"` in deterministic dispatch
2. **B10:** Move signing_secret to `.env` or `~/.kria/secrets/n8n.key`, remove from VCS
3. **B3:** Implement "await-completion" mode (see Streaming section)

**P1 — Should Fix:**
4. **B1:** Add 3-retry with exponential backoff (500ms, 1s, 2s) for HTTP errors
5. **B4:** Add SSE endpoint `/api/n8n/events` streaming run state changes
6. **B8:** Add `axum::extract::DefaultBodyLimit::max(128 * 1024)` on callback route

**P2 — Nice to Have:**
7. **B5:** Standardize on `X-N8N-API-KEY` header for all n8n REST API calls
8. **B9:** Add `occurred_at_ms` window check (reject callbacks > 5min old)
9. **B7:** Use workflow's configured `risk_tier` in HITL bridge


---

## 4. Frontend Audit

### 4.1 Current State

**Only one UI surface exists:** `ui/src/components/N8nDashboard.tsx` (250 lines)

| Feature | Status | Notes |
|---------|--------|-------|
| Workflow listing | ✅ Basic | Shows configured_workflows from config |
| Execution runs display | ✅ Basic | correlation_id + status + sequence |
| Governance decisions | ✅ Basic | continuation_action + explanation |
| Dead letters | ✅ Count | Number shown, no drilldown |
| Discovery (remote) | ✅ Button | Returns raw JSON dump |
| Reconcile button | ✅ Per-run | Hits n8n API for execution details |
| Real-time updates | ⚠️ Polling | 5s interval + `n8n:callback` event |
| Evidence drilldown | ❌ Missing | Only shows `evidence_log.length` |
| Workflow search/filter | ❌ Missing | |
| Workflow categories/tags | ❌ Missing | |
| Execution progress | ❌ Missing | No node-by-node visualization |
| Output formatting | ❌ Missing | Raw JSON only |
| Chat integration | ❌ Missing | Results never appear in ChatView |
| CRUD operations | ❌ Partial | Import-as-Draft only, no approve/delete |

### 4.2 Critical Frontend Weaknesses

| ID | Weakness | Severity |
|----|----------|----------|
| F1 | **No chat integration** — workflow results never appear in conversation | CRITICAL |
| F2 | **No execution progress** — user sees "accepted" then waits for poll | HIGH |
| F3 | **No evidence viewer** — evidence_log items not expandable | HIGH |
| F4 | **No workflow invoke from chat** naturally beyond tool-call format | MEDIUM |
| F5 | **Raw JSON for discovery** — not rendered as selectable workflow cards | MEDIUM |
| F6 | **No workflow status badges** in chat messages | MEDIUM |
| F7 | **No missing_evidence display** in governance entries | LOW |
| F8 | **No SolidJS store** — all state local to Dashboard component | MEDIUM |
| F9 | **No responsive/mobile layout** for Dashboard | LOW |
| F10 | **Polling (5s) wasteful** when no workflows running | LOW |

### 4.3 Frontend Recommendations

**P0:**
1. **F1:** When `n8n:callback` arrives with terminal status, inject a system
   message into the active chat session showing the workflow result
2. **F2:** Add `WorkflowExecutionCard` component in ChatView that shows
   live status (Accepted → Running → Completed/Failed)

**P1:**
3. **F3:** Evidence viewer — collapsible JSON tree per evidence entry
4. **F5:** Workflow browser — cards with name, description, risk badge,
   one-click invoke button
5. **F8:** Create `stores/n8n.ts` with reactive signals for workflow state

**P2:**
6. **F4:** Natural-language workflow invoke: "send an email to X" routes to
   configured workflow if matching display_name or tags
7. **F6:** Inline workflow status badge in chat messages (✓ completed, ⏳ running)

---

## 5. Runtime / Operational Audit

### 5.1 Identified Issues

| ID | Issue | Root Cause | Risk |
|----|-------|-----------|------|
| R1 | **State machine uses std::sync::Mutex** — blocks async runtime briefly | `state.rs:73-76` | LOW (hold time <1ms) |
| R2 | **JSONL append has no fsync** — crash loses last callback | `local_api.rs:710-714` | MEDIUM |
| R3 | **Catalog rebuild on import is non-atomic with in-flight handlers** | `n8n.rs:236-247` | LOW |
| R4 | **Duplicate HITL bridge spawns** possible on rapid PauseForHitl callbacks | `local_api.rs:767` | MEDIUM |
| R5 | **`n8n_hitl_responses` never cleaned up** — grows unbounded | `local_api.rs:878` | LOW |
| R6 | **Callback URL in status uses config.server.port** not actual API port | `n8n.rs:38-42` | LOW |
| R7 | **No health check for n8n connectivity** before invocation | architecture | MEDIUM |
| R8 | **No execution timeout enforcement** — long-running workflows have no KRIA-side deadline | architecture | HIGH |

### 5.2 Operational Recommendations

1. **R8:** Add a configurable per-workflow deadline. If no callback arrives
   within `timeout_class` duration (Interactive=30s, Background=5min,
   LongRunning=1h), emit a timeout governance decision automatically.
2. **R4:** Track `pending_hitl_requests` per correlation_id; skip bridge
   spawn if one already pending for that run.
3. **R7:** Add `n8n_health_check()` at startup and before each invocation
   (simple GET to `{base_url}/api/v1/executions?limit=1`, timeout 3s).
4. **R5:** Expire `n8n_hitl_responses` entries older than 10 minutes.

---

## 6. Observability Audit

### Current State

| Capability | Status |
|-----------|--------|
| Structured logging (tracing) | ✅ `[n8n]` prefix at startup + errors |
| JSONL audit trail | ✅ callbacks + governance decisions |
| In-memory governance log (cap 100) | ✅ |
| Dead-letter preservation | ✅ |
| Metrics / histograms | ❌ None |
| Span-based tracing (invocation latency) | ❌ None |
| Error classification in telemetry | ❌ Only in tool result |
| Event bus integration | ❌ No connection to automation event_bus |
| Dashboard real-time | ⚠️ 5s polling |
| Health endpoint for n8n status | ❌ None |

### Recommendations

1. Add `tracing::span` around each `N8nClient::invoke` call with fields:
   `workflow_id`, `correlation_id`, `duration_ms`, `status_code`
2. Expose `/api/n8n/health` endpoint: checks n8n connectivity, returns
   catalog size, active runs count, dead-letter count
3. Add metrics counters: `n8n_invocations_total`, `n8n_callbacks_total`,
   `n8n_failures_total`, `n8n_hitl_requests_total`
4. Replace 5s polling with SSE push (event-driven, zero waste)

---

## 7. Streaming / Realtime Audit

### Current State: NO STREAMING

The n8n integration has **zero streaming capability**. Execution feels like:

```
User: "Run test_workflow"
KRIA: "✓ n8n_invoke_workflow [accepted, correlation_id: abc-123]"
... [silence, minutes pass] ...
[Dashboard shows "Completed" on next 5s poll — user never sees this in chat]
```

### What Streaming Should Feel Like

```
User: "Run test_workflow"
KRIA: "⏳ Starting workflow 'Test Workflow'..."
KRIA: "  → Node 1: Webhook received ✓"
KRIA: "  → Node 2: Processing data..."
KRIA: "  → Node 3: Sending callback..."
KRIA: "✓ Workflow completed: Hello from n8n!"
```

### Streaming Architecture Recommendation

**Two complementary approaches:**

#### Approach A: Callback-Based Progressive Updates (n8n side)

n8n workflow sends **multiple callbacks** at each node completion:
```
sequence_number: 1, status: "running", evidence: {node: "Webhook", done: true}
sequence_number: 2, status: "running", evidence: {node: "Process", done: true}
sequence_number: 3, status: "completed", evidence: {result: "Hello from n8n!"}
```

KRIA state machine already supports this (sequence ordering, non-terminal
→ terminal transitions). Just need:
1. Frontend to render intermediate callbacks in chat
2. n8n workflow template that sends per-node progress callbacks

#### Approach B: Execution Polling (KRIA side)

KRIA proactively polls `GET /api/v1/executions/{id}` every 2s while a
workflow is Running. n8n's execution API shows per-node status.

Requirements:
- n8n API key configured
- Background polling task per active run
- Node status mapping to human-readable progress

**Recommendation:** Implement Approach A first (lower complexity, works
without n8n API key). Approach B as enhancement for detailed node tracing.


---

## 8. Workflow CRUD Feasibility Audit

### n8n REST API Capabilities (v1)

| Operation | n8n Endpoint | Auth Required | KRIA Support |
|-----------|-------------|---------------|--------------|
| List workflows | `GET /api/v1/workflows` | X-N8N-API-KEY | ✅ `discover_n8n_workflows` |
| Get workflow | `GET /api/v1/workflows/{id}` | X-N8N-API-KEY | ❌ Not implemented |
| Create workflow | `POST /api/v1/workflows` | X-N8N-API-KEY | ❌ Not implemented |
| Update workflow | `PATCH /api/v1/workflows/{id}` | X-N8N-API-KEY | ❌ Not implemented |
| Delete workflow | `DELETE /api/v1/workflows/{id}` | X-N8N-API-KEY | ❌ Not implemented |
| Activate workflow | `PATCH /api/v1/workflows/{id}` `{active:true}` | X-N8N-API-KEY | ❌ Not implemented |
| List executions | `GET /api/v1/executions` | X-N8N-API-KEY | ❌ (only in reconcile) |
| Get execution | `GET /api/v1/executions/{id}` | X-N8N-API-KEY | ✅ `reconcile_n8n_run` |

### What's Realistically Possible

**Full CRUD is achievable** through n8n's REST API. Requirements:
1. **API key must be configured** — currently `api_key = ""` in default config
2. **Auth header must be `X-N8N-API-KEY`** (standardize from current inconsistency)

### CRUD Implementation Plan

```rust
// New Tauri commands needed:
#[tauri::command] async fn list_n8n_workflows(state: State<'_, AppStateCell>) -> ...
#[tauri::command] async fn get_n8n_workflow(id: String, ...) -> ...
#[tauri::command] async fn activate_n8n_workflow(id: String, ...) -> ...
#[tauri::command] async fn deactivate_n8n_workflow(id: String, ...) -> ...
#[tauri::command] async fn approve_kria_workflow(workflow_id: String, ...) -> ...
#[tauri::command] async fn delete_kria_workflow(workflow_id: String, ...) -> ...
```

### KRIA-Side Workflow Lifecycle

```
Import/Discover → Draft → Test → Approved → [Deprecated | Disabled]
                                     ↑
                            Only "Approved" can execute
```

Currently transitions require TOML edits. With CRUD commands:
- `approve_kria_workflow` promotes Draft → Approved (rebuilds catalog)
- `disable_kria_workflow` sets Disabled (safe stop)
- `delete_kria_workflow` removes from config + catalog

---

## 9. UX / DX Audit

### Developer Experience — Current Pain Points

| Action | Difficulty | Why |
|--------|-----------|-----|
| Add a workflow | HIGH | Edit TOML, restart KRIA, activate n8n, match UUIDs |
| Debug a failed workflow | HIGH | Check JSONL files manually, no structured view |
| Test a workflow | MEDIUM | Must click "Execute" in n8n first for test URLs |
| See workflow output | HIGH | Only visible in Dashboard poll, not chat |
| Understand errors | MEDIUM | Error classification exists but not user-friendly |
| Retry a failed invocation | IMPOSSIBLE | No retry UI or command |
| Cancel a running workflow | IMPOSSIBLE | No cancellation path |

### User Experience — Layman Assessment

A non-developer user would:
- ❌ Never discover available workflows (hidden in admin Dashboard)
- ❌ Not understand "correlation_id" or "governance decision"
- ❌ Not know how to troubleshoot "webhook not registered"
- ❌ Not realize workflow completed (no chat feedback)
- ✅ Be able to type "Run test_workflow" (if told the exact name)

### DX Recommendations

1. **One-command workflow setup:**
   ```
   kria n8n add --url http://localhost:5678/webhook/abc --name "My Workflow"
   ```
   Auto-generates TOML entry + approves + rebuilds catalog

2. **In-chat workflow discovery:**
   "What workflows can I run?" → KRIA lists available workflows with descriptions

3. **Auto-detect webhook URL from n8n API:**
   When importing, fetch the workflow's webhook nodes and auto-populate endpoint_path

---

## 10. Security + Scalability Audit

### Security Issues

| ID | Issue | Severity | Mitigation |
|----|-------|----------|-----------|
| S1 | Signing secret in VCS (`config/default.toml:289`) | HIGH | Move to `.env` or secrets file |
| S2 | No replay protection (timestamp/nonce) | MEDIUM | Add `occurred_at_ms` window check |
| S3 | Inbound body size unbounded | MEDIUM | Add Axum body limit layer |
| S4 | HITL responses never expire from memory | LOW | Add TTL cleanup (10min) |
| S5 | No CSRF on callback endpoint | LOW | Acceptable: HMAC is sufficient gate |

### Scalability Concerns

| Concern | Current Limit | Recommendation |
|---------|--------------|----------------|
| Concurrent workflows | Unlimited (no throttle) | Add max_concurrent_invocations config |
| State store memory | Grows with runs (no eviction) | Add TTL eviction for completed runs (1h) |
| JSONL inbox size | Unbounded file growth | Rotate daily, compress old files |
| Governance log | 100 entries (ring buffer) | Sufficient for in-memory |
| Dead letters | Unbounded Vec | Cap at 1000, rotate oldest |

---

## 11. Root Cause Analysis

### Why n8n Feels "External"

**Root Cause 1: Fire-and-forget architecture**
The tool handler returns the webhook ack immediately. The agent loop finishes
the turn. Async callbacks arrive on a completely separate channel that the
chat UI never consumes. This is the #1 reason it feels disconnected.

**Root Cause 2: Admin-only UI surface**
N8nDashboard is an administrative monitoring panel, not a user-facing
workflow experience. It's designed for operators debugging state, not for
users running workflows conversationally.

**Root Cause 3: No conversational bridge**
The LLM/agent loop has no mechanism to "wait for n8n completion" and then
summarize the result in natural language. Workflow outputs are raw JSON that
a layman cannot interpret.

**Root Cause 4: Manual workflow lifecycle**
Adding, approving, or modifying workflows requires TOML editing + restart.
This makes the system feel static rather than dynamic.

---

## 12. Identified Weaknesses Table

| # | Category | Weakness | Severity | Effort |
|---|----------|----------|----------|--------|
| 1 | Streaming | No execution progress in chat | CRITICAL | 3 days |
| 2 | UX | Chat never shows async workflow results | CRITICAL | 2 days |
| 3 | Security | Signing secret committed to VCS | HIGH | 1 hour |
| 4 | Resilience | No retry/backoff on invocation | HIGH | 4 hours |
| 5 | CRUD | No approve/disable/delete commands | HIGH | 1 day |
| 6 | Bug | `payload` vs `input_payload` field name | HIGH | 10 min |
| 7 | Streaming | No SSE endpoint for n8n events | HIGH | 1 day |
| 8 | UX | Dashboard is admin-only, not conversational | MEDIUM | 3 days |
| 9 | DX | Adding workflows requires TOML + restart | MEDIUM | 1 day |
| 10 | Observability | No metrics/spans for invocations | MEDIUM | 4 hours |
| 11 | Timeout | No KRIA-side execution deadline | MEDIUM | 4 hours |
| 12 | Security | No inbound body size limit | MEDIUM | 30 min |
| 13 | Security | No replay window check | MEDIUM | 2 hours |
| 14 | Scalability | State store never evicts old runs | LOW | 2 hours |
| 15 | UX | Raw JSON for discovery results | LOW | 4 hours |


---

## 13. Production Risk Table

| Risk | Probability | Impact | Mitigation |
|------|------------|--------|-----------|
| n8n down → invocation fails silently | HIGH | User confusion | Health check + clear error |
| Signing secret leaked from VCS | MEDIUM | Full compromise | Move to secrets file |
| Long workflow → user thinks KRIA stuck | HIGH | Poor UX | Progress streaming |
| Duplicate HITL spawns on rapid callbacks | LOW | Double approval prompts | Track pending per correlation |
| JSONL grows unbounded over months | MEDIUM | Disk full | Daily rotation |
| Body size attack on callback endpoint | LOW | Memory exhaustion | Axum body limit |
| n8n updates webhook UUID on redeploy | HIGH | 404 until config updated | Auto-detect from API |

---

## 14. Prioritized Fixes

### Wave 1: Make It Work Properly (Days 1-3)

| # | Fix | Files | Impact |
|---|-----|-------|--------|
| 1 | Fix `payload` → `input_payload` bug | `loop_engine/mod.rs:168` | Workflows receive correct data |
| 2 | Move signing secret out of VCS | `config/default.toml`, `.env` | Security |
| 3 | Add retry with backoff (3x) | `n8n/client.rs` | Resilience |
| 4 | Add n8n health check before invoke | `n8n/client.rs` | Clear error on n8n down |
| 5 | Add inbound body size limit | `local_api.rs` route config | Security |
| 6 | Add execution timeout per workflow | `state.rs` + background task | No silent hangs |
| 7 | Expire old HITL responses (10min) | `local_api.rs` | Memory bounded |

### Wave 2: Make It Feel Native (Days 4-8)

| # | Fix | Files | Impact |
|---|-----|-------|--------|
| 8 | Inject workflow results into chat on completion | `chat.rs` + new listener | Users see results |
| 9 | Add SSE `/api/n8n/events` endpoint | `local_api.rs` | Real-time updates |
| 10 | Add `approve_kria_workflow` command | `commands/n8n.rs` | No TOML editing |
| 11 | Add `disable_kria_workflow` command | `commands/n8n.rs` | Safe stop |
| 12 | Add `delete_kria_workflow` command | `commands/n8n.rs` | Clean removal |
| 13 | Workflow browser UI (cards + invoke button) | `ui/src/components/` | Discovery |
| 14 | In-progress indicator in chat | `chat.rs` + `WorkflowRunCard` | Live feedback |

### Wave 3: Make It Conversational (Days 9-14)

| # | Fix | Files | Impact |
|---|-----|-------|--------|
| 15 | LLM-assisted workflow discovery ("What can I automate?") | `loop_engine` routing | Natural use |
| 16 | Semantic output summarization (JSON → natural language) | `loop_engine` post-tool | Readable results |
| 17 | Multi-callback progress rendering in chat | `chat.rs` + new component | Live streaming feel |
| 18 | Workflow retry/re-run from chat | deterministic dispatch | Recovery |
| 19 | Natural-language workflow parameterization | LLM extracts input_payload | Smart invoke |
| 20 | Workflow history view (last 50 executions) | `commands/n8n.rs` + UI | Audit trail |

---

## 15. Recommended Architectural Improvements

### 15.1 Chat-Workflow Bridge (Most Important)

```rust
// New: WorkflowCompletionBridge — listens for terminal n8n callbacks
// and injects a system message into the originating chat session.

pub struct WorkflowCompletionBridge {
    session_map: Arc<RwLock<HashMap<String, String>>>, // correlation_id → session_id
}

impl WorkflowCompletionBridge {
    /// Called by tool handler BEFORE invoke — registers the mapping
    pub fn register_invocation(&self, correlation_id: &str, session_id: &str) { ... }

    /// Called by callback handler when terminal status arrives
    pub fn on_terminal_callback(&self, correlation_id: &str, run: &N8nWorkflowRunState) {
        if let Some(session_id) = self.get_session(correlation_id) {
            // Inject a system message into that chat session
            // showing the workflow result
            emit_workflow_completion_to_chat(session_id, run);
        }
    }
}
```

### 15.2 Execution Progress Channel

```rust
// SSE endpoint that streams per-run events in real-time
// Replaces 5s Dashboard polling

GET /api/n8n/events?correlation_id=<optional>

Events:
  event: run_started
  data: {"correlation_id":"abc","workflow_id":"test_workflow"}

  event: progress
  data: {"correlation_id":"abc","node":"Process","status":"running"}

  event: completed
  data: {"correlation_id":"abc","status":"completed","result":{...}}
```

### 15.3 Workflow Registry (Dynamic CRUD)

```rust
// Replace static TOML-only workflow management with dynamic registry
// that supports hot-reload without KRIA restart

pub struct DynamicWorkflowRegistry {
    config_workflows: Vec<N8nWorkflowConfig>,  // from TOML (base)
    runtime_workflows: Vec<N8nWorkflowConfig>, // from commands (dynamic)
    catalog: Arc<RwLock<N8nCatalog>>,          // rebuilt on any change
}

impl DynamicWorkflowRegistry {
    pub fn approve(&mut self, workflow_id: &str) -> Result<()> { ... }
    pub fn disable(&mut self, workflow_id: &str) -> Result<()> { ... }
    pub fn delete(&mut self, workflow_id: &str) -> Result<()> { ... }
    pub fn hot_reload(&self) { /* rebuild catalog, no restart */ }
}
```


---

## 16. Conversational Workflow Integration Recommendations

### Goal: "Run my email workflow" → KRIA handles everything conversationally

**Step 1: Workflow Name Resolution (like app alias resolution)**
```
User: "Send the weekly report"
KRIA: [matches display_name "Weekly Report Email" → workflow_id "weekly_report"]
KRIA: "Running 'Weekly Report Email'..."
```

**Step 2: Parameter Extraction via LLM**
```
User: "Send an email to john@company.com with subject 'Q4 Results'"
KRIA: [LLM extracts: {to: "john@company.com", subject: "Q4 Results"}]
KRIA: [invokes workflow with input_payload = extracted params]
```

**Step 3: Live Progress in Chat**
```
KRIA: "⏳ Sending email..."
KRIA: "  → Composing message ✓"
KRIA: "  → Sending via Gmail API ✓"
KRIA: "✓ Email sent to john@company.com — Subject: Q4 Results"
```

**Step 4: Error Recovery**
```
KRIA: "⚠️ Email workflow failed: Gmail authentication expired"
KRIA: [Recovery options:]
  - [Re-authenticate Gmail]
  - [Retry workflow]
  - [Cancel]
```

### Implementation Requirements

1. **Display name matching** in deterministic dispatch (already partially done)
2. **LLM parameter extraction** for workflows with known input schemas
3. **Multi-callback rendering** in chat (each progress update = new message line)
4. **Recovery options** on failure (reuse existing RecoveryOptions infrastructure)

---

## 17. Native KRIA Workflow Experience Plan

### Vision: Workflows as KRIA Capabilities

```
┌─────────────────────────────────────────────┐
│ What the USER sees:                          │
│                                              │
│ "KRIA, send the weekly report"              │
│ "KRIA, fetch my Jira tickets"              │
│ "KRIA, backup the database"                │
│ "What workflows do I have?"                 │
│ "Show me the last 5 workflow runs"          │
│ "Why did the email workflow fail?"          │
│                                              │
│ = KRIA capabilities, not "n8n commands"      │
└─────────────────────────────────────────────┘

┌─────────────────────────────────────────────┐
│ What happens BEHIND the scenes:              │
│                                              │
│ Intent → workflow match → catalog resolve    │
│ → payload build → HMAC sign → POST webhook  │
│ → progress callbacks → chat rendering       │
│ → terminal → governance → result in chat    │
│                                              │
│ User never sees: HMAC, correlation_ids,      │
│ webhook URLs, JSON envelopes, TOML config    │
└─────────────────────────────────────────────┘
```

### Implementation Phases

**Phase A — Transparent Execution (Week 1)**
- Workflow completion shows in chat (bridge correlation→session)
- Progress indicator while running ("⏳ Running...")
- Error shows in chat with recovery options

**Phase B — Conversational Access (Week 2)**
- "What workflows can I run?" → list with descriptions
- Display-name matching for natural invocation
- LLM extracts parameters from natural language

**Phase C — Full Native Experience (Week 3-4)**
- Workflow browser sidebar panel
- One-click invoke with parameter form
- Execution history view
- Auto-retry on transient failures
- Workflow templates and recommendations

---

## 18. Final Production Readiness Assessment

### Current Score: 5/10

| Dimension | Score | Notes |
|-----------|-------|-------|
| Security | 6/10 | HMAC good, secret in VCS bad |
| Resilience | 3/10 | No retry, no timeout, fire-and-forget |
| Observability | 4/10 | JSONL audit good, no metrics/spans |
| UX (Developer) | 4/10 | TOML editing, manual UUID matching |
| UX (End User) | 2/10 | No chat integration, admin dashboard only |
| Streaming | 1/10 | Zero progressive output |
| CRUD | 3/10 | Import-draft only, no approve/delete |
| Scalability | 6/10 | Sound architecture, missing eviction/limits |
| Conversational | 2/10 | Deterministic dispatch works, nothing else |
| Integration Depth | 5/10 | Governance + HITL good, no chat bridge |

### After Wave 1 (3 days): 7/10
Security fixes + retry + timeout + bug fix + body limit

### After Wave 2 (8 days): 8/10
Chat integration + SSE + CRUD commands + workflow browser

### After Wave 3 (14 days): 9/10
Conversational discovery + LLM params + progress streaming + history

---

## 19. Future Integration & Feature Enhancement Plan

### 19.1 Advanced Workflow Browsing
- **Workflow categories:** Group by domain (email, data, devops, reporting)
- **Tags system:** User-defined tags per workflow for quick filtering
- **Search:** Full-text search across display_name + description + tags
- **Favorites:** Pin frequently-used workflows to quick-access panel
- **Recent:** Show last 5 invoked workflows for instant re-run

### 19.2 Workflow Templates & Marketplace
- **Built-in templates:** Pre-configured workflow JSONs for common tasks
  (email send, Jira fetch, Slack notify, DB backup, RSS digest)
- **Community sharing:** Export workflow + KRIA config as portable bundle
- **One-click install:** Import template → auto-register → approve → ready
- **Template gallery UI:** Browse templates by category with screenshots

### 19.3 Conversational Workflow Intelligence
- **Intent matching:** Map natural prompts to workflows via display_name + description embeddings
- **Parameter inference:** LLM extracts structured `input_payload` from natural language
- **Confirmation flow:** "I'll send an email to john@co.com. Proceed?" [Yes] [Edit]
- **Context memory:** Remember last-used parameters ("send to the usual list")
- **Chained workflows:** "Fetch Jira tickets and send summary to Slack" → two sequential invocations

### 19.4 Live Execution Visualization
- **Node-by-node progress:** Each n8n node reports status via callback
- **Execution timeline:** Visual horizontal bar showing node progression
- **Data preview:** Show intermediate data flowing between nodes
- **Error pinpointing:** Highlight exactly which node failed + input/output
- **Estimated time:** Based on historical execution duration per workflow

### 19.5 Workflow Analytics & History
- **Execution history:** Last N runs per workflow with status, duration, result
- **Success rate dashboard:** Per-workflow reliability metrics
- **Duration trends:** Spot slowdowns over time
- **Error patterns:** Classify recurring failure modes
- **Cost tracking:** If n8n cloud, track execution credits used

### 19.6 Workflow CRUD + Versioning
- **Full lifecycle from KRIA:** Create → Test → Approve → Execute → Deprecate → Delete
- **Version management:** Multiple versions per workflow, rollback support
- **Approval flows:** Require explicit human approval before promotion to Production
- **Audit trail:** Every lifecycle change logged with timestamp + user
- **Diff view:** Show changes between workflow versions

### 19.7 AI-Assisted Workflow Generation
- **Generate from description:** "Create a workflow that fetches Jira tickets every morning"
  → KRIA generates n8n workflow JSON via LLM → deploys to n8n → registers in catalog
- **Repair suggestions:** When a workflow fails, LLM suggests node configuration fixes
- **Optimization hints:** "This workflow could be 2x faster if you parallelize nodes 3 and 4"
- **Security review:** AI-assisted scan of workflow for credential leaks, unsafe operations

### 19.8 Hybrid KRIA + n8n Orchestration
- **KRIA as orchestrator:** Multi-workflow pipelines where KRIA decides which workflow to invoke next
  based on results of the previous one
- **Conditional routing:** "If Jira has > 5 critical tickets, notify Slack; else just log"
- **Fan-out/fan-in:** Invoke multiple workflows in parallel, aggregate results
- **Agentic chaining:** KRIA plans a multi-step goal, selects workflows for each step dynamically

### 19.9 Semantic Workflow Outputs
- **Auto-summarization:** LLM converts raw JSON workflow output into natural language
- **Structured rendering:** Tables for tabular data, code blocks for code, cards for entities
- **Multimodal results:** Render images, charts, PDFs produced by workflows inline in chat
- **Contextual memory:** Store workflow results in KRIA's knowledge base for future reference

### 19.10 Workflow Continuation & Interruption
- **Pause/resume:** Long workflows can be paused and resumed via HITL
- **Branching decisions:** Workflow reaches a decision point → KRIA asks user → sends choice back
- **Error continuation:** On failure, show options: retry this step / skip / abort / use fallback
- **Checkpoint recovery:** If KRIA restarts mid-workflow, resume from last known good state

### 19.11 Permission & Security System
- **Per-workflow permissions:** Mark workflows as requiring explicit approval before each run
- **Rate limits:** Max N invocations per hour per workflow
- **Data classification:** Tag workflows with data sensitivity (PII, financial, internal)
- **Audit compliance:** Every invocation logged with full provenance chain
- **Secret rotation:** Automated HMAC secret rotation with zero-downtime

### 19.12 Workflow Testing & Debugging
- **Dry-run mode:** Execute workflow with mock data, show what WOULD happen
- **Test mode integration:** Use n8n's test webhook URL for debugging without activating
- **Replay:** Re-run a previous execution with same parameters
- **Step-through:** Manual node-by-node execution for debugging
- **Log viewer:** Structured view of per-node input/output/duration/errors

---

## Document End

**This audit represents the complete current-state analysis and future
roadmap for transforming KRIA's n8n integration from "external automation
calling" into "native intelligent workflow capability."**

The infrastructure is sound. The experience needs building.
Priority: Wave 1 (security + resilience) → Wave 2 (chat integration) → Wave 3 (conversational).

*Generated: 2026-05-29*
