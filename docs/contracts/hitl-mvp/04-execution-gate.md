# KRIA Execution Gate Spec

**Document status:** MVP gate logic  
**Purpose:** Define deterministic pre-execution decision logic.

---

## 1. Gate Position

The execution gate runs after planning and before any side effect:

```text
ActionProposal
  -> ExecutionGate
  -> Proceed / Block / PauseForDecision / NeedReobserve / NeedLease
```

The gate does not execute tools.

---

## 2. Inputs

- `ActionProposal`,
- policy decision,
- target authority result,
- verifier evidence if required,
- current workflow checkpoint,
- current leases,
- decision history for same workflow attempt.

---

## 3. Deterministic Order

```text
1. validate action proposal
2. apply hard policy
3. classify risk
4. bind/verify target
5. check verifier requirements
6. check rollbackability
7. check required leases
8. choose outcome
```

No LLM call is allowed inside the gate.

---

## 4. Outcomes

| Outcome | Meaning |
|---|---|
| `Proceed` | Safe enough and all required leases can be acquired. |
| `Block` | Hard policy or impossible safety condition. |
| `PauseForDecision` | Human answer changes safety, meaning, or recovery. |
| `NeedReobserve` | Read-only observation may resolve uncertainty. |
| `NeedLease` | Action may proceed only after resource ownership. |

---

## 5. Pause Rules

Create `PauseForDecision` only when:

- destructive approval is required,
- target/scope ambiguity changes meaning,
- verifier conflict blocks safe execution,
- auth/user-only action is required,
- recovery cannot proceed safely without user input.

Do not pause for internal planner insecurity.

---

## 6. Block Rules

Block when:

- hard policy says Black,
- target identity is unknowable for side-effecting action,
- action requires privilege KRIA does not have,
- verifier proves action precondition false,
- rollbackability is Unknown for forbidden destructive scope.

---

## 7. MVP Tests

- Black policy always returns `Block`,
- low-risk reversible action returns `Proceed`,
- missing target for write returns `PauseForDecision` or `Block`,
- verifier conflict returns `PauseForDecision`,
- unavailable lease returns `NeedLease`,
- LLM recommendation cannot affect gate outcome.

