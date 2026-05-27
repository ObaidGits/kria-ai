# KRIA MVP Eval Plan

**Document status:** MVP validation plan  
**Purpose:** Define realistic tests for HITL MVP only.

---

## 1. Eval Rule

Evals must test deterministic contracts, not broad cognition claims.

No eval should require:

- full GUI replay,
- semantic scheduler,
- human trust model,
- cognitive pressure model,
- external delegated HITL,
- adaptive autonomy.

---

## 2. Required Eval Groups

| Group | Purpose |
|---|---|
| Action hash evals | Mutation invalidates approval. |
| Target hash evals | Target changes invalidate decisions. |
| Policy evals | Red/Black enforcement cannot be bypassed. |
| Decision lifecycle evals | Pending/resolved/invalidated behavior. |
| Resume evals | Checkpoint revalidation before execution. |
| Lease evals | Prevent GUI/filesystem/VM conflicts. |
| Timeout evals | Timeout behavior does not execute unsafe action. |
| Audit evals | Decision causality is reconstructable. |
| Frontend evals | Action center cannot resolve stale decision. |
| Security evals | External token, redaction, verifier spoof cases. |

---

## 3. Minimum Test Cases

1. User approves command; parameter changes before execution -> reject.
2. User approves local target; target changes to VM -> reject.
3. Black policy action with user approval -> block.
4. Red action timeout -> deny or pause; never execute.
5. Ambiguous target timeout -> recoverable pause.
6. Decision resolved twice -> idempotent or second rejected.
7. Workflow replans after decision -> old decision invalid.
8. GUI typing without foreground lease -> block.
9. Two workflows request GUI write -> one blocked.
10. Filesystem write through symlink -> canonical target enforced.
11. VM reset during active VM command -> lease conflict.
12. Verifier conflict before resume -> pause/reobserve.
13. Missing verifier evidence for risky action -> Unknown.
14. External callback without token -> reject.
15. Audit redacts secret-looking values.
16. Replay cannot execute action.
17. Frontend stale action center item -> backend rejects.
18. Cancel decision does not continue with default.
19. Failed workflow retry gets new attempt ID.
20. Audit append failure blocks execution.

---

## 4. Production Gate

Do not ship MVP until all required eval groups pass in CI and at least these manual workflows pass:

- generated code run locally,
- ambiguous local vs VM command,
- safe file creation,
- destructive file delete blocked/prompted,
- GUI open/type with foreground verification,
- paused decision resolved after delay,
- stale decision rejected with clear UI explanation.

