# KRIA HITL Audit Scope

**Document status:** Implementation-bound audit scope
**Last updated:** 2026-05-27
**Primary code:** `crates/kria-core/src/safety/audit.rs`, `crates/kria-core/src/agent/collaborative_decision.rs`, `resume_executor.rs`, `continuation_reentry.rs`

---

## 1. Audit Goal

Audit must answer:

```text
why did KRIA pause or ask,
what exact action and target were involved,
who/what resolved the decision,
why execution was allowed or blocked,
what one-step execution did,
and whether continuation verification advanced action-level progress.
```

Audit is not full GUI replay and must not claim deterministic desktop reproduction.

---

## 2. Audit Surfaces

KRIA currently has two audit surfaces:

| Surface | Storage | Purpose |
|---|---|---|
| `AuditLogger` | SQLite `audit_log` table | Policy/HITL action decisions with hash-chain integrity. |
| `DecisionStore` | JSONL `decision_events.jsonl` | Decision, execution, continuation, and evidence lifecycle replay. |

Both are useful. They do not replace each other.

---

## 3. SQLite AuditLogger

`AuditLogger` records:

- timestamp,
- session ID,
- action,
- parameters JSON,
- `RiskLevel`,
- decision: `AUTO_EXECUTED`, `APPROVED`, `DENIED`, `BLOCKED`, `TIMEOUT`,
- decider: `POLICY`, `USER_VOICE`, `USER_GUI`, `TIMEOUT`, `HARDCODED`, `VERIFICATION`,
- optional execution result fields,
- previous row hash,
- row hash.

It supports query, stats, result update, and hash-chain verification.

Current limitation: `log` returns `()` and ignores insertion errors. Do not document audit append failure as a side-effect blocker until the implementation changes.

---

## 4. DecisionStore Event Log

`DecisionStore` records replayable JSONL events including:

- `DecisionCreated`,
- `DecisionResolved`,
- `DecisionExpired`,
- `DecisionInvalidated`,
- `DecisionDenied`,
- `DecisionCancelled`,
- decision execution claim/start/completion/failure/cancel/invalidated/unknown/block-by-lease,
- continuation claim/verification/action-progress/next-step/pause/failure/cancel/unknown/invalidated,
- `EvidenceObserved`.

Each event includes:

- event ID,
- decision ID,
- workflow ID,
- optional stage ID,
- event type,
- actor,
- authority,
- payload,
- created timestamp,
- policy version,
- runtime version.

---

## 5. Replay Boundary

Replay may reconstruct:

- decision state,
- execution state,
- continuation state,
- evidence summaries,
- selected option,
- invalidation reason,
- one-step action outcome.

Replay must not:

- re-execute tools,
- replay GUI input,
- trigger external callbacks,
- mutate workflow state,
- infer verifier truth not present in evidence,
- claim deterministic desktop reproduction.

---

## 6. Redaction Boundary

Decision execution records store redacted tool results. Audit and decision payloads must avoid secrets where practical.

Required redaction targets:

- API keys,
- tokens,
- cookies,
- SSH keys,
- secret-looking env vars,
- raw browser session details,
- raw screenshots unless explicitly required and scoped.

---

## 7. Required Tests

- SQLite audit hash-chain verification detects tampering,
- decision JSONL replay reconstructs decisions,
- stale resolution emits invalidation/error,
- execution claim emits execution lifecycle events,
- lease conflict records blocked execution state,
- continuation verification emits continuation/evidence events,
- replay helpers are read-only,
- redacted tool result does not store raw sensitive output.
