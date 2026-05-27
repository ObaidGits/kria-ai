# KRIA Decision Invalidation Contract

**Document status:** Implementation-bound revalidation contract
**Last updated:** 2026-05-27
**Primary code:** `crates/kria-core/src/agent/collaborative_decision.rs`, `execution_gate.rs`, `resume_executor.rs`, `continuation_reentry.rs`

---

## 1. Core Rule

A decision can authorize only the exact action and target it was created for.

```text
decision.version == submitted_expected_version
decision.action_hash == expected_action_hash
decision.target_hash == expected_target_hash
persisted ActionProposal hashes still recompute to the same values
```

If any check fails, execution must not start.

---

## 2. Resolution-Time Checks

`DecisionStore::resolve_with_context`, `deny_with_context`, and `cancel_with_context` must reject:

| Condition | Store behavior |
|---|---|
| Unknown decision ID | Return `Ok(None)`. |
| Non-`Pending` decision | Return `DecisionStoreError::NotPending`. |
| Expired decision | Mark invalidated and return `DecisionExpired`. |
| Version mismatch | Return `VersionMismatch`. |
| Expected action hash mismatch | Return `ActionHashMismatch`. |
| Expected target hash mismatch | Return `TargetHashMismatch`. |
| Invalid option ID | Return `InvalidOption`. |

`resolve_with_version` checks only version and option. Hash-sensitive callers must use `DecisionResolutionContext`.

---

## 3. Resume-Time Checks

`DecisionStore::validate_resume_context` and `ResumeExecutor` must reject:

- decision is not `Resolved`,
- decision expired before resume,
- expected version mismatch,
- expected action hash mismatch,
- expected target hash mismatch,
- missing persisted `ActionProposal`,
- stored proposal hash mismatch,
- session changed when the proposal is session-bound,
- workspace changed when the proposal is workspace-bound,
- current tool schema version differs from proposal,
- current tool registry version differs from proposal,
- tool does not support deterministic local resume,
- tool handler is missing.

---

## 4. Gate Revalidation Checks

`ExecutionGate::revalidate_resume` must reject or pause when:

- persisted action proposal is missing,
- recomputed target hash differs,
- recomputed action hash differs,
- tool readiness fails,
- preflight fails,
- policy blocks,
- risk level increased since the decision,
- approval is required but the decision is not an approved `Approval`.

Only `ResumeGateOutcome::Ready` can execute.

---

## 5. Timeout And Expiry

Current decisions receive a default expiry. Expired decisions:

- cannot be resolved,
- cannot be resumed,
- are marked invalidated when encountered during resolution/resume validation.

Timeout from the live HITL gateway is handled as approval failure: the durable decision is expired and the tool returns `HITL_DENIED`.

---

## 6. Continuation Invalidation

`ContinuationReentryService` must reject continuation when:

- the decision is unknown,
- the decision/context hash checks fail,
- the action was not executed,
- execution state is unknown after crash,
- checkpoint binding is unsupported,
- tool versions changed,
- post-decision verification fails,
- the continuation was already claimed.

Continuation re-entry verifies one executed decision-bound action and does not authorize unrelated next actions.

---

## 7. Required Tests

- stale decision version rejected,
- stale action hash rejected at resolution and resume,
- stale target hash rejected at resolution and resume,
- expired decision invalidates,
- risk increase blocks resume,
- missing proposal blocks resume,
- schema or registry version change blocks resume,
- unsupported tool cannot resume,
- continuation duplicate is rejected,
- continuation verification failure does not advance workflow state.
