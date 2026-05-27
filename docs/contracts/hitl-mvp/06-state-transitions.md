# KRIA HITL State Transitions

**Document status:** Implementation-bound state contract
**Last updated:** 2026-05-27
**Primary code:** `crates/kria-core/src/agent/collaborative_decision.rs`, `resume_executor.rs`, `continuation_reentry.rs`, `execution_transparency/mod.rs`

---

## 1. Decision States

Persisted decisions use:

```text
Pending
Resolved
Deferred
Expired
Invalidated
Denied
Cancelled
```

Only `Pending` decisions may transition through resolution helpers. `Resolved` decisions may be considered for one-step resume. Expired or invalidated decisions cannot authorize execution.

---

## 2. Decision Transitions

Allowed transition shape:

```text
Pending -> Resolved
Pending -> Denied
Pending -> Cancelled
Pending -> Expired
Pending -> Invalidated
Resolved -> Invalidated
Resolved -> execution claim
```

`Deferred` is a persisted status value, but it must not be treated as permission to execute.

---

## 3. Execution States

Decision-bound execution records use:

```text
NotStarted
Preparing
BlockedByLease
Executing
Executed
Failed
Cancelled
Invalidated
UnknownAfterCrash
```

Terminal execution states are:

```text
Executed
Failed
Cancelled
Invalidated
UnknownAfterCrash
```

Duplicate execution claims are rejected unless the existing state is `BlockedByLease`.

---

## 4. Resume Execution Flow

Current `ResumeExecutor` flow:

```text
refresh decision store
  -> validate resolved decision context
  -> load persisted ActionProposal
  -> validate action/session/workspace context
  -> claim execution
  -> validate tool schema and registry version
  -> require deterministic local resume support
  -> collect grounding facts
  -> run resume gate
  -> acquire leases
  -> run final resume gate
  -> mark Executing
  -> execute exactly one tool action
  -> mark Executed / Failed / Cancelled
```

No whole-workflow replay is allowed in this path.

---

## 5. Continuation States

Continuation records may use:

```text
NotStarted
VerifyingPriorAction
VerifiedPriorAction
AdvancingActionState
ReadyForNextSafeStep
ExecutingNextSafeStep
PausedAgain
CompletedOneStep
Failed
Cancelled
UnknownAfterCrash
Invalidated
```

`ContinuationReentryService` may report a preview of the next safe step, but it must not execute that step automatically.

---

## 6. Trace-Level Workflow States

Execution transparency may report:

```text
Running
PausedForDecision
Completed
Failed
Aborted
```

These are trace/user-facing states. They do not replace persisted decision/execution/continuation state.

---

## 7. Required Tests

- cannot resolve non-pending decision,
- cannot execute unresolved decision,
- duplicate execution claim is rejected,
- blocked-by-lease execution can be retried through a new claim path,
- execution cancellation records `Cancelled`,
- crash-unknown state is terminal for resume,
- continuation duplicate is rejected,
- continuation does not auto-run the next safe step.
