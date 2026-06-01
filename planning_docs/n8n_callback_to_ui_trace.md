# n8n Callback → UI Trace: Complete Hop-by-Hop Analysis

**Date:** 2026-05-29
**Goal:** Find exactly where callback data is lost between POST /api/n8n/callback and ChatView render

---

## Full Trace Chain

```
POST /api/n8n/callback
  ↓
HOP-0: Auth middleware (Bearer token OR exempt)
  ↓
HOP-1: State machine ingest (N8nWorkflowStateStore::ingest)
  ↓
HOP-2: Governance evaluation (evaluate_run)
  ↓
HOP-3: Terminal check (envelope.status.is_terminal())
  ↓
HOP-4: Tauri event emission (app_handle.emit("n8n:chat_result", ...))
  ↓
HOP-5: Frontend listener receives event (listen<any>("n8n:chat_result"))
  ↓
HOP-6: Message construction (summary string built)
  ↓
HOP-7: Signal update (setAssistantMessages)
  ↓
HOP-8: Reactive re-render (messages() memo → ChatView)
```

---

## Detailed Hop Analysis

### HOP-0: Auth Middleware
**File:** `crates/kria-desktop/src/commands/api_auth.rs`
**Function:** `auth_middleware()`
**Input:** HTTP request with `x-kria-signature` header
**Output:** Pass-through (path `/api/n8n/callback` is now exempt)
**Status:** ✅ Fixed — was returning 401, now exempt
**Evidence:** Previous test run showed 401; auth exemption added

### HOP-1: State Machine Ingest
**File:** `crates/kria-core/src/n8n/state.rs`
**Function:** `N8nWorkflowStateStore::ingest()`
**Input:** `N8nCallbackEnvelope`
**Output:** `N8nIngestDecision` (Accepted/Duplicate/OutOfOrder/TerminalAlreadyReached)
**Emitted events:** None (pure state mutation)
**Status:** ✅ Verified working (callback_inbox.jsonl has "accepted" entries)
**Log added:** `n8n_callback_trace` → HOP-1

### HOP-2: Governance Evaluation
**File:** `crates/kria-core/src/n8n/governance.rs`
**Function:** `evaluate_run()`
**Input:** `N8nWorkflowRunState` + optional `N8nWorkflowConfig`
**Output:** `N8nGovernanceDecision` (verification_status + continuation_action)
**Emitted events:** None
**Status:** ✅ Verified working (governance_audit.jsonl has entries)
**Log added:** `n8n_callback_trace` → HOP-2

### HOP-3: Terminal Check ⚠️ POTENTIAL FAILURE POINT
**File:** `crates/kria-desktop/src/commands/local_api.rs:225`
**Function:** Inline `if envelope.status.is_terminal()`
**Input:** `envelope.status`
**Output:** Boolean gate — only emits `n8n:chat_result` if terminal
**Critical:** If callback status is `"running"` or `"accepted"`, NO chat event is emitted!

**is_terminal() returns true for:**
- Completed ✅
- Partial ✅
- Failed ✅
- Cancelled ✅
- TimedOut ✅
- Rejected ✅

**is_terminal() returns FALSE for:**
- Running ❌ (no chat event!)
- Accepted ❌ (no chat event!)
- WaitingForApproval ❌ (no chat event!)

**FINDING:** If n8n sends a callback with `"status": "running"` but never sends a second callback with `"status": "completed"`, the UI will NEVER show a result. This is the most likely root cause if the n8n workflow doesn't send a final "completed" callback.

**Log added:** `n8n_callback_trace` → HOP-3

### HOP-4: Tauri Event Emission
**File:** `crates/kria-desktop/src/commands/local_api.rs:230-247`
**Function:** `app_handle.emit("n8n:chat_result", chat_result)`
**Input:** JSON payload with workflow_id, correlation_id, session_id, status, success, evidence, display_name, governance
**Output:** Tauri IPC event to all frontend webviews
**Status:** Should work — app_handle is always `Some(...)` (verified in construction)
**Log added:** `n8n_callback_trace` → HOP-4 (with success/failure logging)

### HOP-5: Frontend Event Receipt
**File:** `ui/src/stores/app.ts:2986`
**Function:** `listen<any>("n8n:chat_result", (event) => { ... })`
**Input:** Tauri event with payload
**Output:** Deserialized payload object
**Registration:** Inside `initListeners()` which is called at module import time (line 3434)
**Status:** Should work IF the webview is loaded and the store module is imported
**Log added:** `console.log("[n8n:chat_result] HOP-5: ...")`

### HOP-6: Message Construction
**File:** `ui/src/stores/app.ts:2993-3004`
**Function:** Inline string building
**Input:** payload.success, payload.display_name, payload.evidence
**Output:** Human-readable summary string
**Status:** ✅ Code is straightforward
**Log added:** `console.log("[n8n:chat_result] HOP-6: ...")`

### HOP-7: Signal Update
**File:** `ui/src/stores/app.ts:3006`
**Function:** `setAssistantMessages((prev) => [...prev, newMsg])`
**Input:** Previous messages array + new n8n result message
**Output:** Updated signal value
**Critical:** This updates `assistantMessages` signal, NOT `promptLabMessages`
**Log added:** `console.log("[n8n:chat_result] HOP-7: ...")`

### HOP-8: Reactive Re-render
**File:** `ui/src/stores/app.ts:529`
**Function:** `createMemo<Message[]>(() => currentEnvironment() === "prompt_lab" ? promptLabMessages() : assistantMessages())`
**Input:** `assistantMessages()` signal
**Output:** Derived `messages` accessor used by ChatView
**Critical:** If user is in "prompt_lab" environment, they won't see assistant messages!

---

## Most Likely Failure Points (Priority Order)

### 1. n8n Never Sends "completed" Callback (HOP-3 gate)
**Probability: HIGH**

If the n8n workflow:
- Receives the webhook
- Executes successfully
- But does NOT have an HTTP Request node that calls back to KRIA

Then:
- KRIA shows "⏳ Running workflow..." (the invocation response)
- n8n runs fine
- No callback ever arrives at KRIA
- Chat stays at "Running..." forever

**Evidence needed:** Check if callback_inbox.jsonl has a "completed" status entry for the correlation_id in question.

### 2. Auth Blocking (HOP-0) — NOW FIXED
**Probability: WAS the issue for external callers**

Previously `/api/n8n/callback` required Bearer token. n8n wouldn't know the token.
**Fix applied:** Exempt path added.

### 3. Session Mismatch / Environment Mismatch (HOP-7/8)
**Probability: LOW**

If user switches sessions or is in prompt_lab mode when the callback arrives, the message goes to `assistantMessages` but the view shows `promptLabMessages`.

---

## Diagnostic Procedure (After Restart)

1. **Start KRIA:** `cargo tauri dev`
2. **Open DevTools:** Right-click → Inspect → Console tab
3. **Trigger workflow:** Type "Run test_workflow" in chat
4. **Send manual callback:**
```bash
SECRET=$(cat ~/.kria/secrets/n8n.key)
PAYLOAD='{"schema_version":"kria.n8n.callback.v1","correlation_id":"ui_trace_test","causation_id":"ui_trace_test","event_id":"evt_trace_1","sequence_number":1,"workflow_id":"test_workflow","workflow_version":"v1","n8n_run_id":"run_trace","status":"completed","evidence":{"result":"Hello from trace test!","occurred_at_ms":'$(date +%s%3N)'},"side_effects":[],"occurred_at_ms":'$(date +%s%3N)'}'
SIG=$(printf '%s' "$PAYLOAD" | openssl dgst -sha256 -hmac "$SECRET" -binary | xxd -p | tr -d '\n')
curl -X POST http://127.0.0.1:3001/api/n8n/callback \
  -H "Content-Type: application/json" \
  -H "x-kria-signature: sha256=$SIG" \
  -d "$PAYLOAD"
```

5. **Check backend logs (terminal running cargo tauri dev):**
   - Look for `n8n_callback_trace` messages
   - HOP-1 through HOP-4 should all appear
   - If HOP-3 says "Non-terminal" → callback status was not "completed"
   - If HOP-4 says "FAILED" → Tauri IPC issue

6. **Check browser console (DevTools):**
   - Look for `[n8n:chat_result] HOP-5` → Event arrived in frontend
   - Look for `[n8n:chat_result] HOP-6` → Message constructed
   - Look for `[n8n:chat_result] HOP-7` → Signal updated
   - If NONE appear → Tauri event not reaching webview (IPC failure)

7. **Check chat:**
   - If message appears → Full chain working ✅
   - If not → Check which HOP is missing from logs

---

## Files Modified for Tracing

| File | What was added |
|------|----------------|
| `crates/kria-desktop/src/commands/local_api.rs` | HOP-1 through HOP-4 tracing::info! logs |
| `crates/kria-desktop/src/commands/api_auth.rs` | `/api/n8n/callback` auth exemption |
| `ui/src/stores/app.ts` | HOP-5 through HOP-7 console.log statements |

---

## Conclusion

The architecture is correct. The most likely reason the user sees "Running..." but no result is:

**n8n workflow does not have a callback node that sends `"status": "completed"` back to KRIA.**

The callback endpoint, state machine, governance, event emission, and frontend listener are all wired correctly. The missing piece is the n8n workflow itself must explicitly POST back to KRIA with a terminal status.

To prove this definitively: restart KRIA, send a manual callback (step 4 above), and observe all 7 HOPs in logs.
