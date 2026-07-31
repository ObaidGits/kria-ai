# Negative golden input — checklist-only pass (role: traceability.md ledger)

Planted defect: `MGR-001` claims a `Pass` status backed only by a ticked task
checkbox, with no manifest path or content hash. Expected diagnostic kind
`status_without_manifest` for id `MGR-001`.

| ID+desc | Design | Work | Validation | Risk | Gate | Evidence | Status |
|---|---|---|---|---|---|---|---|
| MGR-001 Truth Contract | §2 | WE | V-AUTH-01 | R-TRUTH | F0 | task checkbox ticked | Pass |
