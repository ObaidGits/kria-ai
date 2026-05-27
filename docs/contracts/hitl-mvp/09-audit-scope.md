# KRIA Audit Scope

**Document status:** Bounded audit and replay scope  
**Purpose:** Define what KRIA must record and what it must not pretend to replay.

---

## 1. Audit Goal

Audit must answer:

```text
why did KRIA pause,
who/what resolved it,
why was execution allowed or blocked,
and what happened afterward?
```

Audit is not a full workflow replay engine.

---

## 2. Required Audit Events

| Event | Required Fields |
|---|---|
| `ActionProposed` | workflow, attempt, stage, action hash, target hash. |
| `GateEvaluated` | policy, authority, verifier, gate outcome. |
| `DecisionCreated` | decision ID, options, risk, evidence refs. |
| `DecisionResolved` | decision ID, option, actor, version. |
| `DecisionInvalidated` | decision ID, reason. |
| `LeaseAcquired` | lease ID, resource, action hash. |
| `LeaseReleased` | lease ID, reason. |
| `ActionExecuted` | action hash, outcome. |
| `WorkflowPaused` | checkpoint ID, decision ID. |
| `WorkflowResumed` | checkpoint ID, validation result. |

---

## 3. Replay Boundary

Replay may reconstruct:

- decision causality,
- authority order,
- evidence references,
- selected option,
- invalidation reason,
- final action outcome.

Replay must not:

- re-execute tools,
- replay GUI input,
- trigger external callbacks,
- mutate workflow state,
- claim deterministic desktop reproduction.

---

## 4. Redaction

Audit payloads must redact:

- API keys,
- tokens,
- cookies,
- SSH keys,
- env vars with secrets,
- email bodies unless explicitly required,
- browser session details,
- raw screenshots by default.

---

## 5. MVP Tests

- every decision has audit create and resolve/invalidate event,
- audit event contains action and target hash,
- secret-like values are redacted,
- replay reconstruction is read-only,
- audit append failure blocks side-effecting execution.

