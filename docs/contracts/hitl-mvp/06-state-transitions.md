# KRIA State Transitions

**Document status:** MVP state machine  
**Purpose:** Keep workflow and decision transitions deterministic and small.

---

## 1. Workflow States

Only these workflow states are allowed in MVP:

```text
Running
PausedForDecision
Resuming
Completed
Failed
Aborted
```

Do not add `Deferred`, `Expired`, `ReGrounding`, `ReVerifying`, or scheduler substates in MVP. Those are events or actions, not workflow states.

---

## 2. Workflow Transitions

```text
Running -> PausedForDecision
Running -> Completed
Running -> Failed
Running -> Aborted

PausedForDecision -> Resuming
PausedForDecision -> Aborted

Resuming -> Running
Resuming -> Failed
Resuming -> Aborted

Completed -> terminal
Failed -> terminal unless explicit retry creates new attempt_id
Aborted -> terminal
```

No automatic transition from `PausedForDecision` to `Running` is allowed.

---

## 3. Decision States

```text
Pending
Resolved
Invalidated
Denied
Expired
Cancelled
```

Only `Pending` decisions can be resolved.

---

## 4. Resume Flow

```text
resolve decision
  -> workflow enters Resuming
  -> rebuild ActionProposal
  -> revalidate hashes/risk/evidence/lease
  -> Running or Failed/Aborted
```

If revalidation fails, workflow returns to `PausedForDecision` with a new or invalidated decision.

---

## 5. Terminal Rules

- `Completed` cannot resume.
- `Aborted` cannot resume.
- `Failed` can retry only by creating a new `attempt_id`.
- Old decisions cannot move a terminal workflow.

---

## 6. MVP Tests

- cannot resolve non-pending decision,
- cannot resume terminal workflow,
- failed retry creates new attempt ID,
- pause never auto-runs after timeout,
- invalidation returns to paused state with clear reason.

