# KRIA HITL MVP Boundary

**Document status:** Scope freeze  
**Purpose:** Define the smallest production-survivable HITL implementation.  
**Rule:** If a feature is not listed as in-scope here, it is out of MVP.

---

## 1. MVP Goal

KRIA MVP HITL exists to do one thing reliably:

```text
pause unsafe or underdetermined workflow execution,
ask a bounded decision,
persist enough state to resume safely,
reject stale answers,
and audit the result.
```

MVP is not an AI OS, scheduler, replay engine, trust system, or adaptive autonomy system.

---

## 2. In Scope

| Capability | MVP Requirement |
|---|---|
| Durable decision | Persist `InteractionDecision` with status and version. |
| Immutable action intent | Bind every decision to `action_hash` and `target_hash`. |
| Deterministic gate | Return only `Proceed`, `Block`, `PauseForDecision`, `NeedReobserve`, or `NeedLease`. |
| Workflow pause | `StageExecutor` can return `PausedForDecision`. |
| Checkpoint binding | A paused workflow stores decision ID and checkpoint generation. |
| Decision resolution | Backend command resolves decision by ID, option ID, and version. |
| Revalidation | Backend rejects stale decision before execution. |
| Minimal leases | GUI foreground, keyboard/mouse, filesystem write, VM destructive, external owner token. |
| Minimal action center | List pending decisions, show details, resolve/pause/abort. |
| Audit | Record create, resolve, invalidate, execute, abort. |
| Tests | Machine-testable invariants and stale-resolution evals. |

---

## 3. Explicitly Out Of MVP

Do not implement these in MVP:

- semantic scheduler,
- cognitive pressure scoring,
- human trust model,
- adaptive urgency AI,
- autonomy drift scoring,
- substrate trust decay,
- planner outcome scoring,
- global causality graph,
- broad event sourcing,
- deterministic GUI replay,
- external delegated HITL bridging,
- mobile/remote/voice approval,
- LLM-generated recovery sessions,
- metrics-driven safety tuning,
- preference learning from HITL answers,
- multi-user collaboration model,
- full replay UI,
- GPU/model/verifier leases in HITL layer.

These may be reconsidered only after MVP invariants pass in real workflows.

---

## 4. MVP Authority Order

Runtime authority is fixed:

```text
HardPolicyBlock
PolicyRisk
VerifierTruth
ExecutionAuthority
WorkflowSemantics
CurrentUserInstruction
ExplicitScopedPreference
PlannerSuggestion
LLMWording
```

No lower authority may reduce risk, override verifier truth, or bypass policy.

---

## 5. MVP Success Criteria

MVP is complete only when:

- stale decisions are rejected,
- action mutation invalidates approval,
- target mutation invalidates approval,
- Red/Black policy cannot be bypassed,
- timeout ambiguity pauses recoverably,
- workflow checkpoint survives restart,
- action center cannot resolve invalid decisions,
- GUI typing requires valid foreground lease,
- destructive VM operations require exclusive lease,
- audit can explain why a decision existed and what happened.

