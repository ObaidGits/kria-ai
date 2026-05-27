# KRIA HITL Safety Invariants

**Document status:** Implementation-bound safety contract
**Last updated:** 2026-05-27
**Primary code:** `crates/kria-core/src/agent/execution_gate.rs`, `collaborative_decision.rs`, `resume_executor.rs`, `resource_lease.rs`, `crates/kria-core/src/safety/policy.rs`, `audit.rs`

---

## 1. Policy Invariants

1. Readiness failure blocks execution.
2. Preflight failure blocks execution.
3. Execution-authority block blocks execution.
4. Policy block blocks execution.
5. Policy-required approval cannot execute until HITL approval is received.
6. HITL denial or timeout returns failure and does not execute the tool.
7. User, planner, or LLM wording cannot reduce `RiskLevel`.

---

## 2. Decision Invariants

8. Decision resolution with expected version rejects mismatches.
9. Hash-sensitive resolution rejects expected `action_hash` mismatch.
10. Hash-sensitive resolution rejects expected `target_hash` mismatch.
11. Non-pending decisions cannot be resolved.
12. Expired decisions cannot be resolved.
13. Invalid option IDs cannot resolve decisions.
14. Durable approval decisions store the `ActionProposal` they authorize.

---

## 3. Resume Invariants

15. Resume requires a `Resolved` decision.
16. Resume requires a persisted `ActionProposal`.
17. Resume rejects stored proposal hash mismatch.
18. Resume rejects session/workspace mismatch when bound.
19. Resume rejects tool schema version change.
20. Resume rejects tool registry version change.
21. Resume rejects unsupported tool resume capability.
22. Resume reruns the execution gate before execution.
23. Resume reruns the execution gate again after leases are acquired.
24. Resume executes exactly one persisted tool action.

---

## 4. Lease Invariants

25. Declared resource requirements must be acquired before side effects.
26. Lease conflicts block execution.
27. GUI input tools declare both foreground and keyboard/mouse requirements.
28. Filesystem mutation tools declare filesystem path requirements.
29. Browser navigation/search tools declare browser profile requirements.
30. VM operation tools declare VM target requirements.

---

## 5. Continuation Invariants

31. Continuation re-entry verifies the prior action before advancing action-level progress.
32. Continuation re-entry does not replay the whole workflow.
33. Continuation duplicate claims are rejected.
34. Failed post-decision verification prevents next-step readiness.
35. Unknown-after-crash execution state is not treated as success.

---

## 6. Audit And Evidence Invariants

36. SQLite audit rows are hash-chained.
37. DecisionStore JSONL events are replayable into current decision/execution/continuation state.
38. Redacted tool results are stored in decision execution records.
39. Human approval is authorization for an exact action, not verifier truth about final state.

---

## 7. Current Non-Guarantees

These are not guaranteed by the current implementation and must not be claimed as production-complete:

- `AuditLogger::log` is best-effort and does not currently return an error to block side effects.
- `ResourceLeaseManager` uses in-memory leases; active leases are not recovered after process restart.
- Filesystem path canonicalization is not performed by the lease manager itself.
- GUI foreground lease ownership is not equivalent to content verification.
- A resolved approval does not prove the tool outcome; verifier/runtime evidence must still prove outcome separately.

---

## 8. Test Requirement

Every invariant listed as a current guarantee must have automated coverage before HITL is considered production-ready. Every non-guarantee above must either remain documented or be closed by code and tests.
