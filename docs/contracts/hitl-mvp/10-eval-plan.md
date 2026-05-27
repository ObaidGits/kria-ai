# KRIA HITL MVP Eval Plan

**Document status:** Implementation-bound validation plan
**Last updated:** 2026-05-27
**Primary code:** `crates/kria-core/src/agent/collaborative_decision.rs`, `execution_gate.rs`, `resume_executor.rs`, `continuation_reentry.rs`, `resource_lease.rs`, `crates/kria-core/src/safety/audit.rs`

---

## 1. Eval Rule

HITL MVP evals must test deterministic runtime contracts, not broad cognition claims.

No HITL MVP eval should require:

- full GUI replay,
- semantic scheduler,
- human trust model,
- cognitive pressure model,
- external delegated HITL,
- adaptive autonomy,
- preference learning.

---

## 2. Required Eval Groups

| Group | Required coverage |
|---|---|
| Action proposal hash evals | Parameter, target, schema, registry, workflow/attempt/stage mutation invalidates old authority. |
| Decision lifecycle evals | Pending-only resolution, invalid option rejection, denial, cancellation, expiry, JSONL replay. |
| Execution gate evals | Readiness block, preflight block, authority pause, policy block, approval, proceed. |
| Resume evals | Context validation, hash revalidation, tool-version validation, unsupported tool rejection, final gate after leases. |
| Lease evals | GUI/input conflict, filesystem path write conflict, browser profile conflict, VM target conflict. |
| Continuation evals | Prior-action verification, duplicate continuation, no whole-workflow autoplay. |
| Audit evals | SQLite hash-chain verification, query/stats behavior, result update, redacted execution output. |
| Integration evals | `PolicyToolExecutor` returns `DECISION_PAUSED`, `HITL_DENIED`, `RESOURCE_LEASE_DENIED`, or executes only after approval/gate/lease. |

---

## 3. Minimum Test Cases

1. User approves action; parameters change before resolution -> reject.
2. User approves action; target changes before resolution -> reject.
3. Decision version changes before resolution -> reject.
4. Expired decision is invalidated and cannot execute.
5. Invalid option ID cannot resolve a decision.
6. `Black` policy action blocks even with approval wording.
7. Authority ambiguity creates target-selection decision and pauses.
8. Policy approval creates approval decision and calls HITL.
9. HITL timeout expires durable decision and does not execute.
10. HITL denial resolves/records denial and does not execute.
11. Safe policy action logs auto-execution and proceeds.
12. GUI input action declares foreground and input leases.
13. Two workflows conflict on GUI input lease.
14. Filesystem write action declares filesystem path write lease.
15. Browser navigation/search declares browser profile lease.
16. VM reset/snapshot command declares exclusive VM target lease.
17. Resume rejects missing persisted action proposal.
18. Resume rejects action hash mismatch.
19. Resume rejects target hash mismatch.
20. Resume rejects session/workspace mismatch when bound.
21. Resume rejects tool schema/registry version mismatch.
22. Resume rejects unsupported tool capability.
23. Resume gate risk increase blocks execution.
24. Lease conflict during resume returns blocked-by-lease result.
25. Resume executes exactly one persisted tool action.
26. Duplicate execution claim is rejected.
27. Continuation verifies prior executed action before action-progress update.
28. Continuation duplicate is rejected.
29. Continuation with failed verification does not expose next safe step.
30. SQLite audit hash-chain verification detects tampering.
31. DecisionStore JSONL replay reconstructs decisions/executions/continuations.
32. Redacted tool result does not persist raw sensitive output.

---

## 4. Suggested Test Commands

Use focused test runs during development:

```bash
cargo test -p kria-core agent::collaborative_decision
cargo test -p kria-core agent::execution_gate
cargo test -p kria-core agent::resource_lease
cargo test -p kria-core agent::resume_executor
cargo test -p kria-core agent::continuation_reentry
cargo test -p kria-core safety::audit
```

If module-path filtering is unreliable, use file or test-name filters from `rg -n "fn .*decision|fn .*lease|fn .*resume|fn .*continuation|fn .*audit" crates/kria-core/src crates/kria-core/tests`.

---

## 5. Production Gate

Do not call HITL production-ready until:

- all eval groups above pass in CI,
- `PolicyToolExecutor` live path is covered for block/pause/approval/proceed/lease-denied behavior,
- action-center resume executes only deterministic local tools,
- stale action/target/version decisions are rejected from backend APIs,
- audit and decision logs can reconstruct the decision chain,
- known non-guarantees in `08-safety-invariants.md` are either accepted as limitations or closed with code and tests.
