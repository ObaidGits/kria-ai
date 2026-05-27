# KRIA Safety Invariants

**Document status:** Machine-testable safety guarantees  
**Purpose:** Define non-negotiable runtime properties for HITL MVP.

---

## 1. Policy Invariants

1. Black policy cannot be overridden.
2. Red policy cannot proceed without explicit valid decision.
3. User preference cannot reduce risk class.
4. LLM output cannot reduce risk class.
5. Unknown rollbackability is treated as high risk.

---

## 2. Decision Invariants

6. Decision resolution requires matching decision version.
7. Decision resolution requires matching `action_hash`.
8. Decision resolution requires matching `target_hash`.
9. Non-pending decisions cannot be resolved.
10. Expired destructive decisions cannot execute.

---

## 3. Verifier Invariants

11. Verifier conflict blocks completion claim.
12. Missing verifier evidence is `Unknown`, not `Safe`.
13. Human input is intent evidence, not state truth.
14. External tool success string is not verifier truth.
15. Evidence must bind to target identity.

---

## 4. Lease Invariants

16. Side-effecting action requires required leases.
17. GUI typing requires foreground lease.
18. VM destructive action requires exclusive VM lease.
19. Lease action hash must match current action hash.
20. Stale lease cannot authorize execution.

---

## 5. Workflow Invariants

21. Terminal workflows cannot resume.
22. Failed retry creates new attempt ID.
23. Timeout cannot silently execute risky action.
24. Replan invalidates old attempt decisions.
25. Cancel never means continue with default.

---

## 6. Security Invariants

26. External callback requires ownership token.
27. Tool descriptions are untrusted input.
28. Secrets must not appear in decision evidence.
29. Replay mode must be read-only.
30. Audit append failure blocks side-effecting execution.

---

## 7. Test Requirement

Every invariant above must have at least one automated test before MVP is considered production-ready.

