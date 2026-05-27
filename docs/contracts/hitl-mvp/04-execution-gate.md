# KRIA Execution Gate Contract

**Document status:** Implementation-bound gate contract
**Last updated:** 2026-05-27
**Primary code:** `crates/kria-core/src/agent/execution_gate.rs`, `gui_wiring.rs`, `resume_executor.rs`

---

## 1. Gate Position

The live execution gate runs immediately before tool side effects:

```text
tool action + params
  -> readiness check
  -> preflight
  -> execution authority
  -> ActionProposal
  -> resource requirement declaration
  -> policy
  -> ExecutionGateOutcome
```

The gate does not execute tools and does not acquire leases. It returns resource requirements for the caller to acquire before side effects.

---

## 2. Inputs

`ExecutionGateInput` currently includes:

- `session_id`,
- `user_text`,
- tool `action`,
- JSON `params`,
- `destructive_hint`.

The gate owns no LLM calls and no GUI actions.

---

## 3. Deterministic Live Order

Current `ExecutionGate::evaluate` order:

```text
1. check tool readiness through gui_services::check_action_readiness
2. run tool preflight through preflight::run_preflight
3. infer execution target from user text and action
4. validate execution authority with params
5. build ActionProposal
6. declare ResourceRequirement values
7. block or pause on execution-authority result
8. evaluate policy for authorized actions
9. block, require approval, or proceed
```

No model output may alter this order.

---

## 4. Live Outcomes

| Outcome | Meaning |
|---|---|
| `Proceed` | Action is authorized by execution authority and policy. Caller still must acquire returned resource requirements before execution. |
| `Block` | Readiness, preflight, authority, policy, or decision-store failure forbids execution. |
| `PauseForDecision` | Execution authority needs bounded user clarification, usually target selection. |
| `RequiresApproval` | Policy requires HITL approval before execution. |

`NeedLease` is not a live gate outcome. Lease conflict is detected when the caller acquires `resource_requirements`.

---

## 5. PolicyToolExecutor Behavior

The GUI live tool path in `PolicyToolExecutor` must:

- call `ExecutionGate::evaluate`,
- log policy blocks through `AuditLogger`,
- return `DECISION_PAUSED` with decision ID for `PauseForDecision`,
- request HITL approval for `RequiresApproval`,
- resolve or expire durable approval decisions,
- log approval, denial, timeout, or auto-execution,
- acquire `ResourceLeaseManager` requirements before side effects,
- return `RESOURCE_LEASE_DENIED` on lease conflict,
- execute the tool only after all gates pass.

---

## 6. Resume Gate

`ExecutionGate::revalidate_resume` must run before action-center execution of a resolved decision.

It returns `Ready` only if:

- persisted action proposal exists,
- target hash still matches,
- action hash still matches,
- tool readiness passes,
- preflight passes,
- policy does not block,
- risk did not increase,
- required approval is already valid.

Any other resume outcome invalidates or blocks execution.

---

## 7. Required Tests

- readiness failure returns `Block`,
- preflight failure returns `Block`,
- authority ambiguity returns `PauseForDecision`,
- policy block returns `Block`,
- policy approval returns `RequiresApproval`,
- safe authorized action returns `Proceed`,
- resource requirements are declared for GUI input, filesystem writes, browser profile actions, and VM operations,
- resume rejects missing proposal, hash changes, risk increase, policy block, and missing approval,
- LLM wording cannot affect gate outcome.
