# KRIA Decision Invalidation Spec

**Document status:** MVP revalidation rules  
**Purpose:** Define when a pending or resolved decision can no longer authorize execution.

---

## 1. Core Rule

A decision is valid only for the exact action and target it was created for.

```text
decision.action_hash == current.action_hash
decision.target_hash == current.target_hash
decision.version == submitted_version
```

If any check fails, reject the resolution and rebuild the decision.

---

## 2. Invalidation Triggers

| Trigger | MVP Behavior |
|---|---|
| Action hash changed | Invalidate. |
| Target hash changed | Invalidate. |
| Risk increased | Invalidate. |
| Workflow attempt changed | Invalidate. |
| Stage checkpoint changed | Invalidate. |
| Evidence expired | Invalidate or reobserve. |
| Required lease unavailable | Reject resume; retry lease or pause. |
| Policy version introduces stricter rule | Invalidate. |
| User issues conflicting instruction | Invalidate and replan. |
| External owner token expired | Invalidate. |

---

## 3. Resolution Algorithm

```text
load decision
  -> check status pending
  -> check version
  -> load workflow checkpoint
  -> compare workflow_id / attempt_id / stage_id
  -> rebuild current ActionProposal
  -> compare action_hash / target_hash
  -> recompute risk
  -> check evidence freshness
  -> check lease availability
  -> resolve or invalidate
```

---

## 4. Timeout Semantics

| Decision Type | Timeout |
|---|---|
| Destructive approval | Deny or pause safely. |
| Target ambiguity | Remain paused. |
| Recovery choice | Remain paused. |
| Low-risk reversible default | Proceed only if default was declared at creation. |
| Auth required | Remain paused. |

Timeout must never silently execute a risky action.

---

## 5. MVP Tests

- stale decision version rejected,
- target change rejected,
- risk increase rejected,
- expired evidence rejected,
- workflow replan rejects old decision,
- timeout destructive action does not proceed,
- default execution only happens when declared safe at decision creation.

